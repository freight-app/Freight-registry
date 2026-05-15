use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteConnectOptions, FromRow, SqlitePool};

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
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

#[derive(FromRow)]
pub struct TokenRow {
    pub id: i64,
    pub name: String,
}

impl Db {
    pub async fn open(path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(opts).await?;
        let db = Self { pool };
        db.migrate().await?;
        Ok(db)
    }

    async fn migrate(&self) -> Result<()> {
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
            "CREATE TABLE IF NOT EXISTS tokens (
                id         INTEGER PRIMARY KEY,
                name       TEXT NOT NULL UNIQUE,
                token_hash TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

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

    pub async fn publish_version(
        &self,
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

        Ok(())
    }

    pub async fn set_yanked(&self, name: &str, version: &str, yanked: bool) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE versions SET yanked = ?
             WHERE version = ?
               AND package_id = (SELECT id FROM packages WHERE lower(name) = lower(?))",
        )
        .bind(yanked as i64)
        .bind(version)
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn validate_token(&self, token: &str) -> Result<Option<TokenRow>> {
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        let row: Option<TokenRow> = sqlx::query_as(
            "SELECT id, name FROM tokens WHERE token_hash = ?",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn create_token(&self, name: &str) -> Result<String> {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        sqlx::query("INSERT INTO tokens (name, token_hash) VALUES (?, ?)")
            .bind(name)
            .bind(&hash)
            .execute(&self.pool)
            .await?;
        Ok(token)
    }

    pub async fn list_tokens(&self) -> Result<Vec<TokenRow>> {
        let rows = sqlx::query_as("SELECT id, name FROM tokens ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn revoke_token(&self, name: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM tokens WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
