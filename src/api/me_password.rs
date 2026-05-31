//! POST /api/v1/me/password — change the authenticated user's password
//!
//! Both `current_password` and `new_password` must be SHA-256(plaintext),
//! matching the convention used by the login and register endpoints.

use std::sync::Arc;

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::{hash_password, AuthToken}, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    current_password: String,
    new_password:     String,
}

pub async fn change_password(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<Json<Value>> {
    let user = state.db.get_user_by_id(auth.user.id).await?
        .ok_or_else(|| ApiError::internal("user not found"))?;

    if user.password_hash.starts_with("!oauth:") {
        return Err(ApiError::bad_request(
            "OAuth accounts do not have a password — sign in via your OAuth provider",
        ));
    }

    let parsed = PasswordHash::new(&user.password_hash)
        .map_err(|_| ApiError::internal("password hash corrupt"))?;
    if Argon2::default()
        .verify_password(req.current_password.as_bytes(), &parsed)
        .is_err()
    {
        return Err(ApiError::bad_request("current password is incorrect"));
    }

    let new_hash = hash_password(&req.new_password)
        .map_err(|_| ApiError::internal("password hashing failed"))?;

    state.db.set_password_hash(auth.user.id, &new_hash).await?;
    state.db.audit(Some(auth.user.id), "change_password", None, None, None);
    tracing::info!(user = %user.username, "password changed");

    Ok(Json(json!({ "ok": true })))
}
