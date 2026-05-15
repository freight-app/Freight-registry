use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
};

use crate::AppState;
use super::{ApiError, ApiResult};

pub async fn download(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
) -> ApiResult<Response> {
    if !state.storage.exists(&name, &version) {
        return Err(ApiError::not_found(format!("`{name}@{version}` not found")));
    }

    let data = state
        .storage
        .read(&name, &version)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let filename = format!("{name}-{version}.tar.gz");
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from(data))
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(resp)
}
