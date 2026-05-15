//! POST /api/v1/auth/refresh
//!
//! Exchange a valid refresh token for a new access token (90-day expiry).
//! Uses the `RefreshTokenAuth` extractor which only accepts kind="refresh" tokens.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{auth::RefreshTokenAuth, AppState};
use super::ApiResult;

pub async fn refresh(
    auth: RefreshTokenAuth,
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<Value>> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let name = format!("access-{ts}");
    let token = state
        .db
        .create_token(auth.user.id, &name, Some(90), "access")
        .await?;

    Ok(Json(json!({
        "token":       token,
        "expires_days": 90,
    })))
}
