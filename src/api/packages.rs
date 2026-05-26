use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use reqwest;

use crate::{db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

/// Proxy a GET request to `url` and return parsed JSON, or `None` on 404.
async fn proxy_get_json(url: &str) -> Option<serde_json::Value> {
    let resp = reqwest::get(url).await.ok()?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND { return None; }
    resp.json().await.ok()
}

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
}

async fn get_package_local(
    state: &Arc<AppState>,
    name: &str,
    channel: &str,
) -> ApiResult<Json<Value>> {
    let (pkg, versions) = state
        .db
        .get_package(name, channel)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("`{name}` not found in channel `{channel}`")))?;

    let latest = versions
        .iter()
        .find(|v| v.yanked == 0)
        .or_else(|| versions.first())
        .map(|v| v.version.as_str())
        .unwrap_or("");

    let mut versions_json: Vec<Value> = Vec::new();
    for v in &versions {
        let url = download_url(&state.base_url, &pkg.name, &v.version, channel);
        let prebuilts = state.db.list_prebuilts(&pkg.name, channel, &v.version).await
            .unwrap_or_default();
        let prebuilt_triples: Vec<&str> = prebuilts.iter().map(|p| p.triple.as_str()).collect();
        let deps: serde_json::Value = serde_json::from_str(&v.dependencies).unwrap_or(json!({}));
        // For metadata-only packages, expose the upstream URL directly so clients
        // can fetch the source archive without routing through the registry server.
        let effective_download_url = v.upstream_url.clone().unwrap_or(url);
        versions_json.push(json!({
            "version":         v.version,
            "checksum":        v.checksum,
            "download_url":    effective_download_url,
            "upstream_url":    v.upstream_url,
            "build_system":    v.build_system,
            "yanked":          v.yanked != 0,
            "downloads":       v.downloads,
            "prebuilt_triples": prebuilt_triples,
            "dependencies":    deps,
        }));
    }

    Ok(Json(json!({
        "name":        pkg.name,
        "channel":     pkg.channel,
        "description": pkg.description,
        "latest":      latest,
        "versions":    versions_json,
    })))
}

pub async fn get_package(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(name): Path<String>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<Json<Value>> {
    if state.limiters.api.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);

    match get_package_local(&state, &name, channel).await {
        Ok(resp) => return Ok(resp),
        Err(ApiError(axum::http::StatusCode::NOT_FOUND, _)) => {}
        Err(e) => return Err(e),
    }

    // Not found locally — try the mirror upstream if configured.
    if let Some(ref upstream) = state.mirror_upstream {
        let url = if channel == DEFAULT_CHANNEL {
            format!("{upstream}/api/v1/packages/{name}")
        } else {
            format!("{upstream}/api/v1/packages/{name}?channel={channel}")
        };
        if let Some(body) = proxy_get_json(&url).await {
            return Ok(Json(body));
        }
    }

    Err(ApiError::not_found(format!("`{name}` not found in channel `{channel}`")))
}

pub fn download_url(base_url: &str, name: &str, version: &str, channel: &str) -> String {
    if channel == DEFAULT_CHANNEL {
        format!("{base_url}/api/v1/packages/{name}/{version}/download")
    } else {
        format!("{base_url}/api/v1/packages/{name}/{version}/download?channel={channel}")
    }
}
