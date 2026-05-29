//! Prebuilt binary tarball endpoints.
//!
//! PUT    /api/v1/packages/:name/:version/prebuilt/:triple  — upload
//! GET    /api/v1/packages/:name/:version/prebuilt/:triple/download — download
//! GET    /api/v1/packages/:name/:version/prebuilts         — list triples

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{ConnectInfo, Path, Query, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Presigned URL TTL for S3 prebuilt downloads.
const PRESIGN_TTL: Duration = Duration::from_secs(15 * 60);

use crate::{auth::PublishToken, db::DEFAULT_CHANNEL, validate, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
}

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    arch: Option<String>,
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    backend: Option<String>,
}

/// PUT /api/v1/packages/:name/:version/prebuilt/:triple
///
/// Body: raw `.tar.gz` bytes (the prebuilt tarball).
/// Requires a publish token and ownership of the package.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    auth: PublishToken,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version, triple)): Path<(String, String, String)>,
    Query(params): Query<ChannelParam>,
    body: axum::body::Bytes,
) -> ApiResult<Json<Value>> {
    if state.limiters.write.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    validate::channel_name(channel)?;
    validate::package_name(&name)?;
    validate::version(&version)?;
    validate_triple(&triple)?;

    // Package must already exist (source was published first).
    let _ = state.db.get_version(&name, &version, channel).await?
        .ok_or_else(|| ApiError::not_found(
            format!("`{name}@{version}` not found in channel `{channel}` — publish source first")
        ))?;

    // Must be an owner.
    match state.db.user_owns_package(auth.user.id, &name, channel).await? {
        Some(false) | None => {
            return Err(ApiError::forbidden(format!(
                "you are not an owner of `{name}` in channel `{channel}`"
            )));
        }
        _ => {}
    }

    if body.len() < 2 || body[0] != 0x1f || body[1] != 0x8b {
        return Err(ApiError::bad_request("body is not a valid gzip archive"));
    }

    let checksum = hex::encode(Sha256::digest(&body));

    state.storage.save_prebuilt(&name, &version, &triple, &body).await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.db.store_prebuilt(&name, channel, &version, &triple, &checksum).await?;

    let ip = addr.ip().to_string();
    state.db.audit(Some(auth.user.id), "publish_prebuilt", Some(&name), Some(&version), Some(&ip));
    tracing::info!(
        user = %auth.user.username, channel, triple,
        "uploaded prebuilt for {}@{}", name, version
    );

    Ok(Json(json!({ "ok": true, "triple": triple, "checksum": checksum })))
}

/// GET /api/v1/packages/:name/:version/prebuilt/:triple/download
pub async fn download(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version, triple)): Path<(String, String, String)>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<Response> {
    if state.limiters.api.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);

    let row = state.db.get_prebuilt(&name, channel, &version, &triple).await?
        .ok_or_else(|| ApiError::not_found(
            format!("no prebuilt for `{name}@{version}` triple `{triple}` in channel `{channel}`")
        ))?;

    // ── Priority 1: explicit download URL ────────────────────────────────────
    if let Some(ref base) = state.download_url {
        let url = format!("{base}/{name}/{version}/{triple}/{name}-{version}-{triple}.tar.gz");
        return Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, url)
            .body(Body::empty())
            .map_err(|e| ApiError::internal(e.to_string()));
    }

    // ── Priority 2: S3 presigned URL ─────────────────────────────────────────
    if let Ok(Some(presigned)) = state
        .storage
        .presigned_get_prebuilt_url(&name, &version, &triple, PRESIGN_TTL)
        .await
    {
        return Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, presigned.as_str())
            .body(Body::empty())
            .map_err(|e| ApiError::internal(e.to_string()));
    }

    // ── Priority 3: stream from local storage ─────────────────────────────────
    let data = state.storage.read_prebuilt(&name, &version, &triple).await
        .map_err(|_| ApiError::not_found(
            format!("prebuilt file for `{name}@{version}` triple `{triple}` not found on disk")
        ))?;

    let actual = hex::encode(Sha256::digest(&data));
    if actual != row.checksum {
        tracing::error!(
            name, version, triple,
            expected = %row.checksum, actual = %actual,
            "prebuilt checksum mismatch on download",
        );
        return Err(ApiError::internal("stored checksum does not match file on disk"));
    }

    let filename = format!("{name}-{version}-{triple}.tar.gz");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\""))
        .header("x-checksum-sha256", &row.checksum)
        .body(Body::from(data))
        .map_err(|e| ApiError::internal(e.to_string()))
}

/// GET /api/v1/packages/:name/:version/prebuilts
///
/// Optional filters: `?arch=x86_64`, `?os=linux`, `?backend=gnu`
/// Filters match the corresponding segment of the `arch-os-backend` triple.
pub async fn list(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((name, version)): Path<(String, String)>,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Value>> {
    if state.limiters.api.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);

    let rows = state.db.list_prebuilts(&name, channel, &version).await?;
    let triples: Vec<Value> = rows.iter()
        .filter(|r| triple_matches(&r.triple, &params))
        .map(|r| json!({
            "triple":    r.triple,
            "checksum":  r.checksum,
        }))
        .collect();

    Ok(Json(json!({ "name": name, "version": version, "channel": channel, "prebuilts": triples })))
}

/// Returns true if `triple` matches all provided filter components.
/// Triple format is `arch-os[-backend]` (e.g. `x86_64-linux-gnu`).
fn triple_matches(triple: &str, params: &ListParams) -> bool {
    let parts: Vec<&str> = triple.splitn(3, '-').collect();
    let arch    = parts.first().copied().unwrap_or("");
    let os      = parts.get(1).copied().unwrap_or("");
    let backend = parts.get(2).copied().unwrap_or("");

    params.arch.as_deref().map_or(true, |f| arch == f)
        && params.os.as_deref().map_or(true, |f| os == f)
        && params.backend.as_deref().map_or(true, |f| backend == f)
}

/// Triple must be `arch-os[-abi]` format with only safe characters.
fn validate_triple(triple: &str) -> ApiResult<()> {
    if triple.is_empty() || triple.len() > 64 {
        return Err(ApiError::bad_request("triple must be 1–64 characters"));
    }
    if !triple.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(ApiError::bad_request(
            "triple may only contain ASCII letters, digits, hyphens, and underscores",
        ));
    }
    Ok(())
}
