/// Integration tests: publish → download → yank flow, TOTP enforcement,
/// org role gating, and org-scoped token enforcement (E4).
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;

use axum::{body::Body, extract::ConnectInfo};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};

use freight_registry::{api, db::Db, mail::StdoutMailer, metrics::Metrics, rate_limit::Limiters, storage::Storage, AppState, ScanBackend};

// ── Infrastructure (mirrors tests/api.rs) ─────────────────────────────────────

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_dir() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("freight-integ-{}-{}", std::process::id(), n))
}

async fn make_state() -> Arc<AppState> {
    let db = Db::open_memory().await.unwrap();
    Arc::new(AppState {
        db,
        storage:               Storage::new(tmp_dir()),
        base_url:              "http://localhost".to_string(),
        limiters:              Limiters::new(100_000, 100_000),
        metrics:               Metrics::new(),
        mailer:                Arc::new(StdoutMailer),
        mirror_upstream:       None,
        max_packages_per_user: None,
        allowed_languages:     None,
        scan_backend:          ScanBackend::None,
        verify_image:          None,
        verify_images:         std::collections::HashMap::new(),
        download_url:          None,
        oauth_providers:       vec![],
        oauth_states:          Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    })
}

fn ci() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234)))
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    use tower::ServiceExt;
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).extension(ci()).body(Body::empty()).unwrap()
}

fn get_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET").uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci()).body(Body::empty()).unwrap()
}

fn post_json_auth(uri: &str, token: &str, body: impl serde::Serialize) -> Request<Body> {
    Request::builder()
        .method("POST").uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci())
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()
}

fn delete_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE").uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci()).body(Body::empty()).unwrap()
}

fn put_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("PUT").uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci()).body(Body::empty()).unwrap()
}

fn put_json_auth(uri: &str, token: &str, body: impl serde::Serialize) -> Request<Body> {
    Request::builder()
        .method("PUT").uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci())
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()
}

/// Register a user and return their publish token.
async fn register(state: &Arc<AppState>, username: &str, password: &str) -> String {
    let app = api::router(state.clone(), 50 * 1024 * 1024);
    let (status, body) = send(
        app,
        Request::builder()
            .method("POST")
            .uri("/api/v1/users/register")
            .header("Content-Type", "application/json")
            .extension(ci())
            .body(Body::from(serde_json::to_vec(&json!({
                "username": username,
                "password": password,
                "email":    format!("{username}@example.com"),
            })).unwrap()))
            .unwrap(),
    ).await;
    assert_eq!(status, StatusCode::OK, "register failed: {body}");
    body["token"].as_str().unwrap().to_string()
}

/// Build a minimal valid gzip-compressed tarball publish body (matches wire format).
fn build_publish_body(name: &str, vers: &str) -> Vec<u8> {
    use std::io::Write;
    let meta = serde_json::to_vec(&json!({
        "name": name, "vers": vers,
        "description": "test", "license": "MIT",
    })).unwrap();
    // Minimal gzip bytes (empty tarball — enough to pass the gzip magic check).
    let gz: Vec<u8> = {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(b"").unwrap();
        enc.finish().unwrap()
    };
    let mut body = Vec::new();
    body.extend_from_slice(&(meta.len() as u32).to_le_bytes());
    body.extend_from_slice(&meta);
    body.extend_from_slice(&(gz.len() as u32).to_le_bytes());
    body.extend_from_slice(&gz);
    body
}

async fn publish(state: &Arc<AppState>, token: &str, name: &str, vers: &str) -> StatusCode {
    let app = api::router(state.clone(), 50 * 1024 * 1024);
    let body = build_publish_body(name, vers);
    let (status, _) = send(
        app,
        Request::builder()
            .method("PUT")
            .uri("/api/v1/packages")
            .header("Content-Type", "application/octet-stream")
            .header("Authorization", format!("Bearer {token}"))
            .extension(ci())
            .body(Body::from(body))
            .unwrap(),
    ).await;
    status
}

// ── Publish → download → yank flow ───────────────────────────────────────────

#[tokio::test]
async fn full_publish_download_yank_flow() {
    let state = make_state().await;
    let token = register(&state, "alice", "password123").await;

    // Publish
    assert_eq!(publish(&state, &token, "mylib", "1.0.0").await, StatusCode::OK);

    let app = api::router(state.clone(), 50 * 1024 * 1024);

    // Package appears in get_package
    let (status, body) = send(app.clone(), get("/api/v1/packages/mylib")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "mylib");

    // Download returns 200
    let (dl_status, _) = send(app.clone(), get("/api/v1/packages/mylib/1.0.0/download")).await;
    assert_eq!(dl_status, StatusCode::OK);

    // Yank the version
    let (yank_status, _) = send(
        app.clone(),
        delete_auth("/api/v1/packages/mylib/1.0.0/yank", &token),
    ).await;
    assert_eq!(yank_status, StatusCode::OK);

    // Download is now 410 Gone (yanked)
    let (dl_after, _) = send(app.clone(), get("/api/v1/packages/mylib/1.0.0/download")).await;
    assert_eq!(dl_after, StatusCode::GONE);

    // Un-yank and download works again
    let (unyank_status, _) = send(
        app.clone(),
        put_auth("/api/v1/packages/mylib/1.0.0/yank", &token),
    ).await;
    assert_eq!(unyank_status, StatusCode::OK);

    let (dl_restored, _) = send(app.clone(), get("/api/v1/packages/mylib/1.0.0/download")).await;
    assert_eq!(dl_restored, StatusCode::OK);
}

#[tokio::test]
async fn duplicate_version_rejected() {
    let state = make_state().await;
    let token = register(&state, "bob", "password123").await;
    assert_eq!(publish(&state, &token, "duplib", "0.1.0").await, StatusCode::OK);
    assert_eq!(publish(&state, &token, "duplib", "0.1.0").await, StatusCode::CONFLICT);
}

#[tokio::test]
async fn non_owner_cannot_publish_new_version() {
    let state = make_state().await;
    let alice = register(&state, "alice2", "password123").await;
    let bob   = register(&state, "bob2",   "password123").await;
    assert_eq!(publish(&state, &alice, "alicelib", "1.0.0").await, StatusCode::OK);
    assert_eq!(publish(&state, &bob,   "alicelib", "1.0.1").await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn pending_version_not_downloadable() {
    let state = make_state().await;
    let token = register(&state, "charlie", "password123").await;
    assert_eq!(publish(&state, &token, "pendlib", "1.0.0").await, StatusCode::OK);

    // Manually set the version to pending.
    state.db.set_version_status("pendlib", "1.0.0", "stable", "pending", None).await.unwrap();

    let app = api::router(state.clone(), 50 * 1024 * 1024);
    let (status, _) = send(app, get("/api/v1/packages/pendlib/1.0.0/download")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── TOTP enforcement ──────────────────────────────────────────────────────────

#[tokio::test]
async fn login_requires_totp_when_enabled() {
    let state = make_state().await;
    let token = register(&state, "totp_user", "password123").await;
    let app = api::router(state.clone(), 50 * 1024 * 1024);

    // Enroll TOTP — stores the secret.
    let (enroll_status, _enroll_body) = send(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/api/v1/me/totp/enroll")
            .header("Authorization", format!("Bearer {token}"))
            .extension(ci())
            .body(Body::empty())
            .unwrap(),
    ).await;
    assert_eq!(enroll_status, StatusCode::OK);

    // Manually enable TOTP on the user (skipping TOTP code verification for the test).
    let user = state.db.get_user_by_username("totp_user").await.unwrap().unwrap();
    state.db.enable_totp(user.id, true).await.unwrap();

    // Login without TOTP code → 400.
    let (status, body) = send(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/api/v1/users/login")
            .header("Content-Type", "application/json")
            .extension(ci())
            .body(Body::from(serde_json::to_vec(&json!({
                "username": "totp_user",
                "password": "password123",
            })).unwrap()))
            .unwrap(),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "expected 400 without TOTP: {body}");
}

#[tokio::test]
async fn totp_recovery_code_allows_login() {
    let state = make_state().await;
    let token = register(&state, "recov_user", "password123").await;
    let app = api::router(state.clone(), 50 * 1024 * 1024);

    // Enroll TOTP secret.
    let (_, enroll_body) = send(
        app.clone(),
        Request::builder()
            .method("POST").uri("/api/v1/me/totp/enroll")
            .header("Authorization", format!("Bearer {token}"))
            .extension(ci()).body(Body::empty()).unwrap(),
    ).await;
    let secret = enroll_body["secret"].as_str().unwrap().to_string();

    // Confirm with a valid TOTP code to activate and get recovery codes.
    use totp_rs::{Algorithm, TOTP, Secret};
    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, Secret::Encoded(secret).to_bytes().unwrap(), None, "recov_user".to_string()).unwrap();
    let code = totp.generate_current().unwrap();

    let (confirm_status, confirm_body) = send(
        app.clone(),
        post_json_auth("/api/v1/me/totp/confirm", &token, json!({ "code": code })),
    ).await;
    assert_eq!(confirm_status, StatusCode::OK, "confirm failed: {confirm_body}");
    let recovery_codes = confirm_body["recovery_codes"].as_array().unwrap().clone();
    assert_eq!(recovery_codes.len(), 8);

    let recovery = recovery_codes[0].as_str().unwrap().to_string();

    // Login with a recovery code instead of TOTP → 200.
    let (login_status, login_body) = send(
        app.clone(),
        Request::builder()
            .method("POST").uri("/api/v1/users/login")
            .header("Content-Type", "application/json")
            .extension(ci())
            .body(Body::from(serde_json::to_vec(&json!({
                "username":  "recov_user",
                "password":  "password123",
                "totp_code": recovery,
            })).unwrap()))
            .unwrap(),
    ).await;
    assert_eq!(login_status, StatusCode::OK, "login with recovery failed: {login_body}");

    // Using the same recovery code again → 400 (already consumed).
    let (reuse_status, _) = send(
        app.clone(),
        Request::builder()
            .method("POST").uri("/api/v1/users/login")
            .header("Content-Type", "application/json")
            .extension(ci())
            .body(Body::from(serde_json::to_vec(&json!({
                "username":  "recov_user",
                "password":  "password123",
                "totp_code": recovery,
            })).unwrap()))
            .unwrap(),
    ).await;
    assert_eq!(reuse_status, StatusCode::BAD_REQUEST, "reused recovery code should fail");
}

// ── Org role gating ───────────────────────────────────────────────────────────

#[tokio::test]
async fn only_org_owner_can_add_member() {
    let state = make_state().await;
    let owner  = register(&state, "org_owner",  "password123").await;
    let member = register(&state, "org_member", "password123").await;
    let app = api::router(state.clone(), 50 * 1024 * 1024);

    // Owner creates the org.
    let (status, body) = send(
        app.clone(),
        post_json_auth("/api/v1/orgs", &owner, json!({ "name": "myorg" })),
    ).await;
    assert_eq!(status, StatusCode::OK, "create org failed: {body}");

    // Member cannot add another user (PUT /api/v1/orgs/:name/members).
    let (status, _) = send(
        app.clone(),
        put_json_auth(
            "/api/v1/orgs/myorg/members", &member,
            json!({ "username": "org_owner", "role": "member" }),
        ),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Owner can add a member.
    let (status, _) = send(
        app.clone(),
        put_json_auth(
            "/api/v1/orgs/myorg/members", &owner,
            json!({ "username": "org_member", "role": "member" }),
        ),
    ).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn only_org_owner_can_set_package_org() {
    let state = make_state().await;
    let owner  = register(&state, "pkgorg_owner",  "password123").await;
    let member = register(&state, "pkgorg_member", "password123").await;
    let app = api::router(state.clone(), 50 * 1024 * 1024);

    // Owner creates org and adds member.
    send(app.clone(), post_json_auth("/api/v1/orgs", &owner, json!({ "name": "pkgorg" }))).await;
    send(app.clone(), post_json_auth(
        "/api/v1/orgs/pkgorg/members", &owner,
        json!({ "username": "pkgorg_member", "role": "member" }),
    )).await;

    // Owner publishes a package.
    assert_eq!(publish(&state, &owner, "orgpkg", "1.0.0").await, StatusCode::OK);

    // Member (not owner) tries to assign the package to the org → 403.
    let (status, _) = send(
        app.clone(),
        put_json_auth(
            "/api/v1/packages/orgpkg/stable/org", &member,
            json!({ "org": "pkgorg" }),
        ),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Owner can assign it.
    let (status, _) = send(
        app.clone(),
        put_json_auth(
            "/api/v1/packages/orgpkg/stable/org", &owner,
            json!({ "org": "pkgorg" }),
        ),
    ).await;
    assert_eq!(status, StatusCode::OK);
}

// ── Org-scoped token enforcement (E4) ─────────────────────────────────────────

#[tokio::test]
async fn org_scoped_token_cannot_publish_outside_org() {
    let state = make_state().await;
    let owner = register(&state, "e4_owner", "password123").await;
    let other = register(&state, "e4_other", "password123").await;
    let app = api::router(state.clone(), 50 * 1024 * 1024);

    // Owner creates org.
    send(app.clone(), post_json_auth("/api/v1/orgs", &owner, json!({ "name": "e4org" }))).await;

    // Owner creates an org-scoped publish token.
    let (status, body) = send(
        app.clone(),
        post_json_auth("/api/v1/me/tokens", &owner, json!({
            "name":  "ci-token",
            "scope": "publish",
            "org":   "e4org",
        })),
    ).await;
    assert_eq!(status, StatusCode::OK, "token creation failed: {body}");
    let ci_token = body["token"].as_str().unwrap().to_string();

    // Publish a package owned by "other" that is NOT in e4org.
    assert_eq!(publish(&state, &other, "foreignpkg", "1.0.0").await, StatusCode::OK);

    // ci_token (bound to e4org) cannot publish a new version of foreignpkg.
    assert_eq!(publish(&state, &ci_token, "foreignpkg", "1.0.1").await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn org_scoped_token_can_publish_within_org() {
    let state = make_state().await;
    let owner = register(&state, "e4b_owner", "password123").await;
    let app = api::router(state.clone(), 50 * 1024 * 1024);

    // Owner creates org.
    send(app.clone(), post_json_auth("/api/v1/orgs", &owner, json!({ "name": "e4borg" }))).await;

    // Publish first version with a regular token (assigns owner, not org yet).
    assert_eq!(publish(&state, &owner, "orgpkg2", "1.0.0").await, StatusCode::OK);

    // Assign the package to the org.
    send(app.clone(), put_json_auth(
        "/api/v1/packages/orgpkg2/stable/org", &owner,
        json!({ "org": "e4borg" }),
    )).await;

    // Create org-scoped token.
    let (_, body) = send(
        app.clone(),
        post_json_auth("/api/v1/me/tokens", &owner, json!({
            "name":  "ci-org",
            "scope": "publish",
            "org":   "e4borg",
        })),
    ).await;
    let ci_token = body["token"].as_str().unwrap().to_string();

    // Org-scoped token CAN publish a new version of orgpkg2.
    assert_eq!(publish(&state, &ci_token, "orgpkg2", "1.0.1").await, StatusCode::OK);
}

#[tokio::test]
async fn only_org_owner_can_create_org_scoped_token() {
    let state = make_state().await;
    let owner  = register(&state, "tokorg_owner",  "password123").await;
    let member = register(&state, "tokorg_member", "password123").await;
    let app = api::router(state.clone(), 50 * 1024 * 1024);

    // Create org and add member.
    send(app.clone(), post_json_auth("/api/v1/orgs", &owner, json!({ "name": "tokorg" }))).await;
    send(app.clone(), post_json_auth(
        "/api/v1/orgs/tokorg/members", &owner,
        json!({ "username": "tokorg_member", "role": "member" }),
    )).await;

    // Member cannot create an org-scoped token → 403.
    let (status, _) = send(
        app.clone(),
        post_json_auth("/api/v1/me/tokens", &member, json!({
            "name":  "ci",
            "scope": "publish",
            "org":   "tokorg",
        })),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ── Reports + admin overview ──────────────────────────────────────────────────

fn patch_json_auth(uri: &str, token: &str, body: impl serde::Serialize) -> Request<Body> {
    Request::builder()
        .method("PATCH").uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .extension(ci())
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()
}

/// Register a user and promote them to admin; returns their token.
async fn register_admin(state: &Arc<AppState>, username: &str) -> String {
    let token = register(state, username, "pw-admin-123").await;
    state.db.set_admin(username, true).await.unwrap();
    token
}

#[tokio::test]
async fn report_flow_file_list_resolve() {
    let state = make_state().await;
    let owner = register(&state, "rep_owner", "pw-123456").await;
    let reporter = register(&state, "rep_user", "pw-123456").await;
    let admin = register_admin(&state, "rep_admin").await;
    assert_eq!(publish(&state, &owner, "badpkg", "1.0.0").await, StatusCode::OK);

    // A regular user files a report.
    let (st, _) = send(
        api::router(state.clone(), 1 << 20),
        post_json_auth("/api/v1/packages/badpkg/report", &reporter,
            json!({ "reason": "malware", "details": "ships a miner", "version": "1.0.0" })),
    ).await;
    assert_eq!(st, StatusCode::OK);

    // Admin sees it in the open list.
    let (st, body) = send(
        api::router(state.clone(), 1 << 20),
        get_auth("/api/v1/admin/reports?status=open", &admin),
    ).await;
    assert_eq!(st, StatusCode::OK);
    let reports = body["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["package"], "badpkg");
    assert_eq!(reports[0]["reason"], "malware");
    let id = reports[0]["id"].as_i64().unwrap();

    // A non-admin cannot list reports.
    let (st, _) = send(
        api::router(state.clone(), 1 << 20),
        get_auth("/api/v1/admin/reports", &reporter),
    ).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Admin resolves it.
    let (st, _) = send(
        api::router(state.clone(), 1 << 20),
        patch_json_auth(&format!("/api/v1/admin/reports/{id}"), &admin,
            json!({ "status": "resolved", "note": "yanked the version" })),
    ).await;
    assert_eq!(st, StatusCode::OK);

    // Open list is now empty.
    let (_, body) = send(
        api::router(state.clone(), 1 << 20),
        get_auth("/api/v1/admin/reports?status=open", &admin),
    ).await;
    assert_eq!(body["reports"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn report_validation() {
    let state = make_state().await;
    let owner = register(&state, "v_owner", "pw-123456").await;
    let user = register(&state, "v_user", "pw-123456").await;
    assert_eq!(publish(&state, &owner, "vpkg", "1.0.0").await, StatusCode::OK);

    // Unknown reason → 400.
    let (st, _) = send(
        api::router(state.clone(), 1 << 20),
        post_json_auth("/api/v1/packages/vpkg/report", &user, json!({ "reason": "because" })),
    ).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // Report against a nonexistent package → 404.
    let (st, _) = send(
        api::router(state.clone(), 1 << 20),
        post_json_auth("/api/v1/packages/ghost/report", &user, json!({ "reason": "spam" })),
    ).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Anonymous report → 401.
    let (st, _) = send(
        api::router(state.clone(), 1 << 20),
        Request::builder().method("POST").uri("/api/v1/packages/vpkg/report")
            .header("Content-Type", "application/json").extension(ci())
            .body(Body::from(serde_json::to_vec(&json!({ "reason": "spam" })).unwrap())).unwrap(),
    ).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_overview_counts() {
    let state = make_state().await;
    let owner = register(&state, "ov_owner", "pw-123456").await;
    let admin = register_admin(&state, "ov_admin").await;
    assert_eq!(publish(&state, &owner, "ovpkg", "1.0.0").await, StatusCode::OK);
    let (_, _) = send(
        api::router(state.clone(), 1 << 20),
        post_json_auth("/api/v1/packages/ovpkg/report", &owner, json!({ "reason": "other" })),
    ).await;

    let (st, body) = send(
        api::router(state.clone(), 1 << 20),
        get_auth("/api/v1/admin/overview", &admin),
    ).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["packages"], 1);
    assert_eq!(body["open_reports"], 1);
    assert!(body["users"].as_i64().unwrap() >= 2);
    assert!(body["admins"].as_i64().unwrap() >= 1);

    // Non-admin is forbidden.
    let (st, _) = send(
        api::router(state.clone(), 1 << 20),
        get_auth("/api/v1/admin/overview", &owner),
    ).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}
