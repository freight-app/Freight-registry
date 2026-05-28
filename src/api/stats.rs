//! GET /api/v1/stats — registry-wide aggregate statistics.

use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::json;

use crate::AppState;
use super::ApiResult;

pub async fn stats(State(state): State<Arc<AppState>>) -> ApiResult<Json<serde_json::Value>> {
    let s = state.db.stats().await?;
    Ok(Json(json!({
        "packages":  s.packages,
        "downloads": s.downloads_total,
        "versions":  s.versions,
        "users":     s.users,
    })))
}
