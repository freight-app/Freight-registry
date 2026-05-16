use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::PublishToken, db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct OwnersBody {
    users: Vec<String>,
}

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    let owners = state.db.get_package_owners(&name, channel).await?;
    if owners.is_empty() {
        if state.db.get_package(&name, channel).await?.is_none() {
            return Err(ApiError::not_found(format!("`{name}` not found in channel `{channel}`")));
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
    auth: PublishToken,
    Path(name): Path<String>,
    Query(params): Query<ChannelParam>,
    Json(body): Json<OwnersBody>,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    require_owner(&state, auth.user.id, &name, channel).await?;
    let mut added = Vec::new();
    let mut not_found = Vec::new();
    for username in &body.users {
        if state.db.add_package_owner(&name, channel, username).await? {
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
    auth: PublishToken,
    Path(name): Path<String>,
    Query(params): Query<ChannelParam>,
    Json(body): Json<OwnersBody>,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    require_owner(&state, auth.user.id, &name, channel).await?;
    let current_owners = state.db.get_package_owners(&name, channel).await?;
    let removing_self = body.users.iter().any(|u| u.eq_ignore_ascii_case(&auth.user.username));
    if removing_self && current_owners.len() == 1 {
        return Err(ApiError::bad_request(
            "cannot remove the last owner — add another owner first",
        ));
    }
    let mut removed = 0usize;
    for username in &body.users {
        if state.db.remove_package_owner(&name, channel, username).await? {
            removed += 1;
        }
    }
    Ok(Json(json!({ "ok": true, "msg": format!("removed {removed} owner(s)") })))
}

async fn require_owner(state: &AppState, user_id: i64, package: &str, channel: &str) -> ApiResult<()> {
    match state.db.user_owns_package(user_id, package, channel).await? {
        Some(true) => Ok(()),
        Some(false) => Err(ApiError::forbidden(format!("you are not an owner of `{package}` in channel `{channel}`"))),
        None => Err(ApiError::not_found(format!("`{package}` not found in channel `{channel}`"))),
    }
}
