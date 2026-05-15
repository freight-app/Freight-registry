//! POST /api/v1/users/login
//!
//! Verifies username + password with Argon2, creates a new API token,
//! and returns it.  Tokens created via login expire after 90 days by default.

use std::net::SocketAddr;
use std::sync::Arc;

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    extract::{ConnectInfo, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
    /// Optional name for the resulting token (defaults to `login-<unix>`).
    #[serde(default)]
    token_name: Option<String>,
    /// Token lifetime in days (default 90, max 365).
    #[serde(default)]
    expires_days: Option<i64>,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Json<Value>> {
    // Strict rate limit on login to slow down brute-force attempts.
    if state.limiters.write.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    // Per-username lockout — checked before DB to avoid the query on already-locked accounts.
    if state.limiters.login.is_locked(&req.username) {
        tracing::warn!(user = %req.username, "login blocked — account locked out");
        return Err(ApiError::too_many_requests());
    }

    let user = state
        .db
        .get_user_by_username(&req.username)
        .await?
        .ok_or_else(|| ApiError::not_found("unknown username or wrong password"))?;

    let parsed = PasswordHash::new(&user.password_hash)
        .map_err(|_| ApiError::internal("password hash corrupt"))?;

    if Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed)
        .is_err()
    {
        let locked = state.limiters.login.record_failure(&req.username);
        if locked {
            tracing::warn!(user = %req.username, "account locked after repeated failures");
        }
        return Err(ApiError::not_found("unknown username or wrong password"));
    }

    state.limiters.login.record_success(&req.username);

    let token_name = req.token_name.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("login-{ts}")
    });

    let expires_days = req.expires_days.map(|d| d.clamp(1, 365)).or(Some(90));
    let token = state.db.create_token(user.id, &token_name, expires_days).await?;

    let ip = addr.ip().to_string();
    state
        .db
        .audit(Some(user.id), "login", None, None, Some(&ip));
    tracing::info!(user = %user.username, "logged in from {ip}");

    Ok(Json(json!({
        "token": token,
        "expires_days": expires_days,
    })))
}
