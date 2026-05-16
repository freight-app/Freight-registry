use std::sync::Arc;
use axum::{extract::{Path, State}, response::{IntoResponse, Response}};
use axum::http::{header, StatusCode};

use crate::AppState;

pub async fn get_readme(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    match state.storage.read_readme(&name).await {
        Some(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            content,
        ).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
