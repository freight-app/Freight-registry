use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;
use super::{ApiError, ApiResult, packages::download_url};

#[derive(Deserialize)]
pub struct SearchParams {
    q: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 { 20 }

pub async fn search_packages(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Value>> {
    if state.limiters.api.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let query  = params.q.as_deref().unwrap_or("");
    let limit  = params.limit.clamp(1, 100);
    let offset = params.offset.max(0);

    let (results, total) = state.db.search_packages(query, limit, offset).await?;

    let packages: Vec<Value> = results
        .into_iter()
        .filter_map(|(pkg, latest)| {
            let latest = latest?;
            let url = download_url(&state.base_url, &pkg.name, &latest.version);
            Some(json!({
                "name":        pkg.name,
                "description": pkg.description,
                "latest":      latest.version,
                "downloads":   latest.downloads,
                "versions": [{
                    "version":      latest.version,
                    "checksum":     latest.checksum,
                    "download_url": url,
                }],
            }))
        })
        .collect();

    Ok(Json(json!({
        "packages": packages,
        "total":    total,
        "limit":    limit,
        "offset":   offset,
    })))
}
