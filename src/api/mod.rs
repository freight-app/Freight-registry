use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, put},
    Json, Router,
};
use serde_json::json;

use crate::AppState;

pub mod download;
pub mod packages;
pub mod publish;
pub mod search;
pub mod yank;

pub type ApiResult<T> = Result<T, ApiError>;

pub struct ApiError(StatusCode, String);

impl ApiError {
    pub fn not_found(msg: impl Into<String>) -> Self { Self(StatusCode::NOT_FOUND, msg.into()) }
    pub fn bad_request(msg: impl Into<String>) -> Self { Self(StatusCode::BAD_REQUEST, msg.into()) }
    pub fn conflict(msg: impl Into<String>) -> Self { Self(StatusCode::CONFLICT, msg.into()) }
    pub fn internal(msg: impl Into<String>) -> Self { Self(StatusCode::INTERNAL_SERVER_ERROR, msg.into()) }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"errors": [{"detail": self.1}]}))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("{e:#}");
        Self::internal(e.to_string())
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/packages/:name", get(packages::get_package))
        .route("/api/v1/search", get(search::search_packages))
        .route("/api/v1/packages", put(publish::publish))
        .route("/api/v1/packages/:name/:version/download", get(download::download))
        .route(
            "/api/v1/packages/:name/:version/yank",
            delete(yank::yank).put(yank::unyank),
        )
        .route("/api/v1/me", get(me))
        .with_state(state)
}

async fn me(auth: crate::auth::AuthToken) -> Json<serde_json::Value> {
    Json(json!({ "login": auth.0.name, "id": auth.0.id }))
}
