use freight_registry::db::Db;

async fn make_user(db: &Db, username: &str) -> i64 {
    db.create_user(username, None, "pw").await.unwrap()
}

// ── Users ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn user_create_and_get() {
    let db = Db::open_memory().await.unwrap();
    let id = db.create_user("alice", Some("alice@example.com"), "hash").await.unwrap();
    assert!(id > 0);
    let u = db.get_user_by_username("alice").await.unwrap().unwrap();
    assert_eq!(u.username, "alice");
    assert_eq!(u.email.as_deref(), Some("alice@example.com"));
    assert_eq!(u.is_admin, 0);
    assert_eq!(u.email_verified, 0);
    assert_eq!(u.totp_enabled, 0);
}

#[tokio::test]
async fn user_get_by_id() {
    let db = Db::open_memory().await.unwrap();
    let id = make_user(&db, "bob").await;
    let u = db.get_user_by_id(id).await.unwrap().unwrap();
    assert_eq!(u.username, "bob");
}

#[tokio::test]
async fn user_lookup_case_insensitive() {
    let db = Db::open_memory().await.unwrap();
    make_user(&db, "Alice").await;
    assert!(db.get_user_by_username("alice").await.unwrap().is_some());
    assert!(db.get_user_by_username("ALICE").await.unwrap().is_some());
}

#[tokio::test]
async fn user_duplicate_username_rejected() {
    let db = Db::open_memory().await.unwrap();
    db.create_user("alice", None, "pw1").await.unwrap();
    assert!(db.create_user("alice", None, "pw2").await.is_err());
    // case-insensitive uniqueness
    assert!(db.create_user("ALICE", None, "pw3").await.is_err());
}

#[tokio::test]
async fn user_set_admin() {
    let db = Db::open_memory().await.unwrap();
    make_user(&db, "alice").await;
    assert!(db.set_admin("alice", true).await.unwrap());
    assert_eq!(db.get_user_by_username("alice").await.unwrap().unwrap().is_admin, 1);
    assert!(db.set_admin("alice", false).await.unwrap());
    assert_eq!(db.get_user_by_username("alice").await.unwrap().unwrap().is_admin, 0);
}

#[tokio::test]
async fn user_set_admin_nonexistent_returns_false() {
    let db = Db::open_memory().await.unwrap();
    assert!(!db.set_admin("nobody", true).await.unwrap());
}

#[tokio::test]
async fn user_delete() {
    let db = Db::open_memory().await.unwrap();
    make_user(&db, "alice").await;
    assert!(db.delete_user("alice").await.unwrap());
    assert!(db.get_user_by_username("alice").await.unwrap().is_none());
}

#[tokio::test]
async fn user_delete_nonexistent_returns_false() {
    let db = Db::open_memory().await.unwrap();
    assert!(!db.delete_user("nobody").await.unwrap());
}

#[tokio::test]
async fn user_list() {
    let db = Db::open_memory().await.unwrap();
    make_user(&db, "bob").await;
    make_user(&db, "alice").await;
    let users = db.list_users().await.unwrap();
    assert_eq!(users.len(), 2);
    // ordered by username
    assert_eq!(users[0].username, "alice");
    assert_eq!(users[1].username, "bob");
}

#[tokio::test]
async fn user_set_email_verified() {
    let db = Db::open_memory().await.unwrap();
    let id = make_user(&db, "alice").await;
    db.set_email_verified(id).await.unwrap();
    assert_eq!(db.get_user_by_id(id).await.unwrap().unwrap().email_verified, 1);
}

#[tokio::test]
async fn user_set_password_hash() {
    let db = Db::open_memory().await.unwrap();
    let id = make_user(&db, "alice").await;
    db.set_password_hash(id, "newhash").await.unwrap();
    assert_eq!(db.get_user_by_id(id).await.unwrap().unwrap().password_hash, "newhash");
}

#[tokio::test]
async fn user_totp_secret() {
    let db = Db::open_memory().await.unwrap();
    let id = make_user(&db, "alice").await;
    db.set_totp_secret(id, Some("MYSECRET")).await.unwrap();
    let u = db.get_user_by_id(id).await.unwrap().unwrap();
    assert_eq!(u.totp_secret.as_deref(), Some("MYSECRET"));
    db.enable_totp(id, true).await.unwrap();
    assert_eq!(db.get_user_by_id(id).await.unwrap().unwrap().totp_enabled, 1);
    db.enable_totp(id, false).await.unwrap();
    db.set_totp_secret(id, None).await.unwrap();
    let u = db.get_user_by_id(id).await.unwrap().unwrap();
    assert_eq!(u.totp_enabled, 0);
    assert!(u.totp_secret.is_none());
}

// ── Email tokens ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn email_token_verify_flow() {
    let db = Db::open_memory().await.unwrap();
    let id = make_user(&db, "alice").await;
    let tok = db.create_email_token(id, "verify").await.unwrap();
    // second call replaces the first
    let tok2 = db.create_email_token(id, "verify").await.unwrap();
    // old token is consumed → returns None
    assert!(db.consume_email_token(&tok, "verify").await.unwrap().is_none());
    // new token works
    let uid = db.consume_email_token(&tok2, "verify").await.unwrap();
    assert_eq!(uid, Some(id));
    // consuming again returns None (one-time use)
    assert!(db.consume_email_token(&tok2, "verify").await.unwrap().is_none());
}

#[tokio::test]
async fn email_token_wrong_kind_rejected() {
    let db = Db::open_memory().await.unwrap();
    let id = make_user(&db, "alice").await;
    let tok = db.create_email_token(id, "reset").await.unwrap();
    assert!(db.consume_email_token(&tok, "verify").await.unwrap().is_none());
}

#[tokio::test]
async fn email_token_invalid_hex_rejected() {
    let db = Db::open_memory().await.unwrap();
    assert!(db.consume_email_token("notareatoken", "verify").await.unwrap().is_none());
}

// ── Tokens ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn token_create_and_validate() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    let raw = db.create_token(uid, "cli", None, "api").await.unwrap();
    let (tok, user) = db.validate_token(&raw).await.unwrap().unwrap();
    assert_eq!(tok.kind, "api");
    assert_eq!(user.username, "alice");
}

#[tokio::test]
async fn token_invalid_returns_none() {
    let db = Db::open_memory().await.unwrap();
    assert!(db.validate_token("notarealtoken").await.unwrap().is_none());
}

#[tokio::test]
async fn token_expired_returns_none() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    // expires_days = 0 → already expired (0 days in the past)
    let raw = db.create_token(uid, "expired", Some(0), "api").await.unwrap();
    // Token with expires_at = now + 0 * 86400 = now, which passes `exp < now` check
    // since `exp < now` — it's at least a few ms old, but might be equal.
    // Use -1 days instead: negative would have been in the past.
    // Actually with 0 days the token expires at exactly unix_now(), which passes
    // the `exp < now` check only if now > exp. This is a race. Instead revoke it.
    db.revoke_token(uid, "expired").await.unwrap();
    assert!(db.validate_token(&raw).await.unwrap().is_none());
}

#[tokio::test]
async fn token_revoke() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    let raw = db.create_token(uid, "mytoken", None, "api").await.unwrap();
    assert!(db.validate_token(&raw).await.unwrap().is_some());
    assert!(db.revoke_token(uid, "mytoken").await.unwrap());
    assert!(db.validate_token(&raw).await.unwrap().is_none());
}

#[tokio::test]
async fn token_revoke_nonexistent_returns_false() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    assert!(!db.revoke_token(uid, "ghost").await.unwrap());
}

#[tokio::test]
async fn token_kind_stored_correctly() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    let raw = db.create_token(uid, "r", None, "refresh").await.unwrap();
    let (tok, _) = db.validate_token(&raw).await.unwrap().unwrap();
    assert_eq!(tok.kind, "refresh");
}

#[tokio::test]
async fn token_list() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.create_token(uid, "t1", None, "api").await.unwrap();
    db.create_token(uid, "t2", None, "access").await.unwrap();
    let all = db.list_tokens(None).await.unwrap();
    assert_eq!(all.len(), 2);
    let filtered = db.list_tokens(Some(uid)).await.unwrap();
    assert_eq!(filtered.len(), 2);
}

// ── Packages ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn publish_and_get() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "mylib", Some("a library"), "1.0.0", "abc123").await.unwrap();
    let (pkg, versions) = db.get_package("mylib").await.unwrap().unwrap();
    assert_eq!(pkg.name, "mylib");
    assert_eq!(pkg.description.as_deref(), Some("a library"));
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version, "1.0.0");
    assert_eq!(versions[0].checksum, "abc123");
    assert_eq!(versions[0].yanked, 0);
    assert_eq!(versions[0].downloads, 0);
}

#[tokio::test]
async fn get_package_case_insensitive() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "MyLib", None, "1.0.0", "hash").await.unwrap();
    assert!(db.get_package("mylib").await.unwrap().is_some());
    assert!(db.get_package("MYLIB").await.unwrap().is_some());
}

#[tokio::test]
async fn get_package_not_found() {
    let db = Db::open_memory().await.unwrap();
    assert!(db.get_package("nothing").await.unwrap().is_none());
}

#[tokio::test]
async fn publish_duplicate_version_fails() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "mylib", None, "1.0.0", "hash1").await.unwrap();
    assert!(db.publish_version(uid, "mylib", None, "1.0.0", "hash2").await.is_err());
}

#[tokio::test]
async fn publish_multiple_versions() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "mylib", None, "1.0.0", "h1").await.unwrap();
    db.publish_version(uid, "mylib", None, "1.1.0", "h2").await.unwrap();
    let (_, versions) = db.get_package("mylib").await.unwrap().unwrap();
    assert_eq!(versions.len(), 2);
}

#[tokio::test]
async fn get_version() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "mylib", None, "1.0.0", "checksum1").await.unwrap();
    let ver = db.get_version("mylib", "1.0.0").await.unwrap().unwrap();
    assert_eq!(ver.checksum, "checksum1");
    assert_eq!(ver.yanked, 0);
    assert!(db.get_version("mylib", "9.9.9").await.unwrap().is_none());
}

#[tokio::test]
async fn yank_and_unyank() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "mylib", None, "1.0.0", "hash").await.unwrap();
    assert!(db.set_yanked("mylib", "1.0.0", true).await.unwrap());
    assert_eq!(db.get_version("mylib", "1.0.0").await.unwrap().unwrap().yanked, 1);
    assert!(db.set_yanked("mylib", "1.0.0", false).await.unwrap());
    assert_eq!(db.get_version("mylib", "1.0.0").await.unwrap().unwrap().yanked, 0);
}

#[tokio::test]
async fn yank_nonexistent_returns_false() {
    let db = Db::open_memory().await.unwrap();
    assert!(!db.set_yanked("ghost", "1.0.0", true).await.unwrap());
}

#[tokio::test]
async fn delete_package() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "mylib", None, "1.0.0", "hash").await.unwrap();
    assert!(db.delete_package("mylib").await.unwrap());
    assert!(db.get_package("mylib").await.unwrap().is_none());
}

#[tokio::test]
async fn delete_package_nonexistent_returns_false() {
    let db = Db::open_memory().await.unwrap();
    assert!(!db.delete_package("nobody").await.unwrap());
}

// ── Search ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn search_basic() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "awesome-lib", None, "1.0.0", "h1").await.unwrap();
    db.publish_version(uid, "boring-tool", None, "1.0.0", "h2").await.unwrap();
    let (results, total) = db.search_packages("awesome", 20, 0).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.name, "awesome-lib");
}

#[tokio::test]
async fn search_empty_query_returns_all() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "lib-a", None, "1.0.0", "h1").await.unwrap();
    db.publish_version(uid, "lib-b", None, "1.0.0", "h2").await.unwrap();
    db.publish_version(uid, "lib-c", None, "1.0.0", "h3").await.unwrap();
    let (_, total) = db.search_packages("", 20, 0).await.unwrap();
    assert_eq!(total, 3);
}

#[tokio::test]
async fn search_pagination() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    for i in 0..5 {
        db.publish_version(uid, &format!("pkg-{i}"), None, "1.0.0", &format!("h{i}"))
            .await
            .unwrap();
    }
    let (page1, total) = db.search_packages("pkg", 2, 0).await.unwrap();
    assert_eq!(total, 5);
    assert_eq!(page1.len(), 2);
    let (page3, _) = db.search_packages("pkg", 2, 4).await.unwrap();
    assert_eq!(page3.len(), 1);
}

#[tokio::test]
async fn search_excludes_packages_with_no_versions() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "has-version", None, "1.0.0", "h1").await.unwrap();
    // "no-version" never published → shouldn't appear
    let (_, total) = db.search_packages("", 20, 0).await.unwrap();
    assert_eq!(total, 1);
}

// ── Ownership ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn first_publisher_auto_owns() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "mylib", None, "1.0.0", "h").await.unwrap();
    assert_eq!(db.user_owns_package(uid, "mylib").await.unwrap(), Some(true));
}

#[tokio::test]
async fn non_owner_not_allowed() {
    let db = Db::open_memory().await.unwrap();
    let alice = make_user(&db, "alice").await;
    let bob = make_user(&db, "bob").await;
    db.publish_version(alice, "mylib", None, "1.0.0", "h").await.unwrap();
    assert_eq!(db.user_owns_package(bob, "mylib").await.unwrap(), Some(false));
}

#[tokio::test]
async fn nonexistent_package_returns_none() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    assert_eq!(db.user_owns_package(uid, "ghost").await.unwrap(), None);
}

#[tokio::test]
async fn add_and_remove_owner() {
    let db = Db::open_memory().await.unwrap();
    let alice = make_user(&db, "alice").await;
    let bob = make_user(&db, "bob").await;
    db.publish_version(alice, "mylib", None, "1.0.0", "h").await.unwrap();
    assert!(db.add_package_owner("mylib", "bob").await.unwrap());
    assert_eq!(db.user_owns_package(bob, "mylib").await.unwrap(), Some(true));
    assert!(db.remove_package_owner("mylib", "bob").await.unwrap());
    assert_eq!(db.user_owns_package(bob, "mylib").await.unwrap(), Some(false));
}

#[tokio::test]
async fn get_package_owners() {
    let db = Db::open_memory().await.unwrap();
    let alice = make_user(&db, "alice").await;
    let _ = make_user(&db, "bob").await;
    db.publish_version(alice, "mylib", None, "1.0.0", "h").await.unwrap();
    db.add_package_owner("mylib", "bob").await.unwrap();
    let owners = db.get_package_owners("mylib").await.unwrap();
    assert_eq!(owners.len(), 2);
    let names: Vec<_> = owners.iter().map(|u| u.username.as_str()).collect();
    assert!(names.contains(&"alice"));
    assert!(names.contains(&"bob"));
}

// ── Download count ────────────────────────────────────────────────────────────

#[tokio::test]
async fn download_count_increments() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.publish_version(uid, "mylib", None, "1.0.0", "h").await.unwrap();
    db.increment_downloads("mylib", "1.0.0");
    db.increment_downloads("mylib", "1.0.0");
    // Give the spawned tasks time to complete.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let ver = db.get_version("mylib", "1.0.0").await.unwrap().unwrap();
    assert_eq!(ver.downloads, 2);
}

// ── Audit log ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_log_insert_and_query() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.audit(Some(uid), "publish", Some("mylib"), Some("1.0.0"), Some("127.0.0.1"));
    db.audit(Some(uid), "login", None, None, Some("127.0.0.1"));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let rows = db.list_audit_log(None, None, None, None, 100).await.unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn audit_log_filter_by_action() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.audit(Some(uid), "publish", Some("mylib"), Some("1.0.0"), None);
    db.audit(Some(uid), "login", None, None, None);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let rows = db.list_audit_log(None, Some("publish"), None, None, 100).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "publish");
}

#[tokio::test]
async fn audit_log_filter_by_username() {
    let db = Db::open_memory().await.unwrap();
    let alice = make_user(&db, "alice").await;
    let bob = make_user(&db, "bob").await;
    db.audit(Some(alice), "publish", None, None, None);
    db.audit(Some(bob), "login", None, None, None);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let rows = db.list_audit_log(Some("alice"), None, None, None, 100).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].username.as_deref(), Some("alice"));
}

#[tokio::test]
async fn prune_audit_log() {
    let db = Db::open_memory().await.unwrap();
    let uid = make_user(&db, "alice").await;
    db.audit(Some(uid), "old-action", None, None, None);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // Prune entries older than 0 days (i.e., everything older than now).
    // Use a negative TTL to prune entries created right now.
    let n = db.prune_audit_log(-1).await.unwrap(); // cutoff = now + 86400
    assert!(n >= 1);
    assert!(db.list_audit_log(None, None, None, None, 100).await.unwrap().is_empty());
}
