use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::{AnyPool, FromRow, Row as _};

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Rewrite SQL for the Postgres backend:
/// - `?` placeholders → `$1`, `$2`, …
/// - `INSERT OR IGNORE INTO` → `INSERT INTO … ON CONFLICT DO NOTHING`
/// - `ON CONFLICT(name, channel)` → `ON CONFLICT(lower(name), lower(channel))`
///   (packages uses a functional unique index, not a plain UNIQUE constraint)
pub fn pg_sql(sql: &str) -> String {
    let (sql, insert_ignore) = if sql.contains("OR IGNORE ") {
        (std::borrow::Cow::Owned(sql.replace("OR IGNORE ", "")), true)
    } else {
        (std::borrow::Cow::Borrowed(sql), false)
    };
    let mut out = String::with_capacity(sql.len() + 40);
    let mut n = 0usize;
    for ch in sql.chars() {
        if ch == '?' {
            n += 1;
            out.push('$');
            out.push_str(&n.to_string());
        } else {
            out.push(ch);
        }
    }
    if insert_ignore {
        out.push_str(" ON CONFLICT DO NOTHING");
    }
    // The packages table uses a functional index on (lower(name), lower(channel)).
    // ON CONFLICT must reference the exact index expression.
    if out.contains("ON CONFLICT(name, channel)") {
        out = out.replace("ON CONFLICT(name, channel)", "ON CONFLICT(lower(name), lower(channel))");
    }
    out
}


// ── Row types ─────────────────────────────────────────────────────────────────

#[derive(FromRow, Clone)]
pub struct UserRow {
    pub id:             i64,
    pub username:       String,
    pub email:          Option<String>,
    pub password_hash:  String,
    pub is_admin:       i64,
    pub email_verified: i64,
    pub totp_secret:    Option<String>,
    pub totp_enabled:   i64,
}

#[derive(FromRow, Clone)]
pub struct TokenRow {
    pub id:         i64,
    pub user_id:    i64,
    pub name:       String,
    pub kind:       String,
    pub scope:      String,
    pub expires_at: Option<i64>,
    pub last_used:  Option<i64>,
}

#[derive(FromRow)]
pub struct TokenWithUser {
    pub id:         i64,
    pub user_id:    i64,
    pub name:       String,
    pub kind:       String,
    pub scope:      String,
    pub expires_at: Option<i64>,
    pub last_used:  Option<i64>,
    pub username:   String,
}

/// Compare two version strings heuristically: split on `.`, `-`, `_`;
/// compare segments numerically where possible, lexicographically otherwise.
pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let ta: Vec<&str> = a.split(['.', '-', '_']).collect();
    let tb: Vec<&str> = b.split(['.', '-', '_']).collect();
    for (sa, sb) in ta.iter().zip(tb.iter()) {
        let ord = match (sa.parse::<u64>(), sb.parse::<u64>()) {
            (Ok(na), Ok(nb)) => na.cmp(&nb),
            _ => sa.cmp(sb),
        };
        if ord != std::cmp::Ordering::Equal { return ord; }
    }
    ta.len().cmp(&tb.len())
}

/// Pick the highest non-yanked version from a slice. Falls back to first if all yanked.
pub fn best_version<'a>(versions: &'a [VersionRow]) -> Option<&'a str> {
    versions.iter()
        .filter(|v| v.yanked == 0)
        .max_by(|a, b| cmp_version(&a.version, &b.version))
        .or_else(|| versions.first())
        .map(|v| v.version.as_str())
}

#[derive(FromRow)]
pub struct PackageRow {
    pub id:             i64,
    pub name:           String,
    pub channel:        String,
    pub description:    Option<String>,
    pub license:        Option<String>,
    /// Comma-separated keyword list, e.g. `"math,linear-algebra"`. NULL = no keywords.
    pub keywords:       Option<String>,
    pub latest_version: Option<String>,
}

pub const DEFAULT_CHANNEL: &str = "stable";

#[derive(FromRow)]
pub struct VersionRow {
    pub version:      String,
    pub checksum:     String,
    pub yanked:       i64,
    pub downloads:    i64,
    pub dependencies: String,         // JSON object: {"name": "version", ...}
    pub upstream_url: Option<String>, // upstream source archive URL (metadata-only packages)
    pub build_system: Option<String>, // e.g. "cmake", "make", "meson"
    pub supports:     Option<String>, // platform filter expression, e.g. "!uwp & !arm"
}

#[derive(FromRow)]
pub struct AuditRow {
    pub id:         i64,
    pub user_id:    Option<i64>,
    pub action:     String,
    pub package:    Option<String>,
    pub version:    Option<String>,
    pub ip_addr:    Option<String>,
    pub created_at: i64,
    pub username:   Option<String>, // LEFT JOIN users
}

#[derive(FromRow, Clone)]
pub struct PrebuiltRow {
    pub triple:    String,
    pub checksum:  String,
}

#[derive(FromRow, Clone)]
pub struct OrgRow {
    pub id:          i64,
    pub name:        String,
    pub description: Option<String>,
    pub created_at:  i64,
}

#[derive(FromRow, Clone)]
pub struct OrgMemberRow {
    pub user_id:  i64,
    pub username: String,
    pub role:     String,
}

pub struct DbStats {
    pub packages:        i64,
    pub versions:        i64,
    pub users:           i64,
    pub tokens_active:   i64,
    pub downloads_total: i64,
}

// ── Database handle ───────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Db {
    pool:         AnyPool,
    #[allow(dead_code)]
    is_postgres:  bool,
}

impl Db {
    async fn run_sqlite_migrations(pool: &AnyPool) -> Result<()> {
        sqlx::migrate!("./migrations").run(pool).await?;
        Ok(())
    }

    async fn run_pg_migrations(pool: &AnyPool) -> Result<()> {
        sqlx::migrate!("./migrations_pg").run(pool).await?;
        Ok(())
    }

    /// Rewrite SQL for the active backend.
    fn q_sql(&self, sql: &str) -> String {
        if self.is_postgres { pg_sql(sql) } else { sql.to_owned() }
    }

    /// Returns `true` when connected to PostgreSQL.
    pub fn is_postgres(&self) -> bool { self.is_postgres }

    /// Returns a reference to the underlying connection pool.
    /// Used by bulk-import tooling that builds dynamic SQL directly.
    pub fn pool(&self) -> &AnyPool { &self.pool }


    /// Open an in-memory SQLite database. Only for use in tests.
    #[doc(hidden)]
    pub async fn open_memory() -> Result<Self> {
        sqlx::any::install_default_drivers();
        // Single connection: SQLite in-memory DBs are per-connection, so we must
        // keep exactly one to share the same schema between migrations and queries.
        let pool = sqlx::any::AnyPoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::query("PRAGMA foreign_keys = ON").execute(&pool).await?;
        let db = Self { pool, is_postgres: false };
        Self::run_sqlite_migrations(&db.pool).await?;
        Ok(db)
    }

    /// Open a SQLite database at `path` (the default, local-file backend).
    pub async fn open(path: &Path) -> Result<Self> {
        sqlx::any::install_default_drivers();
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = AnyPool::connect(&url).await?;
        sqlx::query("PRAGMA foreign_keys = ON").execute(&pool).await?;
        sqlx::query("PRAGMA journal_mode = WAL").execute(&pool).await?;
        let db = Self { pool, is_postgres: false };
        Self::run_sqlite_migrations(&db.pool).await?;
        Ok(db)
    }

    /// Open a database from an explicit `DATABASE_URL`.
    /// Supports `sqlite://...` and `postgres://...` / `postgresql://...`.
    pub async fn open_url(url: &str) -> Result<Self> {
        sqlx::any::install_default_drivers();
        let is_postgres = url.starts_with("postgres://") || url.starts_with("postgresql://");
        // Ensure SQLite files are opened in read-write-create mode even when
        // the user omits the query param (e.g. bare `sqlite:///path/to/db`).
        let owned;
        let url = if !is_postgres && !url.contains('?') {
            owned = format!("{url}?mode=rwc");
            owned.as_str()
        } else {
            url
        };
        let pool = AnyPool::connect(url).await?;
        if !is_postgres {
            sqlx::query("PRAGMA foreign_keys = ON").execute(&pool).await?;
        }
        let db = Self { pool, is_postgres };
        if is_postgres {
            Self::run_pg_migrations(&db.pool).await?;
        } else {
            Self::run_sqlite_migrations(&db.pool).await?;
        }
        Ok(db)
    }

    // ── Users ──────────────────────────────────────────────────────────────────

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        let row = sqlx::query_as(&self.q_sql("SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE lower(email) = lower(?)"))
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn create_user(
        &self,
        username: &str,
        email: Option<&str>,
        password_hash: &str,
    ) -> Result<i64> {
        let id = sqlx::query_scalar(&self.q_sql("INSERT INTO users (username, email, password_hash) VALUES (?, ?, ?) RETURNING id"))
        .bind(username)
        .bind(email)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<UserRow>> {
        let row = sqlx::query_as(&self.q_sql("SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE lower(username) = lower(?)"))
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_user_by_id(&self, id: i64) -> Result<Option<UserRow>> {
        let row = sqlx::query_as(&self.q_sql("SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE id = ?"))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_users(&self) -> Result<Vec<UserRow>> {
        let rows = sqlx::query_as(&self.q_sql("SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users ORDER BY username"))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── OAuth ──────────────────────────────────────────────────────────────────

    /// Sentinel stored as `password_hash` for OAuth-only accounts.
    ///
    /// This is deliberately not a valid Argon2 PHC string so that the
    /// password-login handler can detect it and return a helpful error instead
    /// of a 500.  The login handler checks `starts_with("!oauth:")`, so all
    /// provider sentinels match that pattern regardless of provider name.
    pub fn oauth_sentinel(provider: &str) -> String {
        format!("!oauth:{provider}")
    }

    /// Look up the freight user linked to a given OAuth identity.
    pub async fn find_oauth_user(
        &self,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        let row = sqlx::query_as(&self.q_sql("SELECT u.id, u.username, u.email, u.password_hash, u.is_admin,
                    u.email_verified, u.totp_secret, u.totp_enabled
             FROM users u
             JOIN oauth_accounts o ON o.user_id = u.id
             WHERE o.provider = ? AND o.provider_id = ?"))
        .bind(provider)
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Record an OAuth identity against an existing user.
    pub async fn link_oauth_account(
        &self,
        user_id:     i64,
        provider:    &str,
        provider_id: &str,
        login:       &str,
    ) -> Result<()> {
        sqlx::query(&self.q_sql("INSERT OR IGNORE INTO oauth_accounts (user_id, provider, provider_id, login)
             VALUES (?, ?, ?, ?)"))
        .bind(user_id)
        .bind(provider)
        .bind(provider_id)
        .bind(login)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Find the freight user for a given OAuth identity, creating one if needed.
    ///
    /// Priority:
    /// 1. Existing `oauth_accounts` row → return that user.
    /// 2. Matching email in `users` → link the OAuth account to that user.
    /// 3. Create a new user with `password_hash = OAUTH_SENTINEL`.
    ///    If `provider_login` is already taken, `_{id}` is appended until a free
    ///    name is found.
    pub async fn find_or_create_oauth_user(
        &self,
        provider:       &str,
        provider_id:    &str,
        provider_login: &str,
        email:          Option<&str>,
    ) -> Result<UserRow> {
        // 1. Already linked?
        if let Some(user) = self.find_oauth_user(provider, provider_id).await? {
            return Ok(user);
        }

        // 2. Match by email if available.
        if let Some(email) = email {
            if let Some(user) = self.get_user_by_email(email).await? {
                self.link_oauth_account(user.id, provider, provider_id, provider_login).await?;
                return Ok(user);
            }
        }

        // 3. Create a new user.
        let mut username = provider_login.to_string();
        // Sanitize: replace characters invalid in freight usernames with '_'.
        username = username
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        // Ensure uniqueness.
        let base = username.clone();
        let mut suffix = 1u32;
        loop {
            if self.get_user_by_username(&username).await?.is_none() {
                break;
            }
            username = format!("{base}_{suffix}");
            suffix += 1;
        }

        let sentinel = Self::oauth_sentinel(provider);
        let user_id = self
            .create_user(&username, email, &sentinel)
            .await?;

        // Mark email as verified since GitHub already verified it.
        if email.is_some() {
            self.set_email_verified(user_id).await?;
        }

        self.link_oauth_account(user_id, provider, provider_id, provider_login).await?;

        self.get_user_by_id(user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("user {user_id} disappeared after insert"))
    }

    pub async fn delete_user(&self, username: &str) -> Result<bool> {
        let n = sqlx::query(&self.q_sql("DELETE FROM users WHERE lower(username) = lower(?)"))
            .bind(username)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n > 0)
    }

    pub async fn set_admin(&self, username: &str, is_admin: bool) -> Result<bool> {
        let n = sqlx::query(&self.q_sql("UPDATE users SET is_admin = ? WHERE lower(username) = lower(?)"))
        .bind(is_admin as i64)
        .bind(username)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n > 0)
    }

    pub async fn set_email_verified(&self, user_id: i64) -> Result<()> {
        sqlx::query(&self.q_sql("UPDATE users SET email_verified = 1 WHERE id = ?"))
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_password_hash(&self, user_id: i64, hash: &str) -> Result<()> {
        sqlx::query(&self.q_sql("UPDATE users SET password_hash = ? WHERE id = ?"))
            .bind(hash)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_totp_secret(&self, user_id: i64, secret: Option<&str>) -> Result<()> {
        sqlx::query(&self.q_sql("UPDATE users SET totp_secret = ? WHERE id = ?"))
            .bind(secret)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn enable_totp(&self, user_id: i64, enabled: bool) -> Result<()> {
        sqlx::query(&self.q_sql("UPDATE users SET totp_enabled = ? WHERE id = ?"))
            .bind(enabled as i64)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Email tokens ───────────────────────────────────────────────────────────

    /// Create an email verification or password-reset token for `user_id`.
    /// `kind` must be `"verify"` or `"reset"`. Returns the raw token (shown once).
    pub async fn create_email_token(&self, user_id: i64, kind: &str) -> Result<String> {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        let expires_at = if kind == "reset" {
            unix_now() + 3_600          // reset tokens valid 1 hour
        } else {
            unix_now() + 24 * 3_600    // verify tokens valid 24 hours
        };
        // Only one pending token per user per kind.
        sqlx::query(&self.q_sql("DELETE FROM email_tokens WHERE user_id = ? AND kind = ?"))
            .bind(user_id)
            .bind(kind)
            .execute(&self.pool)
            .await?;
        sqlx::query(&self.q_sql("INSERT INTO email_tokens (user_id, kind, token_hash, expires_at)
             VALUES (?, ?, ?, ?)"))
        .bind(user_id)
        .bind(kind)
        .bind(&hash)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    /// Validate and consume an email token. Returns `Some(user_id)` on success, `None` if
    /// the token is unknown, wrong kind, or expired.
    pub async fn consume_email_token(&self, token: &str, kind: &str) -> Result<Option<i64>> {
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        let now = unix_now();
        let row: Option<(i64, i64)> = sqlx::query_as(&self.q_sql("SELECT id, user_id FROM email_tokens
             WHERE token_hash = ? AND kind = ? AND expires_at > ?"))
        .bind(&hash)
        .bind(kind)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        let Some((id, user_id)) = row else { return Ok(None) };
        sqlx::query(&self.q_sql("DELETE FROM email_tokens WHERE id = ?"))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(Some(user_id))
    }

    // ── Tokens ─────────────────────────────────────────────────────────────────

    /// Create a new token for `user_id`.
    /// `kind`: `"api"` (CLI-issued), `"access"` (login session), `"refresh"` (refresh token).
    /// `scope`: `"read"`, `"publish"`, or `"admin"`.
    pub async fn create_token(
        &self,
        user_id: i64,
        name: &str,
        expires_days: Option<i64>,
        kind: &str,
        scope: &str,
    ) -> Result<String> {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        let expires_at = expires_days.map(|d| unix_now() + d * 86_400);
        sqlx::query(&self.q_sql("INSERT INTO tokens (user_id, name, kind, scope, token_hash, expires_at)
             VALUES (?, ?, ?, ?, ?, ?)"))
        .bind(user_id)
        .bind(name)
        .bind(kind)
        .bind(scope)
        .bind(&hash)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    /// Validate a raw token string. Returns `None` for unknown or expired tokens.
    /// Updates `last_used` asynchronously (fire-and-forget).
    pub async fn validate_token(&self, token: &str) -> Result<Option<(TokenRow, UserRow)>> {
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        let now = unix_now();

        let tok: Option<TokenRow> = sqlx::query_as(&self.q_sql("SELECT id, user_id, name, kind, scope, expires_at, last_used
             FROM tokens WHERE token_hash = ?"))
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await?;

        let Some(tok) = tok else { return Ok(None) };

        if tok.expires_at.is_some_and(|exp| exp < now) {
            return Ok(None); // expired
        }

        // Update last_used — not critical, fire and forget.
        let pool = self.pool.clone();
        let tid = tok.id;
        let sql_last_used = self.q_sql("UPDATE tokens SET last_used = ? WHERE id = ?");
        tokio::spawn(async move {
            let _ = sqlx::query(&sql_last_used)
                .bind(now)
                .bind(tid)
                .execute(&pool)
                .await;
        });

        let user: UserRow = sqlx::query_as(&self.q_sql("SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE id = ?"))
        .bind(tok.user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Some((tok, user)))
    }

    pub async fn list_tokens(&self, user_id: Option<i64>) -> Result<Vec<TokenWithUser>> {
        if let Some(uid) = user_id {
            sqlx::query_as(&self.q_sql("SELECT t.id, t.user_id, t.name, t.kind, t.scope, t.expires_at, t.last_used, u.username
                 FROM tokens t JOIN users u ON u.id = t.user_id
                 WHERE t.user_id = ? ORDER BY t.created_at"))
            .bind(uid)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
        } else {
            sqlx::query_as(&self.q_sql("SELECT t.id, t.user_id, t.name, t.kind, t.scope, t.expires_at, t.last_used, u.username
                 FROM tokens t JOIN users u ON u.id = t.user_id
                 ORDER BY u.username, t.created_at"))
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
        }
    }

    pub async fn revoke_token(&self, user_id: i64, name: &str) -> Result<bool> {
        let n = sqlx::query(&self.q_sql("DELETE FROM tokens WHERE user_id = ? AND name = ?"))
            .bind(user_id)
            .bind(name)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n > 0)
    }

    // ── Packages ───────────────────────────────────────────────────────────────

    /// Returns `true` if the database is reachable.
    pub async fn ping(&self) -> bool {
        sqlx::query_scalar::<_, i64>(&self.q_sql("SELECT 1"))
            .fetch_one(&self.pool)
            .await
            .is_ok()
    }


    pub async fn get_package(&self, name: &str, channel: &str) -> Result<Option<(PackageRow, Vec<VersionRow>)>> {
        let pkg: Option<PackageRow> = sqlx::query_as(&self.q_sql("SELECT id, name, channel, description, license, keywords, latest_version FROM packages \
             WHERE lower(name) = lower(?) AND channel = ?"))
        .bind(name)
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?;

        let Some(pkg) = pkg else { return Ok(None) };

        let versions: Vec<VersionRow> = sqlx::query_as(&self.q_sql("SELECT version, checksum, yanked, downloads, dependencies,
                    upstream_url, build_system, supports FROM versions
             WHERE package_id = ? ORDER BY created_at DESC"))
        .bind(pkg.id)
        .fetch_all(&self.pool)
        .await?;

        Ok(Some((pkg, versions)))
    }

    /// Fetch a single version row. Used for download checksum verification and yanked check.
    pub async fn get_version(&self, name: &str, version: &str, channel: &str) -> Result<Option<VersionRow>> {
        let row = sqlx::query_as(&self.q_sql("SELECT version, checksum, yanked, downloads, dependencies,
                    upstream_url, build_system, supports FROM versions
             WHERE version = ?
               AND package_id = (SELECT id FROM packages WHERE lower(name) = lower(?) AND channel = ?)"))
        .bind(version)
        .bind(name)
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Increment the download counter for a version. Fire-and-forget — never blocks.
    pub fn increment_downloads(&self, name: &str, version: &str, channel: &str) {
        let pool = self.pool.clone();
        let name = name.to_string();
        let version = version.to_string();
        let channel = channel.to_string();
        let sql_dl = pg_sql(
            "UPDATE versions SET downloads = downloads + 1
             WHERE version = ?
               AND package_id = (SELECT id FROM packages WHERE lower(name) = lower(?) AND channel = ?)",
        );
        tokio::spawn(async move {
            let _ = sqlx::query(&sql_dl)
            .bind(&version)
            .bind(&name)
            .bind(&channel)
            .execute(&pool)
            .await;
        });
    }

    /// Hard-delete a package and all its versions (cascade). Returns `false` if not found.
    pub async fn delete_package(&self, name: &str, channel: &str) -> Result<bool> {
        let n = sqlx::query(&self.q_sql("DELETE FROM packages WHERE lower(name) = lower(?) AND channel = ?"))
            .bind(name)
            .bind(channel)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n > 0)
    }

    pub async fn search_packages(
        &self,
        query: &str,
        channel: &str,
        limit: i64,
        offset: i64,
        sort: &str,
    ) -> Result<(Vec<(PackageRow, Option<VersionRow>)>, i64)> {
        let pattern = format!("%{query}%");

        let total: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM packages
             WHERE (lower(name) LIKE lower(?) OR lower(keywords) LIKE lower(?)) AND channel = ?
               AND EXISTS (SELECT 1 FROM versions WHERE package_id = id)"))
        .bind(&pattern)
        .bind(&pattern)
        .bind(channel)
        .fetch_one(&self.pool)
        .await?;

        // Join against the version pinned by latest_version (set via cmp_version on publish/import).
        // Falls back to most-recently-inserted version for packages not yet updated.
        let order = match sort {
            "downloads" => "v.downloads DESC, p.name",
            "newest"    => "p.id DESC",
            _           => "p.name",
        };
        let sql = format!(
            "SELECT p.id, p.name, p.channel, p.description, p.license, p.keywords, p.latest_version,
                    v.version      AS v_version,     v.checksum     AS v_checksum,
                    v.yanked       AS v_yanked,       v.downloads    AS v_downloads,
                    v.dependencies AS v_deps,         v.upstream_url AS v_upstream_url,
                    v.build_system AS v_build_system, v.supports     AS v_supports
             FROM packages p
             LEFT JOIN versions v ON v.package_id = p.id
               AND v.version = COALESCE(p.latest_version,
                     (SELECT version FROM versions WHERE package_id = p.id AND yanked = 0
                      ORDER BY created_at DESC LIMIT 1))
               AND v.yanked = 0
             WHERE (lower(p.name) LIKE lower(?) OR lower(p.keywords) LIKE lower(?)) AND p.channel = ?
               AND EXISTS (SELECT 1 FROM versions WHERE package_id = p.id)
             ORDER BY {order} LIMIT ? OFFSET ?"
        );
        let rows = sqlx::query(&self.q_sql(&sql))
        .bind(&pattern)
        .bind(&pattern)
        .bind(channel)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::with_capacity(rows.len());
        for row in &rows {
            let pkg = PackageRow {
                id:             row.try_get("id")?,
                name:           row.try_get("name")?,
                channel:        row.try_get("channel")?,
                description:    row.try_get("description")?,
                license:        row.try_get("license")?,
                keywords:       row.try_get("keywords")?,
                latest_version: row.try_get("latest_version").unwrap_or_default(),
            };
            let ver = match row.try_get::<String, _>("v_version") {
                Ok(version) => Some(VersionRow {
                    version,
                    checksum:     row.try_get("v_checksum").unwrap_or_default(),
                    yanked:       row.try_get("v_yanked").unwrap_or(0),
                    downloads:    row.try_get("v_downloads").unwrap_or(0),
                    dependencies: row.try_get("v_deps").unwrap_or_default(),
                    upstream_url: row.try_get("v_upstream_url").unwrap_or_default(),
                    build_system: row.try_get("v_build_system").unwrap_or_default(),
                    supports:     row.try_get("v_supports").unwrap_or_default(),
                }),
                Err(_) => None,
            };
            results.push((pkg, ver));
        }
        Ok((results, total))
    }

    /// Publish a new version. Grants ownership to `user_id` if the package is new.
    ///
    /// `upstream_url` — when set, marks this as a "metadata-only" entry: no tarball is
    /// stored on the server and `/download` returns a 302 redirect to this URL.
    /// `build_system` — the foreign build system needed to compile the package (e.g. "cmake").
    /// `supports`     — platform filter expression (e.g. "!uwp & !arm").
    pub async fn publish_version(
        &self,
        user_id: i64,
        name: &str,
        channel: &str,
        description: Option<&str>,
        license: Option<&str>,
        keywords: Option<&str>,
        version: &str,
        checksum: &str,
        dependencies: &str,
        upstream_url: Option<&str>,
        build_system: Option<&str>,
        supports: Option<&str>,
    ) -> Result<()> {
        sqlx::query(&self.q_sql("INSERT INTO packages (name, channel, description, license, keywords) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(name, channel) DO UPDATE SET
               description = COALESCE(excluded.description, packages.description),
               license     = COALESCE(excluded.license,     packages.license),
               keywords    = COALESCE(excluded.keywords,    packages.keywords)"))
        .bind(name)
        .bind(channel)
        .bind(description)
        .bind(license)
        .bind(keywords)
        .execute(&self.pool)
        .await?;

        let pkg: PackageRow = sqlx::query_as(&self.q_sql("SELECT id, name, channel, description, license, keywords, latest_version FROM packages WHERE lower(name) = lower(?) AND channel = ?"))
        .bind(name)
        .bind(channel)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(&self.q_sql("INSERT INTO versions (package_id, version, checksum, dependencies, upstream_url, build_system, supports)
             VALUES (?, ?, ?, ?, ?, ?, ?)"))
        .bind(pkg.id)
        .bind(version)
        .bind(checksum)
        .bind(dependencies)
        .bind(upstream_url)
        .bind(build_system)
        .bind(supports)
        .execute(&self.pool)
        .await?;

        // Maintain latest_version using semantic ordering.
        let cur_latest: Option<String> = sqlx::query_scalar(
            &self.q_sql("SELECT latest_version FROM packages WHERE id = ?"))
            .bind(pkg.id)
            .fetch_optional(&self.pool)
            .await?
            .flatten();
        let is_newer = cur_latest.as_deref()
            .map(|cur| cmp_version(version, cur) == std::cmp::Ordering::Greater)
            .unwrap_or(true);
        if is_newer {
            sqlx::query(&self.q_sql("UPDATE packages SET latest_version = ? WHERE id = ?"))
                .bind(version)
                .bind(pkg.id)
                .execute(&self.pool)
                .await?;
        }

        // Auto-grant ownership if the package has no owners yet.
        let owner_count: i64 =
            sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM package_owners WHERE package_id = ?"))
                .bind(pkg.id)
                .fetch_one(&self.pool)
                .await?;
        if owner_count == 0 {
            sqlx::query(&self.q_sql("INSERT OR IGNORE INTO package_owners (package_id, user_id) VALUES (?, ?)"))
            .bind(pkg.id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Return the top `limit` keywords by package count for the given channel.
    pub async fn keywords_top(&self, channel: &str, limit: i64) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(&self.q_sql(
            "SELECT keywords FROM packages WHERE channel = ? AND keywords IS NOT NULL AND keywords != ''"))
            .bind(channel)
            .fetch_all(&self.pool)
            .await?;

        let mut counts: HashMap<String, i64> = HashMap::new();
        for row in rows {
            let kws: String = row.try_get("keywords").unwrap_or_default();
            for kw in kws.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                *counts.entry(kw.to_lowercase()).or_insert(0) += 1;
            }
        }

        let mut sorted: Vec<(String, i64)> = counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        sorted.truncate(limit as usize);
        Ok(sorted)
    }

    /// For each term in `terms`, count how many packages in `channel` match it
    /// (by package name or keyword list).  Returns only terms with count > 0,
    /// sorted descending by count.  Used as a fallback when no keyword metadata exists.
    pub async fn keywords_count_terms(&self, channel: &str, terms: &[&str]) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(&self.q_sql(
            "SELECT name, keywords FROM packages WHERE channel = ?
               AND EXISTS (SELECT 1 FROM versions WHERE package_id = id)"))
            .bind(channel)
            .fetch_all(&self.pool)
            .await?;

        let mut counts: HashMap<&str, i64> = HashMap::new();
        for row in &rows {
            let name: String = row.try_get("name").unwrap_or_default();
            let kws:  String = row.try_get("keywords").unwrap_or_default();
            let name_lc = name.to_lowercase();
            let kws_lc  = kws.to_lowercase();
            for &term in terms {
                if name_lc.contains(term) || kws_lc.contains(term) {
                    *counts.entry(term).or_insert(0) += 1;
                }
            }
        }

        let mut result: Vec<(String, i64)> = counts
            .into_iter()
            .filter(|(_, c)| *c > 0)
            .map(|(t, c)| (t.to_string(), c))
            .collect();
        result.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(result)
    }

    /// Recompute `latest_version` for every package using semantic `cmp_version` ordering.
    /// Call this after bulk imports to ensure consistent "latest" across the registry.
    pub async fn update_all_latest_versions(&self) -> Result<u64> {
        let rows = sqlx::query(
            &self.q_sql("SELECT package_id, version FROM versions WHERE yanked = 0 ORDER BY package_id"))
            .fetch_all(&self.pool)
            .await?;

        let mut by_pkg: HashMap<i64, Vec<String>> = HashMap::new();
        for row in rows {
            let pkg_id: i64   = row.try_get("package_id")?;
            let ver:    String = row.try_get("version")?;
            by_pkg.entry(pkg_id).or_default().push(ver);
        }

        let mut updated = 0u64;
        for (pkg_id, versions) in by_pkg {
            let best = versions.iter()
                .max_by(|a, b| cmp_version(a, b))
                .cloned();
            if let Some(best) = best {
                let affected = sqlx::query(
                    &self.q_sql("UPDATE packages SET latest_version = ? WHERE id = ?"))
                    .bind(&best)
                    .bind(pkg_id)
                    .execute(&self.pool)
                    .await?
                    .rows_affected();
                updated += affected;
            }
        }
        Ok(updated)
    }

    pub async fn set_yanked(&self, name: &str, version: &str, channel: &str, yanked: bool) -> Result<bool> {
        let n = sqlx::query(&self.q_sql("UPDATE versions SET yanked = ?
             WHERE version = ?
               AND package_id = (SELECT id FROM packages WHERE lower(name) = lower(?) AND channel = ?)"))
        .bind(yanked as i64)
        .bind(version)
        .bind(name)
        .bind(channel)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n > 0)
    }

    // ── Ownership ──────────────────────────────────────────────────────────────

    /// Count the number of distinct packages owned by `user_id` across all channels.
    pub async fn count_owned_packages(&self, user_id: i64) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM package_owners WHERE user_id = ?"))
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// Returns `None` when the package doesn't exist, `Some(bool)` otherwise.
    pub async fn user_owns_package(
        &self,
        user_id: i64,
        package_name: &str,
        channel: &str,
    ) -> Result<Option<bool>> {
        let pkg: Option<PackageRow> = sqlx::query_as(&self.q_sql("SELECT id, name, channel, description, license, keywords, latest_version FROM packages WHERE lower(name) = lower(?) AND channel = ?"))
        .bind(package_name)
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?;

        let Some(pkg) = pkg else { return Ok(None) };

        let owns: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM package_owners WHERE package_id = ? AND user_id = ?"))
        .bind(pkg.id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Some(owns > 0))
    }

    pub async fn get_package_owners(&self, package_name: &str, channel: &str) -> Result<Vec<UserRow>> {
        let rows = sqlx::query_as(&self.q_sql("SELECT u.id, u.username, u.email, u.password_hash, u.is_admin,
                    u.email_verified, u.totp_secret, u.totp_enabled
             FROM users u
             JOIN package_owners po ON po.user_id = u.id
             JOIN packages p        ON p.id = po.package_id
             WHERE lower(p.name) = lower(?) AND p.channel = ?
             ORDER BY u.username"))
        .bind(package_name)
        .bind(channel)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_package_owner(&self, package_name: &str, channel: &str, username: &str) -> Result<bool> {
        let pkg: Option<PackageRow> = sqlx::query_as(&self.q_sql("SELECT id, name, channel, description, license, keywords, latest_version FROM packages WHERE lower(name) = lower(?) AND channel = ?"))
        .bind(package_name)
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?;
        let Some(pkg) = pkg else { return Ok(false) };

        let user: Option<UserRow> = sqlx::query_as(&self.q_sql("SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE lower(username) = lower(?)"))
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        let Some(user) = user else { return Ok(false) };

        sqlx::query(&self.q_sql("INSERT OR IGNORE INTO package_owners (package_id, user_id) VALUES (?, ?)"))
        .bind(pkg.id)
        .bind(user.id)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    pub async fn remove_package_owner(&self, package_name: &str, channel: &str, username: &str) -> Result<bool> {
        let n = sqlx::query(&self.q_sql("DELETE FROM package_owners
             WHERE package_id = (SELECT id FROM packages WHERE lower(name) = lower(?) AND channel = ?)
               AND user_id    = (SELECT id FROM users    WHERE lower(username) = lower(?))"))
        .bind(package_name)
        .bind(channel)
        .bind(username)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n > 0)
    }

    // ── Audit log ──────────────────────────────────────────────────────────────

    /// Query audit log entries with optional filters. `limit` is clamped to 500.
    pub async fn list_audit_log(
        &self,
        username: Option<&str>,
        action:   Option<&str>,
        since:    Option<i64>,
        until:    Option<i64>,
        limit:    i64,
    ) -> Result<Vec<AuditRow>> {
        let rows = sqlx::query_as(&self.q_sql("SELECT a.id, a.user_id, a.action, a.package, a.version,
                    a.ip_addr, a.created_at, u.username
             FROM audit_log a
             LEFT JOIN users u ON u.id = a.user_id
             WHERE (? IS NULL OR lower(u.username) = lower(?))
               AND (? IS NULL OR a.action = ?)
               AND a.created_at >= COALESCE(?, -9223372036854775808)
               AND a.created_at <= COALESCE(?,  9223372036854775807)
             ORDER BY a.created_at DESC
             LIMIT ?"))
        .bind(username).bind(username)
        .bind(action).bind(action)
        .bind(since)
        .bind(until)
        .bind(limit.min(500))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Append an audit entry. Silently drops failures — never blocks a request.
    pub fn audit(
        &self,
        user_id: Option<i64>,
        action: &str,
        package: Option<&str>,
        version: Option<&str>,
        ip: Option<&str>,
    ) {
        let pool = self.pool.clone();
        let action = action.to_string();
        let package = package.map(str::to_string);
        let version = version.map(str::to_string);
        let ip = ip.map(str::to_string);
        let sql_audit = pg_sql(
            "INSERT INTO audit_log (user_id, action, package, version, ip_addr)
             VALUES (?, ?, ?, ?, ?)",
        );
        tokio::spawn(async move {
            let _ = sqlx::query(&sql_audit)
            .bind(user_id)
            .bind(&action)
            .bind(package.as_deref())
            .bind(version.as_deref())
            .bind(ip.as_deref())
            .execute(&pool)
            .await;
        });
    }

    /// Delete audit log entries older than `ttl_days`. Returns the number of rows deleted.
    pub async fn prune_audit_log(&self, ttl_days: i64) -> Result<u64> {
        let cutoff = unix_now() - ttl_days * 86_400;
        let n = sqlx::query(&self.q_sql("DELETE FROM audit_log WHERE created_at < ?"))
            .bind(cutoff)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n)
    }

    // ── Prebuilts ─────────────────────────────────────────────────────────────

    /// Record a prebuilt tarball for (name, channel, version, triple).
    pub async fn store_prebuilt(
        &self,
        name:     &str,
        channel:  &str,
        version:  &str,
        triple:   &str,
        checksum: &str,
    ) -> Result<()> {
        sqlx::query(&self.q_sql("INSERT INTO prebuilts (name, channel, version, triple, checksum)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(name, channel, version, triple) DO UPDATE SET checksum = excluded.checksum"))
        .bind(name).bind(channel).bind(version).bind(triple).bind(checksum)
        .execute(&self.pool).await?;
        Ok(())
    }

    /// Fetch metadata for a specific prebuilt triple, or `None` if not found.
    pub async fn get_prebuilt(
        &self,
        name:    &str,
        channel: &str,
        version: &str,
        triple:  &str,
    ) -> Result<Option<PrebuiltRow>> {
        Ok(sqlx::query_as::<_, PrebuiltRow>(&self.q_sql("SELECT triple, checksum FROM prebuilts
             WHERE name = ? AND channel = ? AND version = ? AND triple = ?"))
        .bind(name).bind(channel).bind(version).bind(triple)
        .fetch_optional(&self.pool).await?)
    }

    /// List all prebuilt triples available for a given (name, channel, version).
    pub async fn list_prebuilts(
        &self,
        name:    &str,
        channel: &str,
        version: &str,
    ) -> Result<Vec<PrebuiltRow>> {
        Ok(sqlx::query_as::<_, PrebuiltRow>(&self.q_sql("SELECT triple, checksum FROM prebuilts
             WHERE name = ? AND channel = ? AND version = ?
             ORDER BY triple"))
        .bind(name).bind(channel).bind(version)
        .fetch_all(&self.pool).await?)
    }

    /// Delete all prebuilts for a package (used when the package itself is deleted).
    pub async fn delete_package_prebuilts(&self, name: &str, channel: &str) -> Result<()> {
        sqlx::query(&self.q_sql("DELETE FROM prebuilts WHERE name = ? AND channel = ?"))
            .bind(name).bind(channel)
            .execute(&self.pool).await?;
        Ok(())
    }

    // ── Metrics ────────────────────────────────────────────────────────────────

    pub async fn stats(&self) -> Result<DbStats> {
        let packages: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM packages"))
            .fetch_one(&self.pool).await?;
        let versions: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM versions"))
            .fetch_one(&self.pool).await?;
        let users: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM users"))
            .fetch_one(&self.pool).await?;
        let now = unix_now();
        let tokens_active: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM tokens WHERE expires_at IS NULL OR expires_at > ?"))
        .bind(now)
        .fetch_one(&self.pool).await?;
        let downloads_total: i64 = sqlx::query_scalar(&self.q_sql("SELECT COALESCE(SUM(downloads), 0) FROM versions"))
        .fetch_one(&self.pool).await?;
        Ok(DbStats { packages, versions, users, tokens_active, downloads_total })
    }

    // ── Organizations ──────────────────────────────────────────────────────────

    pub async fn create_org(&self, name: &str, description: Option<&str>, owner_id: i64) -> Result<i64> {
        let org_id: i64 = sqlx::query_scalar(&self.q_sql("INSERT INTO organizations (name, description) VALUES (?, ?) RETURNING id"))
        .bind(name)
        .bind(description)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(&self.q_sql("INSERT INTO org_members (org_id, user_id, role) VALUES (?, ?, 'owner')"))
            .bind(org_id)
            .bind(owner_id)
            .execute(&self.pool)
            .await?;

        Ok(org_id)
    }

    pub async fn get_org(&self, name: &str) -> Result<Option<OrgRow>> {
        let row = sqlx::query_as(&self.q_sql("SELECT id, name, description, created_at FROM organizations WHERE lower(name) = lower(?)"))
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_orgs(&self) -> Result<Vec<OrgRow>> {
        let rows = sqlx::query_as(&self.q_sql("SELECT id, name, description, created_at FROM organizations ORDER BY name"))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn delete_org(&self, name: &str) -> Result<bool> {
        let result = sqlx::query(&self.q_sql("DELETE FROM organizations WHERE lower(name) = lower(?)"))
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_org_members(&self, org_name: &str) -> Result<Vec<OrgMemberRow>> {
        let rows = sqlx::query_as(&self.q_sql("SELECT m.user_id, u.username, m.role
             FROM org_members m
             JOIN organizations o ON o.id = m.org_id
             JOIN users u ON u.id = m.user_id
             WHERE lower(o.name) = lower(?)
             ORDER BY m.role DESC, u.username"))
        .bind(org_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_org_member(&self, org_name: &str, username: &str, role: &str) -> Result<bool> {
        let org = match self.get_org(org_name).await? {
            Some(o) => o,
            None => return Ok(false),
        };
        let user = match self.get_user_by_username(username).await? {
            Some(u) => u,
            None => return Ok(false),
        };
        sqlx::query(&self.q_sql("INSERT INTO org_members (org_id, user_id, role) VALUES (?, ?, ?)
             ON CONFLICT(org_id, user_id) DO UPDATE SET role = excluded.role"))
        .bind(org.id)
        .bind(user.id)
        .bind(role)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    pub async fn remove_org_member(&self, org_name: &str, username: &str) -> Result<bool> {
        let result = sqlx::query(&self.q_sql("DELETE FROM org_members
             WHERE org_id = (SELECT id FROM organizations WHERE lower(name) = lower(?))
               AND user_id = (SELECT id FROM users WHERE lower(username) = lower(?))"))
        .bind(org_name)
        .bind(username)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn is_org_owner(&self, org_name: &str, user_id: i64) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM org_members m
             JOIN organizations o ON o.id = m.org_id
             WHERE lower(o.name) = lower(?) AND m.user_id = ? AND m.role = 'owner'"))
        .bind(org_name)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn is_org_member(&self, org_name: &str, user_id: i64) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM org_members m
             JOIN organizations o ON o.id = m.org_id
             WHERE lower(o.name) = lower(?) AND m.user_id = ?"))
        .bind(org_name)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn set_package_org(&self, package_name: &str, channel: &str, org_name: Option<&str>) -> Result<bool> {
        let org_id: Option<i64> = if let Some(name) = org_name {
            match self.get_org(name).await? {
                Some(o) => Some(o.id),
                None => return Ok(false),
            }
        } else {
            None
        };
        let result = sqlx::query(&self.q_sql("UPDATE packages SET org_id = ? WHERE lower(name) = lower(?) AND lower(channel) = lower(?)"))
        .bind(org_id)
        .bind(package_name)
        .bind(channel)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Check if a user can publish/modify a package (owns it directly or via org membership).
    pub async fn user_can_manage_package(&self, package_name: &str, channel: &str, user_id: i64) -> Result<bool> {
        // Direct ownership check (existing table).
        if self.user_owns_package(user_id, package_name, channel).await? == Some(true) {
            return Ok(true);
        }
        // Org membership check.
        let count: i64 = sqlx::query_scalar(&self.q_sql("SELECT COUNT(*) FROM packages p
             JOIN org_members m ON m.org_id = p.org_id
             WHERE lower(p.name) = lower(?) AND lower(p.channel) = lower(?) AND m.user_id = ?"))
        .bind(package_name)
        .bind(channel)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }
}
