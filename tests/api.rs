use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;

use axum::{body::Body, extract::ConnectInfo};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;

use freight_registry::{api, db::Db, metrics::Metrics, rate_limit::Limiters, storage::Storage, AppState};

// ── Test infrastructure ───────────────────────────────────────────────────────

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_dir() -> PathBuf {
    let n = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("freight-test-{}-{}", std::process::id(), n))
}

async fn make_state() -> Arc<AppState> {
    let db = Db::open_memory().await.unwrap();
    Arc::new(AppState {
        db,
        storage:         Storage::new(tmp_dir()),
        base_url:        "http://localhost".to_string(),
        limiters:        Limiters::new(100_000, 100_000),
        metrics:         Metrics::new(),
        mirror_upstream: None,
    })
}

fn ci() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234)))
}

async fn send(
    app:  axum::Router,
    req:  Request<Body>,
) -> (StatusCode, Value) {
    use tower::ServiceExt;
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

async fn send_text(app: axum::Router, req: Request<Body>) -> (StatusCode, String) {
    use tower::ServiceExt;
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .extension(ci())
        .body(Body::empty())
        .unwrap()
}

fn get_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci())
        .body(Body::empty())
        .unwrap()
}

fn post_json(uri: &str, body: impl serde::Serialize) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("Content-Type", "application/json")
        .extension(ci())
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn post_json_auth(uri: &str, token: &str, body: impl serde::Serialize) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci())
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn delete_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci())
        .body(Body::empty())
        .unwrap()
}

fn put_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci())
        .body(Body::empty())
        .unwrap()
}

fn put_json_auth(uri: &str, token: &str, body: impl serde::Serialize) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .extension(ci())
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

/// Register a new user and return their API token.
async fn do_register(state: &Arc<AppState>, username: &str, password: &str) -> String {
    use tower::ServiceExt;
    let app = api::router(state.clone(), 1024 * 1024);
    let req = post_json("/api/v1/users/register", serde_json::json!({
        "username": username,
        "password": password,
    }));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "register failed for {username}");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    body["token"].as_str().unwrap().to_string()
}

/// A minimal valid gzip archive (empty content).
const MINIMAL_GZIP: &[u8] = &[
    0x1f, 0x8b,                         // magic
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, // method + mtime
    0x00, 0xff,                         // xfl + OS
    0x03, 0x00,                         // empty deflate stream
    0x00, 0x00, 0x00, 0x00,             // CRC32
    0x00, 0x00, 0x00, 0x00,             // ISIZE
];

fn build_publish_body(name: &str, vers: &str) -> Vec<u8> {
    let meta = serde_json::json!({"name": name, "vers": vers}).to_string();
    let meta_bytes = meta.as_bytes();
    let mut body = Vec::new();
    body.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    body.extend_from_slice(meta_bytes);
    body.extend_from_slice(&(MINIMAL_GZIP.len() as u32).to_le_bytes());
    body.extend_from_slice(MINIMAL_GZIP);
    body
}

async fn do_publish(state: &Arc<AppState>, token: &str, name: &str, vers: &str) -> StatusCode {
    use tower::ServiceExt;
    let app = api::router(state.clone(), 1024 * 1024);
    let body = build_publish_body(name, vers);
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/packages")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/octet-stream")
        .extension(ci())
        .body(Body::from(body))
        .unwrap();
    app.oneshot(req).await.unwrap().status()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok() {
    let state = make_state().await;
    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get("/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["db"], "ok");
}

#[tokio::test]
async fn security_headers_present() {
    let state = make_state().await;
    let app = api::router(state, 1024 * 1024);
    use tower::ServiceExt;
    let resp = app.oneshot(get("/health")).await.unwrap();
    assert_eq!(resp.headers().get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
}

// ── Auth ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn register_ok() {
    let state = make_state().await;
    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, post_json("/api/v1/users/register", serde_json::json!({
        "username": "alice",
        "password": "pw123",
    }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["login"], "alice");
    assert!(body["token"].is_string());
}

#[tokio::test]
async fn register_duplicate_username_409() {
    let state = make_state().await;
    do_register(&state, "alice", "pw").await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, post_json("/api/v1/users/register", serde_json::json!({
        "username": "alice",
        "password": "pw2",
    }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn register_invalid_username_400() {
    let state = make_state().await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, post_json("/api/v1/users/register", serde_json::json!({
        "username": "-bad",
        "password": "pw",
    }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn login_ok_returns_tokens() {
    let state = make_state().await;
    do_register(&state, "alice", "mypassword").await;
    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, post_json("/api/v1/users/login", serde_json::json!({
        "username": "alice",
        "password": "mypassword",
    }))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].is_string());
    assert!(body["refresh_token"].is_string());
}

#[tokio::test]
async fn login_wrong_password_404() {
    let state = make_state().await;
    do_register(&state, "alice", "correct").await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, post_json("/api/v1/users/login", serde_json::json!({
        "username": "alice",
        "password": "wrong",
    }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn login_unknown_user_404() {
    let state = make_state().await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, post_json("/api/v1/users/login", serde_json::json!({
        "username": "nobody",
        "password": "pw",
    }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn me_requires_auth() {
    let state = make_state().await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, get("/api/v1/me")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_ok() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get_auth("/api/v1/me", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["login"], "alice");
}

#[tokio::test]
async fn refresh_token_cannot_be_used_for_api_auth() {
    let state = make_state().await;
    do_register(&state, "alice", "pw").await;
    let app = api::router(state.clone(), 1024 * 1024);
    let (_, login_body) = send(app, post_json("/api/v1/users/login", serde_json::json!({
        "username": "alice",
        "password": "pw",
    }))).await;
    let refresh_tok = login_body["refresh_token"].as_str().unwrap().to_string();

    let app2 = api::router(state, 1024 * 1024);
    let (status, _) = send(app2, get_auth("/api/v1/me", &refresh_tok)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_token_issues_new_access_token() {
    let state = make_state().await;
    do_register(&state, "alice", "pw").await;
    let app = api::router(state.clone(), 1024 * 1024);
    let (_, login_body) = send(app, post_json("/api/v1/users/login", serde_json::json!({
        "username": "alice",
        "password": "pw",
    }))).await;
    let refresh_tok = login_body["refresh_token"].as_str().unwrap().to_string();

    let app2 = api::router(state.clone(), 1024 * 1024);
    let (status, body) = send(app2, post_json_auth("/api/v1/auth/refresh", &refresh_tok, serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    let new_tok = body["token"].as_str().unwrap().to_string();

    let app3 = api::router(state, 1024 * 1024);
    let (status, me_body) = send(app3, get_auth("/api/v1/me", &new_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me_body["login"], "alice");
}

// ── Packages ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn publish_and_get_package() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    assert_eq!(do_publish(&state, &tok, "mylib", "1.0.0").await, StatusCode::OK);

    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get("/api/v1/packages/mylib")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "mylib");
    assert_eq!(body["versions"][0]["version"], "1.0.0");
}

#[tokio::test]
async fn publish_duplicate_409() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    assert_eq!(do_publish(&state, &tok, "mylib", "1.0.0").await, StatusCode::OK);
    assert_eq!(do_publish(&state, &tok, "mylib", "1.0.0").await, StatusCode::CONFLICT);
}

#[tokio::test]
async fn publish_requires_auth() {
    let state = make_state().await;
    let body = build_publish_body("mylib", "1.0.0");
    let app = api::router(state, 1024 * 1024);
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/packages")
        .header("Content-Type", "application/octet-stream")
        .extension(ci())
        .body(Body::from(body))
        .unwrap();
    let (status, _) = send(app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn publish_wrong_owner_403() {
    let state = make_state().await;
    let alice = do_register(&state, "alice", "pw").await;
    let bob = do_register(&state, "bob", "pw").await;
    assert_eq!(do_publish(&state, &alice, "mylib", "1.0.0").await, StatusCode::OK);
    assert_eq!(do_publish(&state, &bob, "mylib", "1.1.0").await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn get_package_not_found_404() {
    let state = make_state().await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, get("/api/v1/packages/nothing")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn download_ok_with_checksum_header() {
    use tower::ServiceExt;
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    do_publish(&state, &tok, "mylib", "1.0.0").await;

    let app = api::router(state, 1024 * 1024);
    let resp = app.oneshot(get("/api/v1/packages/mylib/1.0.0/download")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("x-checksum-sha256"));
    assert_eq!(resp.headers().get("content-type").unwrap(), "application/gzip");
}

#[tokio::test]
async fn download_not_found_404() {
    let state = make_state().await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, get("/api/v1/packages/ghost/1.0.0/download")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn yank_and_download_gone() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    do_publish(&state, &tok, "mylib", "1.0.0").await;

    let app = api::router(state.clone(), 1024 * 1024);
    let (status, _) = send(app, delete_auth("/api/v1/packages/mylib/1.0.0/yank", &tok)).await;
    assert_eq!(status, StatusCode::OK);

    let app2 = api::router(state, 1024 * 1024);
    let (status, _) = send(app2, get("/api/v1/packages/mylib/1.0.0/download")).await;
    assert_eq!(status, StatusCode::GONE);
}

#[tokio::test]
async fn unyank_allows_download_again() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    do_publish(&state, &tok, "mylib", "1.0.0").await;

    let app = api::router(state.clone(), 1024 * 1024);
    send(app, delete_auth("/api/v1/packages/mylib/1.0.0/yank", &tok)).await;

    let app2 = api::router(state.clone(), 1024 * 1024);
    let (status, _) = send(app2, put_auth("/api/v1/packages/mylib/1.0.0/yank", &tok)).await;
    assert_eq!(status, StatusCode::OK);

    let app3 = api::router(state, 1024 * 1024);
    let (status, _) = send(app3, get("/api/v1/packages/mylib/1.0.0/download")).await;
    assert_eq!(status, StatusCode::OK);
}

// ── Search ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn search_returns_results() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    do_publish(&state, &tok, "awesome-lib", "1.0.0").await;
    do_publish(&state, &tok, "boring-tool", "1.0.0").await;

    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get("/api/v1/search?q=awesome")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["packages"][0]["name"], "awesome-lib");
}

#[tokio::test]
async fn search_pagination_fields() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    for i in 0..5 {
        do_publish(&state, &tok, &format!("pkg-{i}"), "1.0.0").await;
    }

    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get("/api/v1/search?q=pkg&limit=2&offset=2")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 5);
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 2);
    assert_eq!(body["packages"].as_array().unwrap().len(), 2);
}

// ── Owners ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn owners_list_and_add_remove() {
    let state = make_state().await;
    let alice = do_register(&state, "alice", "pw").await;
    do_register(&state, "bob", "pw").await;
    do_publish(&state, &alice, "mylib", "1.0.0").await;

    let app = api::router(state.clone(), 1024 * 1024);
    let (status, body) = send(app, get("/api/v1/packages/mylib/owners")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["users"].as_array().unwrap().len(), 1);

    let app2 = api::router(state.clone(), 1024 * 1024);
    let (status, _) = send(app2, put_json_auth("/api/v1/packages/mylib/owners", &alice,
        serde_json::json!({"users": ["bob"]}))).await;
    assert_eq!(status, StatusCode::OK);

    let app3 = api::router(state.clone(), 1024 * 1024);
    let (status, body) = send(app3, get("/api/v1/packages/mylib/owners")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["users"].as_array().unwrap().len(), 2);

    let app4 = api::router(state, 1024 * 1024);
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/packages/mylib/owners")
        .header("Authorization", format!("Bearer {alice}"))
        .header("Content-Type", "application/json")
        .extension(ci())
        .body(Body::from(serde_json::to_vec(&serde_json::json!({"users": ["bob"]})).unwrap()))
        .unwrap();
    let (status, _) = send(app4, req).await;
    assert_eq!(status, StatusCode::OK);
}

// ── Admin ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_list_users_requires_admin() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, get_auth("/api/v1/admin/users", &tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_list_users_ok() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let user = state.db.get_user_by_username("alice").await.unwrap().unwrap();
    state.db.set_admin(&user.username, true).await.unwrap();

    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get_auth("/api/v1/admin/users", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body["users"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn admin_delete_package() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    do_publish(&state, &tok, "mylib", "1.0.0").await;
    state.db.set_admin("alice", true).await.unwrap();

    let app = api::router(state.clone(), 1024 * 1024);
    let (status, _) = send(app, delete_auth("/api/v1/admin/packages/mylib", &tok)).await;
    assert_eq!(status, StatusCode::OK);

    let app2 = api::router(state, 1024 * 1024);
    let (status, _) = send(app2, get("/api/v1/packages/mylib")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_delete_package_nonexistent_404() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    state.db.set_admin("alice", true).await.unwrap();

    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, delete_auth("/api/v1/admin/packages/ghost", &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn audit_log_requires_admin() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, get_auth("/api/v1/audit", &tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn audit_log_ok_for_admin() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    state.db.set_admin("alice", true).await.unwrap();

    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get_auth("/api/v1/audit", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["entries"].is_array());
}

// ── Publish wire format edge cases ────────────────────────────────────────────

#[tokio::test]
async fn publish_invalid_gzip_400() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let meta = serde_json::json!({"name": "mylib", "vers": "1.0.0"}).to_string();
    let meta_bytes = meta.as_bytes();
    let tarball = b"not a gzip file";
    let mut body = Vec::new();
    body.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    body.extend_from_slice(meta_bytes);
    body.extend_from_slice(&(tarball.len() as u32).to_le_bytes());
    body.extend_from_slice(tarball);

    let app = api::router(state, 1024 * 1024);
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/packages")
        .header("Authorization", format!("Bearer {tok}"))
        .extension(ci())
        .body(Body::from(body))
        .unwrap();
    let (status, _) = send(app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn publish_invalid_package_name_400() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let meta = serde_json::json!({"name": "-bad-name", "vers": "1.0.0"}).to_string();
    let meta_bytes = meta.as_bytes();
    let mut body = Vec::new();
    body.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    body.extend_from_slice(meta_bytes);
    body.extend_from_slice(&(MINIMAL_GZIP.len() as u32).to_le_bytes());
    body.extend_from_slice(MINIMAL_GZIP);

    let app = api::router(state, 1024 * 1024);
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/packages")
        .header("Authorization", format!("Bearer {tok}"))
        .extension(ci())
        .body(Body::from(body))
        .unwrap();
    let (status, _) = send(app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ── Token management endpoints ────────────────────────────────────────────────

#[tokio::test]
async fn my_tokens_list_ok() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get_auth("/api/v1/me/tokens", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body["tokens"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn my_tokens_create_and_revoke() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let app = api::router(state.clone(), 1024 * 1024);
    let (status, body) = send(app, post_json_auth("/api/v1/me/tokens", &tok,
        serde_json::json!({"name": "mytoken"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].is_string());
    let app2 = api::router(state.clone(), 1024 * 1024);
    let (status, _) = send(app2, delete_auth("/api/v1/me/tokens/mytoken", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let app3 = api::router(state, 1024 * 1024);
    let (status, _) = send(app3, delete_auth("/api/v1/me/tokens/mytoken", &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn my_tokens_duplicate_409() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let app = api::router(state.clone(), 1024 * 1024);
    send(app, post_json_auth("/api/v1/me/tokens", &tok,
        serde_json::json!({"name": "dupe"}))).await;
    let app2 = api::router(state, 1024 * 1024);
    let (status, _) = send(app2, post_json_auth("/api/v1/me/tokens", &tok,
        serde_json::json!({"name": "dupe"}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

// ── User admin endpoints ──────────────────────────────────────────────────────

fn post_empty_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST").uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci())
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn admin_promote_and_demote() {
    let state = make_state().await;
    let admin_tok = do_register(&state, "admin", "pw").await;
    do_register(&state, "alice", "pw").await;
    state.db.set_admin("admin", true).await.unwrap();
    let app = api::router(state.clone(), 1024 * 1024);
    let (status, _) = send(app,
        post_empty_auth("/api/v1/admin/users/alice/promote", &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.db.get_user_by_username("alice").await.unwrap().unwrap().is_admin, 1);
    let app2 = api::router(state.clone(), 1024 * 1024);
    let (status, _) = send(app2,
        post_empty_auth("/api/v1/admin/users/alice/demote", &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.db.get_user_by_username("alice").await.unwrap().unwrap().is_admin, 0);
}

#[tokio::test]
async fn admin_remove_user_via_http() {
    let state = make_state().await;
    let admin_tok = do_register(&state, "admin", "pw").await;
    do_register(&state, "alice", "pw").await;
    state.db.set_admin("admin", true).await.unwrap();
    let app = api::router(state.clone(), 1024 * 1024);
    let (status, _) = send(app, delete_auth("/api/v1/admin/users/alice", &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(state.db.get_user_by_username("alice").await.unwrap().is_none());
}

#[tokio::test]
async fn admin_remove_nonexistent_user_404() {
    let state = make_state().await;
    let admin_tok = do_register(&state, "admin", "pw").await;
    state.db.set_admin("admin", true).await.unwrap();
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, delete_auth("/api/v1/admin/users/nobody", &admin_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── Metrics endpoint ──────────────────────────────────────────────────────────

#[tokio::test]
async fn metrics_returns_prometheus_text() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    do_publish(&state, &tok, "mylib", "1.0.0").await;

    let app = api::router(state, 1024 * 1024);
    let (status, body) = send_text(app, get("/metrics")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("freight_packages"), "missing freight_packages in: {body}");
    assert!(body.contains("freight_versions"),  "missing freight_versions in: {body}");
    assert!(body.contains("freight_users"),     "missing freight_users in: {body}");
    assert!(body.contains("freight_publishes"), "missing freight_publishes in: {body}");
}

// ── Token scopes (C1) ─────────────────────────────────────────────────────────

fn put_json_auth_body(uri: &str, token: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/octet-stream")
        .extension(ci())
        .body(Body::from(body))
        .unwrap()
}

async fn create_scoped_token(state: &Arc<AppState>, user_token: &str, scope: &str) -> String {
    use tower::ServiceExt;
    let app = api::router(state.clone(), 1024 * 1024);
    let (status, body) = send(app, post_json_auth("/api/v1/me/tokens", user_token,
        serde_json::json!({ "name": format!("tok-{scope}"), "scope": scope }))).await;
    assert_eq!(status, StatusCode::OK, "create {scope} token failed: {body}");
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn read_scope_token_cannot_publish() {
    let state = make_state().await;
    let publish_tok = do_register(&state, "alice", "pw").await;
    let read_tok = create_scoped_token(&state, &publish_tok, "read").await;

    let body = build_publish_body("mylib", "1.0.0");
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, put_json_auth_body("/api/v1/packages", &read_tok, body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn read_scope_token_can_list_tokens() {
    let state = make_state().await;
    let publish_tok = do_register(&state, "alice", "pw").await;
    let read_tok = create_scoped_token(&state, &publish_tok, "read").await;

    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get_auth("/api/v1/me/tokens", &read_tok)).await;
    assert_eq!(status, StatusCode::OK, "read token should list tokens: {body}");
}

#[tokio::test]
async fn read_scope_token_cannot_create_token() {
    let state = make_state().await;
    let publish_tok = do_register(&state, "alice", "pw").await;
    let read_tok = create_scoped_token(&state, &publish_tok, "read").await;

    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, post_json_auth("/api/v1/me/tokens", &read_tok,
        serde_json::json!({ "name": "new-tok" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn token_scope_returned_in_list() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let _read_tok = create_scoped_token(&state, &tok, "read").await;

    let app = api::router(state, 1024 * 1024);
    let (status, body) = send(app, get_auth("/api/v1/me/tokens", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let tokens = body["tokens"].as_array().unwrap();
    assert!(tokens.iter().any(|t| t["scope"].as_str() == Some("read")));
}

#[tokio::test]
async fn invalid_token_scope_rejected() {
    let state = make_state().await;
    let tok = do_register(&state, "alice", "pw").await;
    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, post_json_auth("/api/v1/me/tokens", &tok,
        serde_json::json!({ "name": "bad", "scope": "superadmin" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn read_scope_token_blocked_from_admin_endpoint() {
    let state = make_state().await;
    let tok = do_register(&state, "admin", "pw").await;
    state.db.set_admin("admin", true).await.unwrap();
    let read_tok = create_scoped_token(&state, &tok, "read").await;

    let app = api::router(state, 1024 * 1024);
    let (status, _) = send(app, get_auth("/api/v1/admin/users", &read_tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
