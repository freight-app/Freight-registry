//! POST /api/v1/users/login
//!
//! Verifies username + password (SHA-256 pre-hashed from client) with Argon2id,
//! optionally checks a TOTP code, then returns an access token + a refresh token.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
    username:    String,
    password:    String,
    #[serde(default)]
    token_name:  Option<String>,
    /// Access token lifetime in days (default 90, max 365).
    #[serde(default)]
    expires_days: Option<i64>,
    /// Required when the account has TOTP enabled.
    #[serde(default)]
    totp_code:   Option<String>,
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

    // Per-username lockout — checked before DB to skip the query on locked accounts.
    if state.limiters.login.is_locked(&req.username) {
        tracing::warn!(user = %req.username, "login blocked — account locked out");
        return Err(ApiError::too_many_requests());
    }

    let user = state
        .db
        .get_user_by_username(&req.username)
        .await?
        .ok_or_else(|| ApiError::not_found("unknown username or wrong password"))?;

    // OAuth-only accounts have a sentinel instead of a real password hash.
    if user.password_hash.starts_with("!oauth:") {
        let provider = user.password_hash.trim_start_matches("!oauth:");
        return Err(ApiError::bad_request(format!(
            "this account uses {provider} login — visit /auth/{provider} to sign in"
        )));
    }

    let parsed = PasswordHash::new(&user.password_hash)
        .map_err(|_| ApiError::internal("password hash corrupt"))?;

    if Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed)
        .is_err()
    {
        state.metrics.logins_fail.inc();
        let locked = state.limiters.login.record_failure(&req.username);
        if locked {
            tracing::warn!(user = %req.username, "account locked after repeated failures");
        }
        return Err(ApiError::not_found("unknown username or wrong password"));
    }

    // TOTP check — required when the account has 2FA enabled.
    // The client may supply either a live TOTP code or a single-use recovery code.
    if user.totp_enabled != 0 {
        let code = req
            .totp_code
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("TOTP code required"))?;
        let secret = user
            .totp_secret
            .as_deref()
            .ok_or_else(|| ApiError::internal("TOTP secret missing"))?;
        let valid_totp = crate::totp::verify(secret, &user.username, code);
        let valid_recovery = if valid_totp {
            false
        } else {
            state.db.consume_recovery_code(user.id, code).await?
        };
        if !valid_totp && !valid_recovery {
            return Err(ApiError::bad_request("invalid TOTP code"));
        }
    }

    state.metrics.logins_ok.inc();
    state.limiters.login.record_success(&req.username);

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let token_name = req
        .token_name
        .unwrap_or_else(|| format!("login-{ts}"));
    let refresh_name = format!("refresh-{ts}");

    let expires_days = req.expires_days.map(|d| d.clamp(1, 365)).or(Some(90));

    // Access token: client-configurable lifetime, kind="access", scope="publish".
    let token = state
        .db
        .create_token(user.id, &token_name, expires_days, "access", "publish", None)
        .await?;
    // Refresh token: 30-day fixed lifetime. Scope is inherited by the new access token on refresh.
    let refresh_token = state
        .db
        .create_token(user.id, &refresh_name, Some(30), "refresh", "publish", None)
        .await?;

    let ip = addr.ip().to_string();
    state.db.audit(Some(user.id), "login", None, None, Some(&ip));
    tracing::info!(user = %user.username, "logged in from {ip}");

    Ok(Json(json!({
        "token":         token,
        "refresh_token": refresh_token,
        "expires_days":  expires_days,
    })))
}
