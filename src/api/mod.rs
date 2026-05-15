use std::sync::Arc;

use axum::{
    extract::{DefaultBodyLimit, Request},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};

use crate::AppState;

pub mod admin;
pub mod audit;
pub mod delete;
pub mod download;
pub mod email;
pub mod health;
pub mod login;
pub mod owners;
pub mod packages;
pub mod publish;
pub mod refresh;
pub mod register;
pub mod reset;
pub mod search;
pub mod totp;
pub mod yank;

pub type ApiResult<T> = Result<T, ApiError>;

pub struct ApiError(StatusCode, String);

impl ApiError {
    pub fn not_found(msg: impl Into<String>) -> Self      { Self(StatusCode::NOT_FOUND, msg.into()) }
    pub fn bad_request(msg: impl Into<String>) -> Self    { Self(StatusCode::BAD_REQUEST, msg.into()) }
    pub fn conflict(msg: impl Into<String>) -> Self       { Self(StatusCode::CONFLICT, msg.into()) }
    pub fn forbidden(msg: impl Into<String>) -> Self      { Self(StatusCode::FORBIDDEN, msg.into()) }
    #[allow(dead_code)]
    pub fn unauthorized(msg: impl Into<String>) -> Self   { Self(StatusCode::UNAUTHORIZED, msg.into()) }
    pub fn gone(msg: impl Into<String>) -> Self           { Self(StatusCode::GONE, msg.into()) }
    pub fn too_many_requests() -> Self                    { Self(StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded — slow down".into()) }
    pub fn internal(msg: impl Into<String>) -> Self       { Self(StatusCode::INTERNAL_SERVER_ERROR, msg.into()) }
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
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    Router::new()
        // Health (no auth, no rate limit)
        .route("/health",                                   get(health::health))
        // Public read
        .route("/api/v1/packages/:name",                   get(packages::get_package))
        .route("/api/v1/search",                           get(search::search_packages))
        .route("/api/v1/packages/:name/:version/download", get(download::download))
        // Auth-required
        .route("/api/v1/packages",                         put(publish::publish))
        .route("/api/v1/packages/:name/:version/yank",     delete(yank::yank).put(yank::unyank))
        .route("/api/v1/packages/:name/owners",            get(owners::list).put(owners::add).delete(owners::remove))
        .route("/api/v1/me",                               get(me))
        // TOTP management
        .route("/api/v1/me/totp/enroll",                   post(totp::enroll))
        .route("/api/v1/me/totp/confirm",                  post(totp::confirm))
        .route("/api/v1/me/totp",                          delete(totp::disable))
        // Admin
        .route("/api/v1/admin/users",                      get(admin::list_users))
        .route("/api/v1/admin/packages/:name",             delete(delete::delete_package))
        .route("/api/v1/audit",                            get(audit::list_audit))
        // Auth
        .route("/api/v1/users/login",                      post(login::login))
        .route("/api/v1/users/register",                   post(register::register))
        .route("/api/v1/auth/refresh",                     post(refresh::refresh))
        // Email / password reset (no auth)
        .route("/api/v1/users/verify-email",               get(email::verify_email))
        .route("/api/v1/users/reset-password/request",     post(reset::request_reset))
        .route("/api/v1/users/reset-password/confirm",     post(reset::confirm_reset))
        // Middleware (applied inside out: security headers → CORS → body limit)
        .layer(middleware::from_fn(security_headers))
        .layer(cors)
        .layer(DefaultBodyLimit::max(max_upload_bytes))
        .with_state(state)
}

async fn me(auth: crate::auth::AuthToken) -> Json<serde_json::Value> {
    Json(json!({ "login": auth.user.username, "id": auth.user.id }))
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut resp = next.run(request).await;
    let h = resp.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options",        HeaderValue::from_static("DENY"));
    h.insert("referrer-policy",        HeaderValue::from_static("strict-origin-when-cross-origin"));
    resp
}
