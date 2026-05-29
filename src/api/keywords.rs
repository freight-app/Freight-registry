use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct KeywordsParams {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

pub async fn keywords(
    State(state): State<Arc<AppState>>,
    Query(params): Query<KeywordsParams>,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    let limit   = params.limit.unwrap_or(30).clamp(1, 100);
    let kws = state.db.keywords_top(channel, limit).await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({
        "keywords": kws.into_iter()
            .map(|(name, count)| json!({"name": name, "count": count}))
            .collect::<Vec<_>>()
    })))
}
