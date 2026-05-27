//! POST /api/v1/users/register
//!
//! Open registration: any client can create a new account and receive an
//! initial API token in one round-trip.  Rate-limited by the write limiter.
//!
//! If an email address is provided a verification link is delivered via the
//! configured [`Mailer`] (real SMTP when `--smtp-host` is set, stdout otherwise).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::hash_password, validate, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct RegisterRequest {
    username: String,
    password: String,
    #[serde(default)]
    email: Option<String>,
    /// Name for the initial token (defaults to `init`).
    #[serde(default)]
    token_name: Option<String>,
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<Json<Value>> {
    if state.limiters.write.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    validate::username(&req.username)
        .map_err(|e| ApiError::bad_request(e))?;
    // Password arrives as SHA-256(plaintext) from the client; length/complexity
    // is validated client-side before hashing.  We just wrap it with Argon2id.

    let hash = hash_password(&req.password)
        .map_err(|_| ApiError::internal("password hashing failed"))?;

    let user_id = state
        .db
        .create_user(&req.username, req.email.as_deref(), &hash)
        .await
        .map_err(|_| ApiError::conflict(
            format!("username `{}` is already taken", req.username)
        ))?;

    let token_name = req.token_name.unwrap_or_else(|| "init".to_string());
    let token = state
        .db
        .create_token(user_id, &token_name, Some(90), "api", "publish")
        .await?;

    // Send a verification link when an email was provided.
    if let Some(ref email) = req.email {
        let verify_token = state.db.create_email_token(user_id, "verify").await?;
        let link = format!(
            "{}/api/v1/users/verify-email?token={verify_token}",
            state.base_url,
        );
        state.mailer.send_verification(email, &req.username, &link).await;
    }

    let ip = addr.ip().to_string();
    state.db.audit(Some(user_id), "register", None, None, Some(&ip));
    tracing::info!(user = %req.username, "registered from {ip}");

    Ok(Json(json!({
        "ok":          true,
        "login":       req.username,
        "id":          user_id,
        "token":       token,
        "expires_days": 90,
    })))
}
