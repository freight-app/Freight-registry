use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;

use crate::{auth::PublishToken, db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
}

pub async fn yank(
    State(state): State<Arc<AppState>>,
    auth: PublishToken,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version)): Path<(String, String)>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    require_owner(&state, auth.user.id, &name, channel).await?;
    let updated = state.db.set_yanked(&name, &version, channel, true).await?;
    if !updated {
        return Err(ApiError::not_found(format!("`{name}@{version}` not found in channel `{channel}`")));
    }
    let ip = addr.ip().to_string();
    state.db.audit(Some(auth.user.id), "yank", Some(&name), Some(&version), Some(&ip));
    tracing::info!(user = %auth.user.username, channel, "yanked {name}@{version}");
    Ok(Json(json!({ "ok": true })))
}

pub async fn unyank(
    State(state): State<Arc<AppState>>,
    auth: PublishToken,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version)): Path<(String, String)>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    require_owner(&state, auth.user.id, &name, channel).await?;
    let updated = state.db.set_yanked(&name, &version, channel, false).await?;
    if !updated {
        return Err(ApiError::not_found(format!("`{name}@{version}` not found in channel `{channel}`")));
    }
    let ip = addr.ip().to_string();
    state.db.audit(Some(auth.user.id), "unyank", Some(&name), Some(&version), Some(&ip));
    tracing::info!(user = %auth.user.username, channel, "unyanked {name}@{version}");
    Ok(Json(json!({ "ok": true })))
}

async fn require_owner(state: &AppState, user_id: i64, package: &str, channel: &str) -> ApiResult<()> {
    match state.db.user_owns_package(user_id, package, channel).await? {
        Some(true) => Ok(()),
        Some(false) => Err(ApiError::forbidden(format!("you are not an owner of `{package}` in channel `{channel}`"))),
        None => Err(ApiError::not_found(format!("`{package}` not found in channel `{channel}`"))),
    }
}
