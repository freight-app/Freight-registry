//! Password reset — two endpoints:
//!
//! POST /api/v1/users/reset-password/request
//!   Sends a reset link via the configured Mailer. Always returns 200 regardless
//!   of whether the username exists, to prevent user enumeration.
//!
//! POST /api/v1/users/reset-password/confirm
//!   Validates the reset token and sets a new password.

use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::hash_password, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct RequestBody {
    username: String,
}

#[derive(Deserialize)]
pub struct ConfirmBody {
    token:        String,
    /// SHA-256(plaintext) hex — same encoding as login/register.
    new_password: String,
}

pub async fn request_reset(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RequestBody>,
) -> ApiResult<Json<Value>> {
    // Intentionally always returns 200 — don't leak whether the username exists.
    if let Ok(Some(user)) = state.db.get_user_by_username(&req.username).await {
        let token = state.db.create_email_token(user.id, "reset").await?;
        let link = format!(
            "{}/api/v1/users/reset-password/confirm?token={token}",
            state.base_url,
        );
        // Only send if the account has an email address on file.
        if let Some(ref email) = user.email {
            state.mailer.send_password_reset(email, &user.username, &link).await;
        } else {
            tracing::warn!(
                user = %user.username,
                "password reset requested but no email on file — link: {link}",
            );
        }
    }
    Ok(Json(json!({
        "ok":      true,
        "message": "if that account exists and has an email address, a reset link has been sent",
    })))
}

pub async fn confirm_reset(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ConfirmBody>,
) -> ApiResult<Json<Value>> {
    let user_id = state
        .db
        .consume_email_token(&req.token, "reset")
        .await?
        .ok_or_else(|| ApiError::bad_request("invalid or expired reset token"))?;

    // Password arrives pre-hashed (SHA-256) from the client; wrap with Argon2id.
    let hash = hash_password(&req.new_password)
        .map_err(|_| ApiError::internal("password hashing failed"))?;
    state.db.set_password_hash(user_id, &hash).await?;

    tracing::info!(user_id, "password reset completed");
    Ok(Json(json!({ "ok": true, "message": "password updated" })))
}
