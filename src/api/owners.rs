use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::AuthToken, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct OwnersBody {
    users: Vec<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let owners = state.db.get_package_owners(&name).await?;
    if owners.is_empty() {
        // Could mean the package doesn't exist — surface a helpful error.
        if state.db.get_package(&name).await?.is_none() {
            return Err(ApiError::not_found(format!("`{name}` not found")));
        }
    }
    let users: Vec<Value> = owners
        .iter()
        .map(|u| json!({ "login": u.username, "id": u.id }))
        .collect();
    Ok(Json(json!({ "users": users })))
}

pub async fn add(
    State(state): State<Arc<AppState>>,
    auth: AuthToken,
    Path(name): Path<String>,
    Json(body): Json<OwnersBody>,
) -> ApiResult<Json<Value>> {
    require_owner(&state, auth.user.id, &name).await?;
    let mut added = Vec::new();
    let mut not_found = Vec::new();
    for username in &body.users {
        if state.db.add_package_owner(&name, username).await? {
            added.push(username.as_str());
        } else {
            not_found.push(username.as_str());
        }
    }
    let msg = if not_found.is_empty() {
        format!("added {} owner(s)", added.len())
    } else {
        format!(
            "added {}; not found: {}",
            added.join(", "),
            not_found.join(", ")
        )
    };
    Ok(Json(json!({ "ok": true, "msg": msg })))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    auth: AuthToken,
    Path(name): Path<String>,
    Json(body): Json<OwnersBody>,
) -> ApiResult<Json<Value>> {
    require_owner(&state, auth.user.id, &name).await?;
    // Guard: cannot remove yourself if you are the only owner.
    let current_owners = state.db.get_package_owners(&name).await?;
    let removing_self = body.users.iter().any(|u| u.eq_ignore_ascii_case(&auth.user.username));
    if removing_self && current_owners.len() == 1 {
        return Err(ApiError::bad_request(
            "cannot remove the last owner — add another owner first",
        ));
    }
    let mut removed = 0usize;
    for username in &body.users {
        if state.db.remove_package_owner(&name, username).await? {
            removed += 1;
        }
    }
    Ok(Json(json!({ "ok": true, "msg": format!("removed {removed} owner(s)") })))
}

async fn require_owner(state: &AppState, user_id: i64, package: &str) -> ApiResult<()> {
    match state.db.user_owns_package(user_id, package).await? {
        Some(true) => Ok(()),
        Some(false) => Err(ApiError::forbidden(format!("you are not an owner of `{package}`"))),
        None => Err(ApiError::not_found(format!("`{package}` not found"))),
    }
}
