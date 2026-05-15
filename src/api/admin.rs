//! GET /api/v1/admin/users  — list all users (admin only)

use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{auth::AdminToken, AppState};
use super::ApiResult;

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
