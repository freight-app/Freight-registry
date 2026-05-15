use std::sync::Arc;

use axum::{
    extract::DefaultBodyLimit,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde_json::json;

use crate::AppState;

pub mod download;
pub mod login;
pub mod owners;
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
    pub fn forbidden(msg: impl Into<String>) -> Self { Self(StatusCode::FORBIDDEN, msg.into()) }
    pub fn too_many_requests() -> Self { Self(StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded — slow down".into()) }
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

pub fn router(state: Arc<AppState>, max_upload_bytes: usize) -> Router {
    Router::new()
        // Public read
        .route("/api/v1/packages/:name", get(packages::get_package))
        .route("/api/v1/search", get(search::search_packages))
        .route("/api/v1/packages/:name/:version/download", get(download::download))
        // Auth-required
        .route("/api/v1/packages", put(publish::publish))
        .route("/api/v1/packages/:name/:version/yank", delete(yank::yank).put(yank::unyank))
        .route("/api/v1/packages/:name/owners", get(owners::list).put(owners::add).delete(owners::remove))
        .route("/api/v1/me", get(me))
        // Login
        .route("/api/v1/users/login", post(login::login))
        // Hard cap on request body size (applies to publish uploads)
        .layer(DefaultBodyLimit::max(max_upload_bytes))
        .with_state(state)
}

async fn me(auth: crate::auth::AuthToken) -> Json<serde_json::Value> {
    Json(json!({ "login": auth.user.username, "id": auth.user.id }))
}
