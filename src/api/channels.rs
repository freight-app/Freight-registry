use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::AppState;
use super::{ApiError, ApiResult};

pub async fn list_channels(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<Value>> {
    let channels = state.db.list_channels().await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "channels": channels })))
}
