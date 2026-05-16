use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
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
    let (pkg, versions) = state
        .db
        .get_package(&name, channel)
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
        versions_json.push(json!({
            "version":         v.version,
            "checksum":        v.checksum,
            "download_url":    url,
            "yanked":          v.yanked != 0,
            "downloads":       v.downloads,
            "prebuilt_triples": prebuilt_triples,
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

pub fn download_url(base_url: &str, name: &str, version: &str, channel: &str) -> String {
    if channel == DEFAULT_CHANNEL {
        format!("{base_url}/api/v1/packages/{name}/{version}/download")
    } else {
        format!("{base_url}/api/v1/packages/{name}/{version}/download?channel={channel}")
    }
}
