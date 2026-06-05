//! GET    /api/v1/me/tokens       — list tokens for the authenticated user
//! POST   /api/v1/me/tokens       — create a new token
//! DELETE /api/v1/me/tokens/:name — revoke a token by name

use std::sync::Arc;

use axum::{extract::{Path, State}, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::{AuthToken, PublishToken}, validate, AppState};
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
            "scope":      t.scope,
            "expires_at": t.expires_at,
            "last_used":  t.last_used,
        }))
        .collect();
    Ok(Json(json!({ "tokens": list })))
}

fn default_scope() -> String { "publish".to_string() }

#[derive(Deserialize)]
pub struct CreateTokenReq {
    name: String,
    #[serde(default)]
    expires_days: Option<i64>,
    /// Token scope: `"read"`, `"publish"` (default), or `"admin"`.
    #[serde(default = "default_scope")]
    scope: String,
    /// Restrict this token to publishing within a specific org.
    /// The authenticated user must be an owner of the named org.
    #[serde(default)]
    org: Option<String>,
}

pub async fn create(
    auth: PublishToken,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTokenReq>,
) -> ApiResult<Json<Value>> {
    validate::token_scope(&req.scope)?;

    // Resolve org name → id, verifying the caller is an org owner.
    let org_id = if let Some(ref org_name) = req.org {
        let org = state.db.get_org(org_name).await?
            .ok_or_else(|| ApiError::not_found(format!("org `{org_name}` not found")))?;
        let is_owner = auth.user.is_admin != 0
            || state.db.is_org_owner(org_name, auth.user.id).await?;
        if !is_owner {
            return Err(ApiError::forbidden("only org owners can create org-scoped tokens"));
        }
        Some(org.id)
    } else {
        None
    };

    let token = state
        .db
        .create_token(auth.user.id, &req.name, req.expires_days, "api", &req.scope, org_id)
        .await
        .map_err(|_| ApiError::conflict(format!("token `{}` already exists", req.name)))?;
    Ok(Json(json!({ "token": token, "name": req.name, "scope": req.scope, "org": req.org })))
}

pub async fn revoke(
    auth: PublishToken,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    if state.db.revoke_token(auth.user.id, &name).await? {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(ApiError::not_found(format!("token `{name}` not found")))
    }
}
