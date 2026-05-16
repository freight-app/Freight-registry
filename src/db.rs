use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::{AnyPool, FromRow};

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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

#[derive(FromRow)]
pub struct PackageRow {
    pub id:          i64,
    pub name:        String,
    pub channel:     String,
    pub description: Option<String>,
}

pub const DEFAULT_CHANNEL: &str = "stable";

#[derive(FromRow)]
pub struct VersionRow {
    pub version:   String,
    pub checksum:  String,
    pub yanked:    i64,
    pub downloads: i64,
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
    pool:        AnyPool,
    is_postgres: bool,
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

    pub async fn create_user(
        &self,
        username: &str,
        email: Option<&str>,
        password_hash: &str,
    ) -> Result<i64> {
        let id = sqlx::query_scalar(
            "INSERT INTO users (username, email, password_hash) VALUES (?, ?, ?) RETURNING id",
        )
        .bind(username)
        .bind(email)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<UserRow>> {
        let row = sqlx::query_as(
            "SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE lower(username) = lower(?)",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_user_by_id(&self, id: i64) -> Result<Option<UserRow>> {
        let row = sqlx::query_as(
            "SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_users(&self) -> Result<Vec<UserRow>> {
        let rows = sqlx::query_as(
            "SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn delete_user(&self, username: &str) -> Result<bool> {
        let n = sqlx::query("DELETE FROM users WHERE lower(username) = lower(?)")
            .bind(username)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n > 0)
    }

    pub async fn set_admin(&self, username: &str, is_admin: bool) -> Result<bool> {
        let n = sqlx::query(
            "UPDATE users SET is_admin = ? WHERE lower(username) = lower(?)",
        )
        .bind(is_admin as i64)
        .bind(username)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n > 0)
    }

    pub async fn set_email_verified(&self, user_id: i64) -> Result<()> {
        sqlx::query("UPDATE users SET email_verified = 1 WHERE id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_password_hash(&self, user_id: i64, hash: &str) -> Result<()> {
        sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(hash)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_totp_secret(&self, user_id: i64, secret: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE users SET totp_secret = ? WHERE id = ?")
            .bind(secret)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn enable_totp(&self, user_id: i64, enabled: bool) -> Result<()> {
        sqlx::query("UPDATE users SET totp_enabled = ? WHERE id = ?")
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
        sqlx::query("DELETE FROM email_tokens WHERE user_id = ? AND kind = ?")
            .bind(user_id)
            .bind(kind)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO email_tokens (user_id, kind, token_hash, expires_at)
             VALUES (?, ?, ?, ?)",
        )
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
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT id, user_id FROM email_tokens
             WHERE token_hash = ? AND kind = ? AND expires_at > ?",
        )
        .bind(&hash)
        .bind(kind)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        let Some((id, user_id)) = row else { return Ok(None) };
        sqlx::query("DELETE FROM email_tokens WHERE id = ?")
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
        sqlx::query(
            "INSERT INTO tokens (user_id, name, kind, scope, token_hash, expires_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
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

        let tok: Option<TokenRow> = sqlx::query_as(
            "SELECT id, user_id, name, kind, scope, expires_at, last_used
             FROM tokens WHERE token_hash = ?",
        )
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
        tokio::spawn(async move {
            let _ = sqlx::query("UPDATE tokens SET last_used = ? WHERE id = ?")
                .bind(now)
                .bind(tid)
                .execute(&pool)
                .await;
        });

        let user: UserRow = sqlx::query_as(
            "SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE id = ?",
        )
        .bind(tok.user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Some((tok, user)))
    }

    pub async fn list_tokens(&self, user_id: Option<i64>) -> Result<Vec<TokenWithUser>> {
        if let Some(uid) = user_id {
            sqlx::query_as(
                "SELECT t.id, t.user_id, t.name, t.kind, t.scope, t.expires_at, t.last_used, u.username
                 FROM tokens t JOIN users u ON u.id = t.user_id
                 WHERE t.user_id = ? ORDER BY t.created_at",
            )
            .bind(uid)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
        } else {
            sqlx::query_as(
                "SELECT t.id, t.user_id, t.name, t.kind, t.scope, t.expires_at, t.last_used, u.username
                 FROM tokens t JOIN users u ON u.id = t.user_id
                 ORDER BY u.username, t.created_at",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
        }
    }

    pub async fn revoke_token(&self, user_id: i64, name: &str) -> Result<bool> {
        let n = sqlx::query("DELETE FROM tokens WHERE user_id = ? AND name = ?")
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
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .is_ok()
    }

    pub async fn get_package(&self, name: &str, channel: &str) -> Result<Option<(PackageRow, Vec<VersionRow>)>> {
        let pkg: Option<PackageRow> = sqlx::query_as(
            "SELECT id, name, channel, description FROM packages \
             WHERE lower(name) = lower(?) AND channel = ?",
        )
        .bind(name)
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?;

        let Some(pkg) = pkg else { return Ok(None) };

        let versions: Vec<VersionRow> = sqlx::query_as(
            "SELECT version, checksum, yanked, downloads FROM versions
             WHERE package_id = ? ORDER BY created_at DESC",
        )
        .bind(pkg.id)
        .fetch_all(&self.pool)
        .await?;

        Ok(Some((pkg, versions)))
    }

    /// Fetch a single version row. Used for download checksum verification and yanked check.
    pub async fn get_version(&self, name: &str, version: &str, channel: &str) -> Result<Option<VersionRow>> {
        let row = sqlx::query_as(
            "SELECT version, checksum, yanked, downloads FROM versions
             WHERE version = ?
               AND package_id = (SELECT id FROM packages WHERE lower(name) = lower(?) AND channel = ?)",
        )
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
        tokio::spawn(async move {
            let _ = sqlx::query(
                "UPDATE versions SET downloads = downloads + 1
                 WHERE version = ?
                   AND package_id = (SELECT id FROM packages WHERE lower(name) = lower(?) AND channel = ?)",
            )
            .bind(&version)
            .bind(&name)
            .bind(&channel)
            .execute(&pool)
            .await;
        });
    }

    /// Hard-delete a package and all its versions (cascade). Returns `false` if not found.
    pub async fn delete_package(&self, name: &str, channel: &str) -> Result<bool> {
        let n = sqlx::query("DELETE FROM packages WHERE lower(name) = lower(?) AND channel = ?")
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
    ) -> Result<(Vec<(PackageRow, Option<VersionRow>)>, i64)> {
        let pattern = format!("%{query}%");

        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM packages
             WHERE lower(name) LIKE lower(?) AND channel = ?
               AND EXISTS (SELECT 1 FROM versions WHERE package_id = id)",
        )
        .bind(&pattern)
        .bind(channel)
        .fetch_one(&self.pool)
        .await?;

        let pkgs: Vec<PackageRow> = sqlx::query_as(
            "SELECT id, name, channel, description FROM packages
             WHERE lower(name) LIKE lower(?) AND channel = ?
               AND EXISTS (SELECT 1 FROM versions WHERE package_id = id)
             ORDER BY name LIMIT ? OFFSET ?",
        )
        .bind(&pattern)
        .bind(channel)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::with_capacity(pkgs.len());
        for pkg in pkgs {
            let latest: Option<VersionRow> = sqlx::query_as(
                "SELECT version, checksum, yanked, downloads FROM versions
                 WHERE package_id = ? AND yanked = 0 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(pkg.id)
            .fetch_optional(&self.pool)
            .await?;
            results.push((pkg, latest));
        }
        Ok((results, total))
    }

    /// Publish a new version. Grants ownership to `user_id` if the package is new.
    pub async fn publish_version(
        &self,
        user_id: i64,
        name: &str,
        channel: &str,
        description: Option<&str>,
        version: &str,
        checksum: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO packages (name, channel, description) VALUES (?, ?, ?)
             ON CONFLICT(name, channel) DO UPDATE SET description = COALESCE(excluded.description, description)",
        )
        .bind(name)
        .bind(channel)
        .bind(description)
        .execute(&self.pool)
        .await?;

        let pkg: PackageRow = sqlx::query_as(
            "SELECT id, name, channel, description FROM packages WHERE lower(name) = lower(?) AND channel = ?",
        )
        .bind(name)
        .bind(channel)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(
            "INSERT INTO versions (package_id, version, checksum) VALUES (?, ?, ?)",
        )
        .bind(pkg.id)
        .bind(version)
        .bind(checksum)
        .execute(&self.pool)
        .await?;

        // Auto-grant ownership if the package has no owners yet.
        let owner_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM package_owners WHERE package_id = ?")
                .bind(pkg.id)
                .fetch_one(&self.pool)
                .await?;
        if owner_count == 0 {
            sqlx::query(
                "INSERT OR IGNORE INTO package_owners (package_id, user_id) VALUES (?, ?)",
            )
            .bind(pkg.id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    pub async fn set_yanked(&self, name: &str, version: &str, channel: &str, yanked: bool) -> Result<bool> {
        let n = sqlx::query(
            "UPDATE versions SET yanked = ?
             WHERE version = ?
               AND package_id = (SELECT id FROM packages WHERE lower(name) = lower(?) AND channel = ?)",
        )
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

    /// Returns `None` when the package doesn't exist, `Some(bool)` otherwise.
    pub async fn user_owns_package(
        &self,
        user_id: i64,
        package_name: &str,
        channel: &str,
    ) -> Result<Option<bool>> {
        let pkg: Option<PackageRow> = sqlx::query_as(
            "SELECT id, name, channel, description FROM packages WHERE lower(name) = lower(?) AND channel = ?",
        )
        .bind(package_name)
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?;

        let Some(pkg) = pkg else { return Ok(None) };

        let owns: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM package_owners WHERE package_id = ? AND user_id = ?",
        )
        .bind(pkg.id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Some(owns > 0))
    }

    pub async fn get_package_owners(&self, package_name: &str, channel: &str) -> Result<Vec<UserRow>> {
        let rows = sqlx::query_as(
            "SELECT u.id, u.username, u.email, u.password_hash, u.is_admin,
                    u.email_verified, u.totp_secret, u.totp_enabled
             FROM users u
             JOIN package_owners po ON po.user_id = u.id
             JOIN packages p        ON p.id = po.package_id
             WHERE lower(p.name) = lower(?) AND p.channel = ?
             ORDER BY u.username",
        )
        .bind(package_name)
        .bind(channel)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_package_owner(&self, package_name: &str, channel: &str, username: &str) -> Result<bool> {
        let pkg: Option<PackageRow> = sqlx::query_as(
            "SELECT id, name, channel, description FROM packages WHERE lower(name) = lower(?) AND channel = ?",
        )
        .bind(package_name)
        .bind(channel)
        .fetch_optional(&self.pool)
        .await?;
        let Some(pkg) = pkg else { return Ok(false) };

        let user: Option<UserRow> = sqlx::query_as(
            "SELECT id, username, email, password_hash, is_admin,
                    email_verified, totp_secret, totp_enabled
             FROM users WHERE lower(username) = lower(?)",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        let Some(user) = user else { return Ok(false) };

        sqlx::query(
            "INSERT OR IGNORE INTO package_owners (package_id, user_id) VALUES (?, ?)",
        )
        .bind(pkg.id)
        .bind(user.id)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    pub async fn remove_package_owner(&self, package_name: &str, channel: &str, username: &str) -> Result<bool> {
        let n = sqlx::query(
            "DELETE FROM package_owners
             WHERE package_id = (SELECT id FROM packages WHERE lower(name) = lower(?) AND channel = ?)
               AND user_id    = (SELECT id FROM users    WHERE lower(username) = lower(?))",
        )
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
        let rows = sqlx::query_as(
            "SELECT a.id, a.user_id, a.action, a.package, a.version,
                    a.ip_addr, a.created_at, u.username
             FROM audit_log a
             LEFT JOIN users u ON u.id = a.user_id
             WHERE (? IS NULL OR lower(u.username) = lower(?))
               AND (? IS NULL OR a.action = ?)
               AND a.created_at >= COALESCE(?, -9223372036854775808)
               AND a.created_at <= COALESCE(?,  9223372036854775807)
             ORDER BY a.created_at DESC
             LIMIT ?",
        )
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
        tokio::spawn(async move {
            let _ = sqlx::query(
                "INSERT INTO audit_log (user_id, action, package, version, ip_addr)
                 VALUES (?, ?, ?, ?, ?)",
            )
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
        let n = sqlx::query("DELETE FROM audit_log WHERE created_at < ?")
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
        sqlx::query(
            "INSERT INTO prebuilts (name, channel, version, triple, checksum)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(name, channel, version, triple) DO UPDATE SET checksum = excluded.checksum",
        )
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
        Ok(sqlx::query_as::<_, PrebuiltRow>(
            "SELECT triple, checksum FROM prebuilts
             WHERE name = ? AND channel = ? AND version = ? AND triple = ?",
        )
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
        Ok(sqlx::query_as::<_, PrebuiltRow>(
            "SELECT triple, checksum FROM prebuilts
             WHERE name = ? AND channel = ? AND version = ?
             ORDER BY triple",
        )
        .bind(name).bind(channel).bind(version)
        .fetch_all(&self.pool).await?)
    }

    /// Delete all prebuilts for a package (used when the package itself is deleted).
    pub async fn delete_package_prebuilts(&self, name: &str, channel: &str) -> Result<()> {
        sqlx::query("DELETE FROM prebuilts WHERE name = ? AND channel = ?")
            .bind(name).bind(channel)
            .execute(&self.pool).await?;
        Ok(())
    }

    // ── Metrics ────────────────────────────────────────────────────────────────

    pub async fn stats(&self) -> Result<DbStats> {
        let packages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM packages")
            .fetch_one(&self.pool).await?;
        let versions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM versions")
            .fetch_one(&self.pool).await?;
        let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool).await?;
        let now = unix_now();
        let tokens_active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tokens WHERE expires_at IS NULL OR expires_at > ?",
        )
        .bind(now)
        .fetch_one(&self.pool).await?;
        let downloads_total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(downloads), 0) FROM versions",
        )
        .fetch_one(&self.pool).await?;
        Ok(DbStats { packages, versions, users, tokens_active, downloads_total })
    }

    // ── Organizations ──────────────────────────────────────────────────────────

    pub async fn create_org(&self, name: &str, description: Option<&str>, owner_id: i64) -> Result<i64> {
        let org_id: i64 = sqlx::query_scalar(
            "INSERT INTO organizations (name, description) VALUES (?, ?) RETURNING id",
        )
        .bind(name)
        .bind(description)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query("INSERT INTO org_members (org_id, user_id, role) VALUES (?, ?, 'owner')")
            .bind(org_id)
            .bind(owner_id)
            .execute(&self.pool)
            .await?;

        Ok(org_id)
    }

    pub async fn get_org(&self, name: &str) -> Result<Option<OrgRow>> {
        let row = sqlx::query_as(
            "SELECT id, name, description, created_at FROM organizations WHERE lower(name) = lower(?)",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_orgs(&self) -> Result<Vec<OrgRow>> {
        let rows = sqlx::query_as(
            "SELECT id, name, description, created_at FROM organizations ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn delete_org(&self, name: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM organizations WHERE lower(name) = lower(?)")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_org_members(&self, org_name: &str) -> Result<Vec<OrgMemberRow>> {
        let rows = sqlx::query_as(
            "SELECT m.user_id, u.username, m.role
             FROM org_members m
             JOIN organizations o ON o.id = m.org_id
             JOIN users u ON u.id = m.user_id
             WHERE lower(o.name) = lower(?)
             ORDER BY m.role DESC, u.username",
        )
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
        sqlx::query(
            "INSERT INTO org_members (org_id, user_id, role) VALUES (?, ?, ?)
             ON CONFLICT(org_id, user_id) DO UPDATE SET role = excluded.role",
        )
        .bind(org.id)
        .bind(user.id)
        .bind(role)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    pub async fn remove_org_member(&self, org_name: &str, username: &str) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM org_members
             WHERE org_id = (SELECT id FROM organizations WHERE lower(name) = lower(?))
               AND user_id = (SELECT id FROM users WHERE lower(username) = lower(?))",
        )
        .bind(org_name)
        .bind(username)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn is_org_owner(&self, org_name: &str, user_id: i64) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM org_members m
             JOIN organizations o ON o.id = m.org_id
             WHERE lower(o.name) = lower(?) AND m.user_id = ? AND m.role = 'owner'",
        )
        .bind(org_name)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn is_org_member(&self, org_name: &str, user_id: i64) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM org_members m
             JOIN organizations o ON o.id = m.org_id
             WHERE lower(o.name) = lower(?) AND m.user_id = ?",
        )
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
        let result = sqlx::query(
            "UPDATE packages SET org_id = ? WHERE lower(name) = lower(?) AND lower(channel) = lower(?)",
        )
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
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM packages p
             JOIN org_members m ON m.org_id = p.org_id
             WHERE lower(p.name) = lower(?) AND lower(p.channel) = lower(?) AND m.user_id = ?",
        )
        .bind(package_name)
        .bind(channel)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }
}
