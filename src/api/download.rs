use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{header, StatusCode},
    response::Response,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

/// Presigned URL TTL for S3 downloads.
const PRESIGN_TTL: Duration = Duration::from_secs(15 * 60); // 15 minutes

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
}

/// Download priority chain:
///
/// 1. `FREIGHT_DOWNLOAD_URL` configured → `302` to `{url}/{name}/{version}/source.tar.gz`
///    (CDN, public S3 bucket, nginx serving the tarballs directory, …)
///
/// 2. S3 backend, no explicit download URL → `302` to a presigned S3 URL (15 min TTL).
///    No bytes pass through this server.
///
/// 3. Local filesystem → read the file and stream the bytes directly.
pub async fn download(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version)): Path<(String, String)>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<Response> {
    if state.limiters.api.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);

    let ver = match state.db.get_version(&name, &version, channel).await? {
        Some(v) => v,
        None => {
            // Not found locally — try the upstream mirror.
            return proxy_from_mirror(&state, &name, &version, channel).await;
        }
    };

    if ver.yanked != 0 {
        return Err(ApiError::gone(format!("`{name}@{version}` has been yanked")));
    }

    // Metadata-only package: redirect to the upstream source archive.
    if let Some(ref upstream_url) = ver.upstream_url {
        state.db.increment_downloads(&name, &version, channel);
        return redirect(upstream_url);
    }

    // ── Priority 1: explicit download URL ────────────────────────────────────
    if let Some(ref base) = state.download_url {
        state.db.increment_downloads(&name, &version, channel);
        state.metrics.downloads_served.inc();
        let url = format!("{base}/{name}/{version}/source.tar.gz");
        return redirect(&url);
    }

    // ── Priority 2: S3 presigned URL ─────────────────────────────────────────
    if let Ok(Some(presigned)) = state
        .storage
        .presigned_get_url(&name, &version, PRESIGN_TTL)
        .await
    {
        state.db.increment_downloads(&name, &version, channel);
        state.metrics.downloads_served.inc();
        return redirect(presigned.as_str());
    }

    // ── Priority 3: stream from local storage ─────────────────────────────────
    let data = state
        .storage
        .read(&name, &version)
        .await
        .map_err(|_| ApiError::not_found(format!("`{name}@{version}` not found")))?;

    let actual = hex::encode(Sha256::digest(&data));
    if actual != ver.checksum {
        tracing::error!(
            name, version,
            expected = %ver.checksum, actual = %actual,
            "checksum mismatch on download",
        );
        return Err(ApiError::internal("stored checksum does not match file on disk"));
    }

    state.metrics.downloads_served.inc();
    state.db.increment_downloads(&name, &version, channel);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(header::CONTENT_DISPOSITION, "attachment; filename=\"source.tar.gz\"")
        .header("x-checksum-sha256", &ver.checksum)
        .body(Body::from(data))
        .map_err(|e| ApiError::internal(e.to_string()))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn redirect(url: &str) -> ApiResult<Response> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, url)
        .body(Body::empty())
        .map_err(|e| ApiError::internal(e.to_string()))
}

async fn proxy_from_mirror(
    state:   &AppState,
    name:    &str,
    version: &str,
    channel: &str,
) -> ApiResult<Response> {
    if let Some(ref upstream) = state.mirror_upstream {
        let url = if channel == DEFAULT_CHANNEL {
            format!("{upstream}/api/v1/packages/{name}/{version}/download")
        } else {
            format!("{upstream}/api/v1/packages/{name}/{version}/download?channel={channel}")
        };
        if let Ok(resp) = reqwest::get(&url).await {
            if resp.status() != reqwest::StatusCode::NOT_FOUND {
                let bytes = resp.bytes().await.unwrap_or_default();
                let filename = format!("{name}-{version}.tar.gz");
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/gzip")
                    .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\""))
                    .body(Body::from(bytes))
                    .map_err(|e| ApiError::internal(e.to_string()));
            }
        }
    }
    Err(ApiError::not_found(format!("`{name}@{version}` not found in channel `{channel}`")))
}
