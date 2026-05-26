use std::net::SocketAddr;
use std::sync::Arc;

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

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
}

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

    match state.db.get_version(&name, &version, channel).await? {
        Some(ver) => {
            if ver.yanked != 0 {
                return Err(ApiError::gone(format!("`{name}@{version}` has been yanked")));
            }

            // Metadata-only package: redirect to the upstream source archive.
            if let Some(ref upstream_url) = ver.upstream_url {
                state.db.increment_downloads(&name, &version, channel);
                return Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, upstream_url.as_str())
                    .body(Body::empty())
                    .map_err(|e| ApiError::internal(e.to_string()));
            }

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

            let filename = format!("{name}-{version}.tar.gz");
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/gzip")
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                )
                .header("x-checksum-sha256", &ver.checksum)
                .body(Body::from(data))
                .map_err(|e| ApiError::internal(e.to_string()))
        }
        None => {
            // Not found locally — proxy from upstream mirror if configured.
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
                            .header(
                                header::CONTENT_DISPOSITION,
                                format!("attachment; filename=\"{filename}\""),
                            )
                            .body(Body::from(bytes))
                            .map_err(|e| ApiError::internal(e.to_string()));
                    }
                }
            }
            Err(ApiError::not_found(format!("`{name}@{version}` not found in channel `{channel}`")))
        }
    }
}
