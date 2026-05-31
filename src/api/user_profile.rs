use std::sync::Arc;

use axum::{extract::{Path, State}, Json};
use serde_json::{json, Value};

use crate::AppState;
use super::{ApiError, ApiResult};

pub async fn get_user(
    State(state): State<Arc<AppState>>,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    let user = state.db.get_user_by_username(&username).await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;

    let packages = state.db.get_packages_by_owner(user.id).await?;
    let pkgs: Vec<Value> = packages.iter().map(|p| json!({
        "name":        p.name,
        "channel":     p.channel,
        "description": p.description,
        "license":     p.license,
        "version":     p.latest_version,
    })).collect();

    Ok(Json(json!({
        "username": user.username,
        "packages": pkgs,
    })))
}
