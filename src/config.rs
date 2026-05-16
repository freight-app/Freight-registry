//! Config-file loader.
//!
//! Reads a TOML file from (in order):
//!   1. `$FREIGHT_CONFIG`
//!   2. `/etc/freight-registry.toml`
//!   3. `$XDG_CONFIG_HOME/freight-registry/config.toml`  (or `~/.config/...`)
//!
//! Values from the file are injected as environment variables **only when the
//! variable is not already set**, so CLI flags and shell env always win.
//!
//! Example config:
//!
//! ```toml
//! data         = "/var/lib/freight-registry"
//! database_url = "postgres://user:pass@db.internal/freight"
//!
//! [serve]
//! bind              = "0.0.0.0:7878"
//! base_url          = "https://freight.example.com"
//! max_upload_mb     = 50
//! mirror_upstream   = "https://freight.dev"
//! rate_limit_read   = 120
//! rate_limit_write  = 10
//! audit_log_ttl_days = 90
//!
//! [serve.s3]
//! bucket   = "freight-packages"
//! endpoint = "http://minio:9000"
//! key_id   = "AKIAIOSFODNN7EXAMPLE"
//! secret   = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
//! region   = "us-east-1"
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub data:         Option<String>,
    pub database_url: Option<String>,
    pub serve:        Option<ServeConfig>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ServeConfig {
    pub bind:               Option<String>,
    pub base_url:           Option<String>,
    pub max_upload_mb:      Option<u64>,
    pub mirror_upstream:    Option<String>,
    pub rate_limit_read:    Option<u32>,
    pub rate_limit_write:   Option<u32>,
    pub audit_log_ttl_days: Option<i64>,
    pub s3:                 Option<S3Config>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct S3Config {
    pub bucket:   Option<String>,
    pub endpoint: Option<String>,
    pub key_id:   Option<String>,
    pub secret:   Option<String>,
    pub region:   Option<String>,
}

/// Load a config file and inject values as env vars (only for unset vars).
/// Returns the path that was loaded, or `None` if no file was found.
pub fn load() -> Option<PathBuf> {
    let path = find_config_file()?;

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warning: could not read config {}: {e}", path.display());
            return None;
        }
    };

    let cfg: Config = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: invalid config {}: {e}", path.display());
            std::process::exit(1);
        }
    };

    apply(cfg);
    Some(path)
}

fn find_config_file() -> Option<PathBuf> {
    // 1. Explicit override
    if let Ok(p) = std::env::var("FREIGHT_CONFIG") {
        return Some(PathBuf::from(p));
    }

    // 2. System-wide
    let system = Path::new("/etc/freight-registry.toml");
    if system.exists() {
        return Some(system.to_path_buf());
    }

    // 3. User config dir
    let user = user_config_path()?;
    if user.exists() {
        return Some(user);
    }

    None
}

fn user_config_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("freight-registry/config.toml"))
}

/// Set an env var only if it is not already present in the environment.
fn set_if_absent(key: &str, val: &str) {
    if std::env::var(key).is_err() {
        // Safety: single-threaded at this point (before tokio runtime starts).
        unsafe { std::env::set_var(key, val); }
    }
}

fn apply(cfg: Config) {
    if let Some(v) = cfg.data         { set_if_absent("FREIGHT_DATA_DIR", &v); }
    if let Some(v) = cfg.database_url { set_if_absent("DATABASE_URL",     &v); }

    let Some(s) = cfg.serve else { return };

    if let Some(v) = s.bind               { set_if_absent("FREIGHT_BIND",              &v); }
    if let Some(v) = s.base_url           { set_if_absent("FREIGHT_BASE_URL",          &v); }
    if let Some(v) = s.max_upload_mb      { set_if_absent("FREIGHT_MAX_UPLOAD_MB",     &v.to_string()); }
    if let Some(v) = s.mirror_upstream    { set_if_absent("FREIGHT_MIRROR_UPSTREAM",   &v); }
    if let Some(v) = s.rate_limit_read    { set_if_absent("FREIGHT_RATE_LIMIT_READ",   &v.to_string()); }
    if let Some(v) = s.rate_limit_write   { set_if_absent("FREIGHT_RATE_LIMIT_WRITE",  &v.to_string()); }
    if let Some(v) = s.audit_log_ttl_days { set_if_absent("FREIGHT_AUDIT_LOG_TTL_DAYS",&v.to_string()); }

    let Some(s3) = s.s3 else { return };

    if let Some(v) = s3.bucket   { set_if_absent("FREIGHT_S3_BUCKET",   &v); }
    if let Some(v) = s3.endpoint { set_if_absent("FREIGHT_S3_ENDPOINT", &v); }
    if let Some(v) = s3.key_id   { set_if_absent("FREIGHT_S3_KEY_ID",   &v); }
    if let Some(v) = s3.secret   { set_if_absent("FREIGHT_S3_SECRET",   &v); }
    if let Some(v) = s3.region   { set_if_absent("FREIGHT_S3_REGION",   &v); }
}
