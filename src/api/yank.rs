use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};

use crate::{auth::AuthToken, AppState};
use super::{ApiError, ApiResult};

pub async fn yank(
    State(state): State<Arc<AppState>>,
    _auth: AuthToken,
    Path((name, version)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let updated = state.db.set_yanked(&name, &version, true).await?;
    if !updated {
        return Err(ApiError::not_found(format!("`{name}@{version}` not found")));
    }
    tracing::info!("yanked {name}@{version}");
    Ok(Json(json!({ "ok": true })))
}

pub async fn unyank(
    State(state): State<Arc<AppState>>,
    _auth: AuthToken,
    Path((name, version)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let updated = state.db.set_yanked(&name, &version, false).await?;
    if !updated {
        return Err(ApiError::not_found(format!("`{name}@{version}` not found")));
    }
    tracing::info!("unyanked {name}@{version}");
    Ok(Json(json!({ "ok": true })))
}
