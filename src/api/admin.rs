//! GET    /api/v1/admin/users                — list all users (admin only)
//! POST   /api/v1/admin/users/:name/promote  — grant admin role
//! POST   /api/v1/admin/users/:name/demote   — revoke admin role
//! DELETE /api/v1/admin/users/:name          — remove user and all their tokens

use std::sync::Arc;

use axum::{extract::{Path, State}, Json};
use serde_json::{json, Value};

use crate::{auth::AdminToken, AppState};
use super::{ApiError, ApiResult};

pub async fn list_users(
    _auth: AdminToken,
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<Value>> {
    let users = state.db.list_users().await?;
    let list: Vec<Value> = users
        .iter()
        .map(|u| json!({
            "id":       u.id,
            "username": u.username,
            "email":    u.email,
            "is_admin": u.is_admin != 0,
        }))
        .collect();
    Ok(Json(json!({ "users": list })))
}

pub async fn promote_user(
    _auth: AdminToken,
    State(state): State<Arc<AppState>>,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    if state.db.set_admin(&username, true).await? {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(ApiError::not_found(format!("user `{username}` not found")))
    }
}

pub async fn demote_user(
    _auth: AdminToken,
    State(state): State<Arc<AppState>>,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    if state.db.set_admin(&username, false).await? {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(ApiError::not_found(format!("user `{username}` not found")))
    }
}

pub async fn remove_user(
    _auth: AdminToken,
    State(state): State<Arc<AppState>>,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    if state.db.delete_user(&username).await? {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(ApiError::not_found(format!("user `{username}` not found")))
    }
}

/// GET /api/v1/admin/overview — registry-wide counts for the admin dashboard.
pub async fn overview(
    _auth: AdminToken,
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<Value>> {
    let o = state.db.admin_overview().await?;
    Ok(Json(json!({
        "packages":        o.stats.packages,
        "versions":        o.stats.versions,
        "users":           o.stats.users,
        "admins":          o.admins,
        "active_tokens":   o.stats.tokens_active,
        "downloads_total": o.stats.downloads_total,
        "open_reports":    o.open_reports,
    })))
}
