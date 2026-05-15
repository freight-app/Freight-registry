use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};

use crate::AppState;
use super::{ApiError, ApiResult};

pub async fn get_package(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let (pkg, versions) = state
        .db
        .get_package(&name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("`{name}` not found")))?;

    // Latest = most recent non-yanked version; fall back to most recent overall.
    let latest = versions
        .iter()
        .find(|v| v.yanked == 0)
        .or_else(|| versions.first())
        .map(|v| v.version.as_str())
        .unwrap_or("");

    let versions_json: Vec<Value> = versions
        .iter()
        .map(|v| {
            let url = download_url(&state.base_url, &pkg.name, &v.version);
            json!({
                "version":      v.version,
                "checksum":     v.checksum,
                "download_url": url,
                "yanked":       v.yanked != 0,
            })
        })
        .collect();

    Ok(Json(json!({
        "name":        pkg.name,
        "description": pkg.description,
        "latest":      latest,
        "versions":    versions_json,
    })))
}

pub fn download_url(base_url: &str, name: &str, version: &str) -> String {
    format!("{base_url}/api/v1/packages/{name}/{version}/download")
}
