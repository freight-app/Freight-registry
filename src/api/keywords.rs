use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

/// Curated browse terms shown when no package has keyword metadata.
/// Only those with at least one matching package are returned.
const FALLBACK_TERMS: &[&str] = &[
    "audio", "compression", "crypto", "database", "graphics", "gui",
    "http", "image", "json", "math", "mqtt", "networking", "opengl", "physics",
    "protobuf", "regex", "serialization", "sqlite", "tls", "unicode",
    "vulkan", "websocket", "xml", "zip", "zlib",
];

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

    let mut kws = state.db.keywords_top(channel, limit).await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // If no packages have keyword metadata, fall back to curated terms —
    // but only return those that actually match at least one package.
    if kws.is_empty() {
        kws = state.db.keywords_count_terms(channel, FALLBACK_TERMS).await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        kws.truncate(limit as usize);
    }

    Ok(Json(json!({
        "keywords": kws.into_iter()
            .map(|(name, count)| json!({"name": name, "count": count}))
            .collect::<Vec<_>>()
    })))
}
