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
use tower_http::services::ServeDir;

use crate::AppState;

pub mod admin;
pub mod audit;
pub mod channels;
pub mod delete;
pub mod docs;
pub mod download;
pub mod email;
pub mod health;
pub mod keywords;
pub mod login;
pub mod metrics_handler;
pub mod me_packages;
pub mod me_password;
pub mod my_tokens;
pub mod oauth;
pub mod orgs;
pub mod owners;
pub mod packages;
pub mod prebuilt;
pub mod publish;
pub mod readme;
pub mod refresh;
pub mod register;
pub mod reset;
pub mod search;
pub mod stats;
pub mod totp;
pub mod user_profile;
pub mod verify_status;
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

/// Resolve a path to the `static/` directory.
///
/// At runtime, looks for `static/` in order:
/// 1. `FREIGHT_STATIC_DIR` env var
/// 2. Next to the running binary (production install)
/// 3. `crates/freight-registry/static/` relative to CWD (Cargo workspace dev layout)
/// 4. `static/` relative to CWD
fn static_dir() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("FREIGHT_STATIC_DIR") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe.parent().unwrap_or(std::path::Path::new(".")).join("static");
        if candidate.is_dir() {
            return candidate;
        }
    }
    // Cargo workspace: binary lives in target/debug/ two levels below the workspace root
    let dev_candidate = std::path::PathBuf::from("crates/freight-registry/static");
    if dev_candidate.is_dir() {
        return dev_candidate;
    }
    std::path::PathBuf::from("static")
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
        // Health + metrics + stats (no auth, no rate limit)
        .route("/health",                                   get(health::health))
        .route("/metrics",                                  get(metrics_handler::metrics))
        .route("/api/v1/stats",                             get(stats::stats))
        .route("/api/v1/keywords",                          get(keywords::keywords))
        .route("/api/v1/channels",                          get(channels::list_channels))
        // Public read
        .route("/api/v1/graph",                                      get(packages::get_graph))
        .route("/api/v1/packages/:name",                             get(packages::get_package))
        .route("/api/v1/packages/:name/:version/readme",              get(readme::get_readme).put(readme::put_readme))
        .route("/api/v1/packages/:name/:version/docs",                get(docs::get_docs).put(docs::put_docs))
        .route("/api/v1/search",                                     get(search::search_packages))
        .route("/api/v1/packages/:name/:version/download",           get(download::download))
        .route("/api/v1/packages/:name/:version/prebuilts",          get(prebuilt::list))
        .route("/api/v1/packages/:name/:version/prebuilt/:triple/download", get(prebuilt::download))
        // Auth-required
        .route("/api/v1/packages",                                   put(publish::publish))
        .route("/api/v1/packages/:name/:version/prebuilt/:triple",   put(prebuilt::upload))
        .route("/api/v1/packages/:name/:version/status",    get(verify_status::get_status))
        .route("/api/v1/packages/:name/:version/yank",     delete(yank::yank).put(yank::unyank))
        .route("/api/v1/packages/:name/owners",            get(owners::list).put(owners::add).delete(owners::remove))
        .route("/api/v1/me",                               get(me))
        .route("/api/v1/me/packages",                      get(me_packages::my_packages))
        .route("/api/v1/me/password",                      post(me_password::change_password))
        // TOTP management
        .route("/api/v1/me/totp/enroll",                   post(totp::enroll))
        .route("/api/v1/me/totp/confirm",                  post(totp::confirm))
        .route("/api/v1/me/totp",                          delete(totp::disable))
        // Token management (current user)
        .route("/api/v1/me/tokens",                        get(my_tokens::list).post(my_tokens::create))
        .route("/api/v1/me/tokens/:name",                  delete(my_tokens::revoke))
        // Orgs
        .route("/api/v1/orgs",                             get(orgs::list_orgs).post(orgs::create_org))
        .route("/api/v1/orgs/:name",                       get(orgs::get_org).delete(orgs::delete_org))
        .route("/api/v1/orgs/:name/members",               get(orgs::list_members).put(orgs::add_member))
        .route("/api/v1/orgs/:name/members/:username",     axum::routing::delete(orgs::remove_member))
        .route("/api/v1/packages/:name/:channel/org",      put(orgs::set_package_org))
        // Admin
        .route("/api/v1/admin/users",                      get(admin::list_users))
        .route("/api/v1/admin/users/:name/promote",        post(admin::promote_user))
        .route("/api/v1/admin/users/:name/demote",         post(admin::demote_user))
        .route("/api/v1/admin/users/:name",                delete(admin::remove_user))
        .route("/api/v1/admin/packages/:name",             delete(delete::delete_package))
        .route("/api/v1/audit",                            get(audit::list_audit))
        // Auth
        .route("/api/v1/users/login",                      post(login::login))
        .route("/api/v1/users/register",                   post(register::register))
        .route("/api/v1/auth/refresh",                     post(refresh::refresh))
        // OAuth / OIDC (provider name is part of the path: /auth/:provider)
        .route("/auth/:provider",          get(oauth::oauth_start))
        .route("/auth/:provider/callback", get(oauth::oauth_callback))
        // Public user profiles
        .route("/api/v1/users/:username",                  get(user_profile::get_user))
        // Email / password reset (no auth)
        .route("/api/v1/users/verify-email",               get(email::verify_email))
        .route("/api/v1/users/reset-password/request",     post(reset::request_reset))
        .route("/api/v1/users/reset-password/confirm",     post(reset::confirm_reset))
        // ── Static web UI ──────────────────────────────────────────────────────
        // /packages/:name  → package.html  (JS reads the name from the URL)
        // /                → index.html    (search + hero)
        // /style.css, /app.js, etc. → served by ServeDir fallback
        .route("/",                get(|()| serve_page("index.html")))
        .route("/graph",           get(|()| serve_page("graph.html")))
        .route("/docs",            get(|()| serve_page("docs/index.html")))
        .route("/docs/",           get(|()| serve_page("docs/index.html")))
        .route("/install",         get(|()| serve_page("install.html")))
        .route("/packages/:_name",      get(|()| serve_page("package.html")))
        .route("/packages/:_name/docs", get(|()| serve_page("docs.html")))
        .route("/login",           get(|()| serve_page("login.html")))
        .route("/register",        get(|()| serve_page("register.html")))
        .route("/account",         get(|()| serve_page("account.html")))
        .route("/users/:_name",    get(|()| serve_page("users.html")))
        .fallback_service({
            let dir = static_dir();
            ServeDir::new(&dir).fallback(tower::service_fn(|_req: axum::http::Request<axum::body::Body>| async {
                let path = static_dir().join("404.html");
                let body = tokio::fs::read(&path).await.unwrap_or_else(|_| b"404 Not Found".to_vec());
                Ok::<_, std::convert::Infallible>(
                    axum::response::Response::builder()
                        .status(axum::http::StatusCode::NOT_FOUND)
                        .header("content-type", "text/html; charset=utf-8")
                        .body(axum::body::Body::from(body))
                        .unwrap()
                )
            }))
        })
        // Middleware (applied inside out: security headers → CORS → body limit)
        .layer(middleware::from_fn(security_headers))
        .layer(cors)
        .layer(DefaultBodyLimit::max(max_upload_bytes))
        .with_state(state)
}

/// Serve a specific HTML file from the static directory.
async fn serve_page(filename: &'static str) -> impl IntoResponse {
    let path = static_dir().join(filename);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            bytes,
        ).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain")],
            b"404 - page not found".to_vec(),
        ).into_response(),
    }
}

async fn me(auth: crate::auth::AuthToken) -> Json<serde_json::Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let expires_at  = auth.token.expires_at;
    let expires_in  = expires_at.map(|t| (t - now).max(0));

    Json(json!({
        "login":            auth.user.username,
        "id":               auth.user.id,
        "email":            auth.user.email,
        "email_verified":   auth.user.email_verified != 0,
        "is_admin":         auth.user.is_admin != 0,
        "totp_enabled":     auth.user.totp_enabled != 0,
        "token_expires_at": expires_at,
        "token_expires_in": expires_in,
    }))
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut resp = next.run(request).await;
    let h = resp.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options",        HeaderValue::from_static("DENY"));
    h.insert("referrer-policy",        HeaderValue::from_static("strict-origin-when-cross-origin"));
    // CSP: allow same-origin resources; allow inline styles/scripts (needed for page-inline JS)
    h.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' 'unsafe-inline'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; \
             connect-src 'self'; \
             frame-ancestors 'none'",
        ),
    );
    resp
}
