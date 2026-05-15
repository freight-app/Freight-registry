//! TOTP / 2FA endpoints (all require a valid API token):
//!
//! POST   /api/v1/me/totp/enroll   — generate a new secret, store it, return QR URI
//! POST   /api/v1/me/totp/confirm  — verify the first code, activate TOTP
//! DELETE /api/v1/me/totp          — disable TOTP after verifying the current code

use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::PublishToken, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct CodeBody {
    code: String,
}

pub async fn enroll(
    auth: PublishToken,
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<Value>> {
    let secret = crate::totp::generate_secret_b32();
    let uri = crate::totp::provisioning_uri(&secret, &auth.user.username)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.db.set_totp_secret(auth.user.id, Some(&secret)).await?;

    Ok(Json(json!({
        "secret": secret,
        "uri":    uri,
    })))
}

pub async fn confirm(
    auth: PublishToken,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodeBody>,
) -> ApiResult<Json<Value>> {
    let user = state
        .db
        .get_user_by_id(auth.user.id)
        .await?
        .ok_or_else(|| ApiError::internal("user not found"))?;

    let secret = user
        .totp_secret
        .as_deref()
        .ok_or_else(|| ApiError::bad_request(
            "TOTP enrollment not started — call POST /api/v1/me/totp/enroll first",
        ))?;

    if !crate::totp::verify(secret, &user.username, &req.code) {
        return Err(ApiError::bad_request("invalid TOTP code"));
    }

    state.db.enable_totp(auth.user.id, true).await?;
    tracing::info!(user = %user.username, "TOTP enabled");

    Ok(Json(json!({ "ok": true })))
}

pub async fn disable(
    auth: PublishToken,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodeBody>,
) -> ApiResult<Json<Value>> {
    let user = state
        .db
        .get_user_by_id(auth.user.id)
        .await?
        .ok_or_else(|| ApiError::internal("user not found"))?;

    if user.totp_enabled == 0 {
        return Err(ApiError::bad_request("TOTP is not enabled"));
    }

    let secret = user
        .totp_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal("TOTP secret missing despite totp_enabled=1"))?;

    if !crate::totp::verify(secret, &user.username, &req.code) {
        return Err(ApiError::bad_request("invalid TOTP code"));
    }

    state.db.enable_totp(auth.user.id, false).await?;
    state.db.set_totp_secret(auth.user.id, None).await?;
    tracing::info!(user = %user.username, "TOTP disabled");

    Ok(Json(json!({ "ok": true })))
}
