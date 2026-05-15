use std::sync::Arc;

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;
use super::{ApiResult, packages::download_url};

#[derive(Deserialize)]
pub struct SearchParams {
    q: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 { 20 }

pub async fn search_packages(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Value>> {
    let query = params.q.as_deref().unwrap_or("");
    let results = state.db.search_packages(query, params.limit.clamp(1, 100)).await?;

    let packages: Vec<Value> = results
        .into_iter()
        .filter_map(|(pkg, latest)| {
            let latest = latest?;
            let url = download_url(&state.base_url, &pkg.name, &latest.version);
            Some(json!({
                "name":        pkg.name,
                "description": pkg.description,
                "latest":      latest.version,
                "versions": [{
                    "version":      latest.version,
                    "checksum":     latest.checksum,
                    "download_url": url,
                }],
            }))
        })
        .collect();

    Ok(Json(json!({ "packages": packages })))
}
