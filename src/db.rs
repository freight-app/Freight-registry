use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteConnectOptions, FromRow, SqlitePool};

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ── Row types ─────────────────────────────────────────────────────────────────

#[derive(FromRow, Clone)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub email: Option<String>,
    pub password_hash: String,
    pub is_admin: i64,
}

#[derive(FromRow, Clone)]
pub struct TokenRow {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub expires_at: Option<i64>,
    pub last_used: Option<i64>,
}

#[derive(FromRow)]
pub struct TokenWithUser {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub expires_at: Option<i64>,
    pub last_used: Option<i64>,
    pub username: String,
}

#[derive(FromRow)]
pub struct PackageRow {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
}

#[derive(FromRow)]
pub struct VersionRow {
    pub version: String,
    pub checksum: String,
    pub yanked: i64,
}

// ── Database handle ───────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn open(path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .pragma("foreign_keys", "ON")
            .pragma("journal_mode", "WAL");
        let pool = SqlitePool::connect_with(opts).await?;
        let db = Self { pool };
        db.migrate().await?;
        Ok(db)
    }

    async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS users (
                id            INTEGER PRIMARY KEY,
                username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
                email         TEXT UNIQUE,
                password_hash TEXT NOT NULL,
                created_at    INTEGER NOT NULL DEFAULT (unixepoch())
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS packages (
                id          INTEGER PRIMARY KEY,
                name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
                description TEXT,
                created_at  INTEGER NOT NULL DEFAULT (unixepoch())
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS versions (
                id         INTEGER PRIMARY KEY,
                package_id INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
                version    TEXT    NOT NULL,
                checksum   TEXT    NOT NULL,
                yanked     INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(package_id, version)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS package_owners (
                package_id INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
                user_id    INTEGER NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
                PRIMARY KEY (package_id, user_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        // If tokens table predates user_id, recreate it (dev-only migration).
        let has_user_id: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('tokens') WHERE name = 'user_id'",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        if has_user_id == 0 {
            sqlx::query("DROP TABLE IF EXISTS tokens")
                .execute(&self.pool)
                .await?;
        }

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tokens (
                id         INTEGER PRIMARY KEY,
                user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                name       TEXT    NOT NULL,
                token_hash TEXT    NOT NULL UNIQUE,
                expires_at INTEGER,
                last_used  INTEGER,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(user_id, name)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id         INTEGER PRIMARY KEY,
                user_id    INTEGER REFERENCES users(id) ON DELETE SET NULL,
                action     TEXT    NOT NULL,
                package    TEXT,
                version    TEXT,
                ip_addr    TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            )",
        )
        .execute(&self.pool)
        .await?;

        // Additive migration: is_admin column (added in v2).
        let has_is_admin: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('users') WHERE name = 'is_admin'",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);
        if has_is_admin == 0 {
            sqlx::query(
                "ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&self.pool)
            .await?;
        }

        Ok(())
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
            "SELECT id, username, email, password_hash, is_admin FROM users
             WHERE lower(username) = lower(?)",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_users(&self) -> Result<Vec<UserRow>> {
        let rows = sqlx::query_as(
            "SELECT id, username, email, password_hash, is_admin FROM users ORDER BY username",
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

    // ── Tokens ─────────────────────────────────────────────────────────────────

    /// Create a new token for `user_id`. `expires_days` = `None` means no expiry.
    /// Returns the raw token string (shown to the user once).
    pub async fn create_token(
        &self,
        user_id: i64,
        name: &str,
        expires_days: Option<i64>,
    ) -> Result<String> {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        let expires_at = expires_days.map(|d| unix_now() + d * 86_400);
        sqlx::query(
            "INSERT INTO tokens (user_id, name, token_hash, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(name)
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
            "SELECT id, user_id, name, expires_at, last_used
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
            "SELECT id, username, email, password_hash, is_admin FROM users WHERE id = ?",
        )
        .bind(tok.user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Some((tok, user)))
    }

    pub async fn list_tokens(&self, user_id: Option<i64>) -> Result<Vec<TokenWithUser>> {
        if let Some(uid) = user_id {
            sqlx::query_as(
                "SELECT t.id, t.user_id, t.name, t.expires_at, t.last_used, u.username
                 FROM tokens t JOIN users u ON u.id = t.user_id
                 WHERE t.user_id = ? ORDER BY t.created_at",
            )
            .bind(uid)
            .fetch_all(&self.pool)
            .await
            .map_err(Into::into)
        } else {
            sqlx::query_as(
                "SELECT t.id, t.user_id, t.name, t.expires_at, t.last_used, u.username
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

    pub async fn get_package(&self, name: &str) -> Result<Option<(PackageRow, Vec<VersionRow>)>> {
        let pkg: Option<PackageRow> = sqlx::query_as(
            "SELECT id, name, description FROM packages WHERE lower(name) = lower(?)",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        let Some(pkg) = pkg else { return Ok(None) };

        let versions: Vec<VersionRow> = sqlx::query_as(
            "SELECT version, checksum, yanked FROM versions
             WHERE package_id = ? ORDER BY created_at DESC",
        )
        .bind(pkg.id)
        .fetch_all(&self.pool)
        .await?;

        Ok(Some((pkg, versions)))
    }

    pub async fn search_packages(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(PackageRow, Option<VersionRow>)>> {
        let pattern = format!("%{query}%");
        let pkgs: Vec<PackageRow> = sqlx::query_as(
            "SELECT id, name, description FROM packages
             WHERE lower(name) LIKE lower(?)
               AND EXISTS (SELECT 1 FROM versions WHERE package_id = id)
             ORDER BY name LIMIT ?",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::with_capacity(pkgs.len());
        for pkg in pkgs {
            let latest: Option<VersionRow> = sqlx::query_as(
                "SELECT version, checksum, yanked FROM versions
                 WHERE package_id = ? AND yanked = 0 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(pkg.id)
            .fetch_optional(&self.pool)
            .await?;
            results.push((pkg, latest));
        }
        Ok(results)
    }

    /// Publish a new version. Grants ownership to `user_id` if the package is new.
    pub async fn publish_version(
        &self,
        user_id: i64,
        name: &str,
        description: Option<&str>,
        version: &str,
        checksum: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO packages (name, description) VALUES (?, ?)
             ON CONFLICT(name) DO UPDATE SET description = COALESCE(excluded.description, description)",
        )
        .bind(name)
        .bind(description)
        .execute(&self.pool)
        .await?;

        let pkg: PackageRow = sqlx::query_as(
            "SELECT id, name, description FROM packages WHERE lower(name) = lower(?)",
        )
        .bind(name)
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

    pub async fn set_yanked(&self, name: &str, version: &str, yanked: bool) -> Result<bool> {
        let n = sqlx::query(
            "UPDATE versions SET yanked = ?
             WHERE version = ?
               AND package_id = (SELECT id FROM packages WHERE lower(name) = lower(?))",
        )
        .bind(yanked as i64)
        .bind(version)
        .bind(name)
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
    ) -> Result<Option<bool>> {
        let pkg: Option<PackageRow> = sqlx::query_as(
            "SELECT id, name, description FROM packages WHERE lower(name) = lower(?)",
        )
        .bind(package_name)
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

    pub async fn get_package_owners(&self, package_name: &str) -> Result<Vec<UserRow>> {
        let rows = sqlx::query_as(
            "SELECT u.id, u.username, u.email, u.password_hash, u.is_admin
             FROM users u
             JOIN package_owners po ON po.user_id = u.id
             JOIN packages p        ON p.id = po.package_id
             WHERE lower(p.name) = lower(?)
             ORDER BY u.username",
        )
        .bind(package_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn add_package_owner(&self, package_name: &str, username: &str) -> Result<bool> {
        let pkg: Option<PackageRow> = sqlx::query_as(
            "SELECT id, name, description FROM packages WHERE lower(name) = lower(?)",
        )
        .bind(package_name)
        .fetch_optional(&self.pool)
        .await?;
        let Some(pkg) = pkg else { return Ok(false) };

        let user: Option<UserRow> = sqlx::query_as(
            "SELECT id, username, email, password_hash, is_admin FROM users WHERE lower(username) = lower(?)",
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

    pub async fn remove_package_owner(&self, package_name: &str, username: &str) -> Result<bool> {
        let n = sqlx::query(
            "DELETE FROM package_owners
             WHERE package_id = (SELECT id FROM packages WHERE lower(name) = lower(?))
               AND user_id    = (SELECT id FROM users    WHERE lower(username) = lower(?))",
        )
        .bind(package_name)
        .bind(username)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n > 0)
    }

    // ── Audit log ──────────────────────────────────────────────────────────────

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
}
