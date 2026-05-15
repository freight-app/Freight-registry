use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Path, State},
    Json,
};
use serde_json::{json, Value};
use std::net::SocketAddr;

use crate::{auth::PublishToken, AppState};
use super::{ApiError, ApiResult};

pub async fn yank(
    State(state): State<Arc<AppState>>,
    auth: PublishToken,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    require_owner(&state, auth.user.id, &name).await?;
    let updated = state.db.set_yanked(&name, &version, true).await?;
    if !updated {
        return Err(ApiError::not_found(format!("`{name}@{version}` not found")));
    }
    let ip = addr.ip().to_string();
    state.db.audit(Some(auth.user.id), "yank", Some(&name), Some(&version), Some(&ip));
    tracing::info!(user = %auth.user.username, "yanked {name}@{version}");
    Ok(Json(json!({ "ok": true })))
}

pub async fn unyank(
    State(state): State<Arc<AppState>>,
    auth: PublishToken,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    require_owner(&state, auth.user.id, &name).await?;
    let updated = state.db.set_yanked(&name, &version, false).await?;
    if !updated {
        return Err(ApiError::not_found(format!("`{name}@{version}` not found")));
    }
    let ip = addr.ip().to_string();
    state.db.audit(Some(auth.user.id), "unyank", Some(&name), Some(&version), Some(&ip));
    tracing::info!(user = %auth.user.username, "unyanked {name}@{version}");
    Ok(Json(json!({ "ok": true })))
}

async fn require_owner(state: &AppState, user_id: i64, package: &str) -> ApiResult<()> {
    match state.db.user_owns_package(user_id, package).await? {
        Some(true) => Ok(()),
        Some(false) => Err(ApiError::forbidden(format!("you are not an owner of `{package}`"))),
        None => Err(ApiError::not_found(format!("`{package}` not found"))),
    }
}
