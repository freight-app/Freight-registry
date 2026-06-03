//! GET /api/v1/packages/:name/:version/status
//!
//! Returns the verification status of a package version so publishers can
//! poll while the CI pipeline runs.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ApiError, ApiResult};
use crate::db::DEFAULT_CHANNEL;
use crate::AppState;

#[derive(Deserialize)]
pub struct Params {
    channel: Option<String>,
}

pub async fn get_status(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
    Query(params): Query<Params>,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);

    match state.db.get_version_status(&name, &version, channel).await
        .map_err(|e| ApiError::internal(e.to_string()))?
    {
        None => Err(ApiError::not_found(
            format!("`{name}@{version}` not found in channel `{channel}`")
        )),
        Some((status, reason)) => Ok(Json(json!({
            "name":    name,
            "version": version,
            "channel": channel,
            "status":  status,
            "reason":  reason,
        }))),
    }
}
