use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult, packages::download_url};

#[derive(Deserialize)]
pub struct SearchParams {
    q: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
    /// Single channel (legacy). Ignored when `channels` is present.
    #[serde(default)]
    channel: Option<String>,
    /// Comma-separated list of channels to search across, e.g. `stable,experimental`.
    #[serde(default)]
    channels: Option<String>,
    /// Sort order: "name" (default), "downloads", "newest"
    #[serde(default)]
    sort: Option<String>,
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
    let sort   = params.sort.as_deref().unwrap_or("name");

    // Resolve channel list: `channels` (multi) takes precedence over `channel` (single).
    let channels_owned: Vec<String> = if let Some(ref cs) = params.channels {
        cs.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    } else {
        vec![params.channel.clone().unwrap_or_else(|| DEFAULT_CHANNEL.to_string())]
    };
    let channels: Vec<&str> = channels_owned.iter().map(String::as_str).collect();
    // Use the first channel as the representative for per-result URLs / mirror queries.
    let primary_channel = channels.first().copied().unwrap_or(DEFAULT_CHANNEL);

    let (results, total) = state.db.search_packages(query, &channels, limit, offset, sort).await?;

    let mut local_names = std::collections::HashSet::new();
    let mut packages: Vec<Value> = results
        .into_iter()
        .filter_map(|(pkg, latest)| {
            let latest = latest?;
            local_names.insert(pkg.name.to_lowercase());
            let url = download_url(&state.base_url, &pkg.name, &latest.version, &pkg.channel);
            let keywords: Vec<&str> = pkg.keywords.as_deref()
                .map(|s| s.split(',').map(str::trim).filter(|k| !k.is_empty()).collect())
                .unwrap_or_default();
            Some(json!({
                "name":         pkg.name,
                "channel":      pkg.channel,
                "description":  pkg.description,
                "latest":       latest.version,
                "downloads":    latest.downloads,
                "keywords":     keywords,
                "build_system": latest.build_system,
                "versions": [{
                    "version":       latest.version,
                    "checksum":      latest.checksum,
                    "download_url":  url,
                    "build_system":  latest.build_system,
                }],
            }))
        })
        .collect();

    // Merge upstream results for packages not in the local registry.
    if let Some(ref upstream) = state.mirror_upstream {
        let url = if primary_channel == DEFAULT_CHANNEL {
            format!("{upstream}/api/v1/search?q={query}&limit={limit}&offset={offset}")
        } else {
            format!("{upstream}/api/v1/search?q={query}&limit={limit}&offset={offset}&channel={primary_channel}")
        };
        if let Ok(resp) = reqwest::get(&url).await {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(upstream_pkgs) = body.get("packages").and_then(|v| v.as_array()) {
                    for pkg in upstream_pkgs {
                        let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        if !local_names.contains(&name.to_lowercase()) {
                            packages.push(pkg.clone());
                        }
                    }
                }
            }
        }
    }

    Ok(Json(json!({
        "packages": packages,
        "total":    total,
        "limit":    limit,
        "offset":   offset,
    })))
}
