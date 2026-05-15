//! GET    /api/v1/me/tokens       — list tokens for the authenticated user
//! POST   /api/v1/me/tokens       — create a new token
//! DELETE /api/v1/me/tokens/:name — revoke a token by name

use std::sync::Arc;

use axum::{extract::{Path, State}, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::AuthToken, AppState};
use super::{ApiError, ApiResult};

pub async fn list(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<Value>> {
    let tokens = state.db.list_tokens(Some(auth.user.id)).await?;
    let list: Vec<Value> = tokens
        .iter()
        .map(|t| json!({
            "id":         t.id,
            "name":       t.name,
            "kind":       t.kind,
            "expires_at": t.expires_at,
            "last_used":  t.last_used,
        }))
        .collect();
    Ok(Json(json!({ "tokens": list })))
}

#[derive(Deserialize)]
pub struct CreateTokenReq {
    name: String,
    #[serde(default)]
    expires_days: Option<i64>,
}

pub async fn create(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTokenReq>,
) -> ApiResult<Json<Value>> {
    let token = state
        .db
        .create_token(auth.user.id, &req.name, req.expires_days, "api")
        .await
        .map_err(|_| ApiError::conflict(format!("token `{}` already exists", req.name)))?;
    Ok(Json(json!({ "token": token, "name": req.name })))
}

pub async fn revoke(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    if state.db.revoke_token(auth.user.id, &name).await? {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(ApiError::not_found(format!("token `{name}` not found")))
    }
}
