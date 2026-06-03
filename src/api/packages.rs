use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use reqwest;

use crate::{db::{best_version, DEFAULT_CHANNEL}, AppState};
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
) -> ApiResult<Value> {
    let (pkg, versions) = state
        .db
        .get_package(name, channel)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("`{name}` not found in channel `{channel}`")))?;

    let latest = best_version(&versions).unwrap_or("");

    let mut versions_json: Vec<Value> = Vec::new();
    for v in &versions {
        let url = download_url(&state.base_url, &pkg.name, &v.version, channel);
        let prebuilts = state.db.list_prebuilts(&pkg.name, channel, &v.version).await
            .unwrap_or_default();
        let prebuilt_triples: Vec<&str> = prebuilts.iter().map(|p| p.triple.as_str()).collect();
        let deps: serde_json::Value = serde_json::from_str(&v.dependencies).unwrap_or(json!({}));
        let effective_download_url = v.upstream_url.clone().unwrap_or(url);
        let languages: Vec<&str> = v.languages.as_deref()
            .map(|s| s.split(',').filter(|l| !l.is_empty()).collect())
            .unwrap_or_default();
        versions_json.push(json!({
            "version":          v.version,
            "checksum":         v.checksum,
            "download_url":     effective_download_url,
            "upstream_url":     v.upstream_url,
            "build_system":     v.build_system,
            "supports":         v.supports,
            "yanked":           v.yanked != 0,
            "downloads":        v.downloads,
            "prebuilt_triples": prebuilt_triples,
            "dependencies":     deps,
            "languages":        languages,
        }));
    }

    let keywords: Vec<&str> = pkg.keywords.as_deref()
        .map(|s| s.split(',').map(str::trim).filter(|k| !k.is_empty()).collect())
        .unwrap_or_default();

    Ok(json!({
        "name":        pkg.name,
        "channel":     pkg.channel,
        "description": pkg.description,
        "license":     pkg.license,
        "keywords":    keywords,
        "latest":      latest,
        "versions":    versions_json,
    }))
}

/// Compute a quoted ETag from arbitrary bytes: `"<sha256-hex>"`.
fn make_etag(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("\"{}\"", hex::encode(Sha256::digest(data)))
}

pub async fn get_package(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req_headers: HeaderMap,
    Path(name): Path<String>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<Response> {
    if state.limiters.api.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);

    let value = match get_package_local(&state, &name, channel).await {
        Ok(v) => v,
        Err(ApiError(StatusCode::NOT_FOUND, _)) => {
            // Not found locally — try the mirror upstream if configured.
            if let Some(ref upstream) = state.mirror_upstream {
                let url = if channel == DEFAULT_CHANNEL {
                    format!("{upstream}/api/v1/packages/{name}")
                } else {
                    format!("{upstream}/api/v1/packages/{name}?channel={channel}")
                };
                if let Some(body) = proxy_get_json(&url).await {
                    // Mirror responses are passed through without ETag (we don't own them).
                    return Ok(Json(body).into_response());
                }
            }
            return Err(ApiError::not_found(format!("`{name}` not found in channel `{channel}`")));
        }
        Err(e) => return Err(e),
    };

    let body = serde_json::to_string(&value)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let etag = make_etag(body.as_bytes());

    // Return 304 Not Modified if the client already has this version.
    if let Some(inm) = req_headers.get(header::IF_NONE_MATCH) {
        if inm.as_bytes() == etag.as_bytes() {
            let mut resp = StatusCode::NOT_MODIFIED.into_response();
            if let Ok(v) = HeaderValue::from_str(&etag) {
                resp.headers_mut().insert(header::ETAG, v);
            }
            return Ok(resp);
        }
    }

    let mut resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))],
        body,
    ).into_response();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        resp.headers_mut().insert(header::ETAG, v);
    }
    Ok(resp)
}

/// GET /api/v1/graph?channel=
///
/// Returns every package in the registry as a flat list of `{ name, deps }`
/// objects — one entry per package, `deps` being its direct dependency names
/// (keys from the latest-version dependencies JSON).  Used by the global
/// dependency-graph page.
pub async fn get_graph(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<impl IntoResponse> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    let rows = state.db.all_packages_with_deps(channel).await?;

    let graph: Vec<serde_json::Value> = rows.iter().map(|(name, deps_json)| {
        let dep_names: Vec<String> = match serde_json::from_str::<serde_json::Value>(deps_json) {
            Ok(serde_json::Value::Object(map)) => map.keys().cloned().collect(),
            _ => vec![],
        };
        serde_json::json!({ "name": name, "deps": dep_names })
    }).collect();

    Ok(Json(serde_json::json!(graph)))
}

pub fn download_url(base_url: &str, name: &str, version: &str, channel: &str) -> String {
    if channel == DEFAULT_CHANNEL {
        format!("{base_url}/api/v1/packages/{name}/{version}/download")
    } else {
        format!("{base_url}/api/v1/packages/{name}/{version}/download?channel={channel}")
    }
}
