use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{ConnectInfo, Path, State},
    http::{header, StatusCode},
    response::Response,
};
use sha2::{Digest, Sha256};

use crate::AppState;
use super::{ApiError, ApiResult};

pub async fn download(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version)): Path<(String, String)>,
) -> ApiResult<Response> {
    if state.limiters.api.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let ver = state
        .db
        .get_version(&name, &version)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("`{name}@{version}` not found")))?;

    if ver.yanked != 0 {
        return Err(ApiError::gone(format!("`{name}@{version}` has been yanked")));
    }

    let data = state
        .storage
        .read(&name, &version)
        .await
        .map_err(|_| ApiError::not_found(format!("`{name}@{version}` not found")))?;

    // Verify integrity against the stored checksum.
    let actual = hex::encode(Sha256::digest(&data));
    if actual != ver.checksum {
        tracing::error!(
            name, version,
            expected = %ver.checksum, actual = %actual,
            "checksum mismatch on download",
        );
        return Err(ApiError::internal("stored checksum does not match file on disk"));
    }

    state.db.increment_downloads(&name, &version);

    let filename = format!("{name}-{version}.tar.gz");
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .header("x-checksum-sha256", &ver.checksum)
        .body(Body::from(data))
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(resp)
}
