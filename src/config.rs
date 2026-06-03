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
//! # sqlite:// for a local file, postgres:// for a remote server
//! url = "sqlite:///var/lib/freight-registry/registry.db"
//! # url = "postgres://user:pass@db.internal/freight"
//!
//! [serve]
//! url               = "https://freight.example.com"
//! bind              = "0.0.0.0:7878"
//! max_upload_mb     = 50
//! mirror_upstream   = "https://freight.dev"
//! rate_limit_read   = 120
//! rate_limit_write  = 10
//! audit_log_ttl_days = 90
//!
//! # Malware scan backend used after each publish.
//! # One of: auto (default), docker, podman, clamscan, none.
//! [serve.scan]
//! backend = "docker"
//!
//! # CI verification pipeline: builds + tests + scans each source publish
//! # before making it public. Packages are held as `pending` until the
//! # container job passes. Omit this section to publish immediately.
//! #
//! # `default` is used for any platform not listed explicitly.
//! # Add per-platform images for cross-platform verification.
//! # Supported platform keys: linux, windows, freebsd, macos, openbsd,
//! #                           netbsd, dragonfly, solaris, android
//! [serve.verify]
//! default = "ghcr.io/tinytinyterminator/freight-ci-linux:latest"
//! linux   = "ghcr.io/tinytinyterminator/freight-ci-linux:latest"
//! windows = "ghcr.io/tinytinyterminator/freight-ci-windows:latest"
//! freebsd = "ghcr.io/tinytinyterminator/freight-ci-freebsd:latest"
//!
//! # Email delivery — omit this section to log links to stdout instead
//! [serve.smtp]
//! host     = "smtp.example.com"
//! port     = 587                          # default: 587 (STARTTLS), 465 (TLS), 25 (none)
//! username = "noreply@example.com"
//! password = "secret"
//! from     = "Freight Registry <noreply@example.com>"
//! tls      = "starttls"                   # "starttls" (default), "tls", or "none"
//!
//! [serve.s3]
//! bucket   = "freight-packages"
//! endpoint = "http://minio:9000"
//! key_id   = "AKIAIOSFODNN7EXAMPLE"
//! secret   = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
//! region   = "us-east-1"
//!
//! # OAuth / OIDC providers — each becomes a /auth/:name login route.
//! # Repeat the [[serve.oauth]] block for multiple providers.
//! #
//! # OIDC auto-discovery (Okta, Azure AD, Keycloak, self-hosted GitLab, …):
//! [[serve.oauth]]
//! name          = "okta"
//! display_name  = "Okta SSO"
//! client_id     = "0oa…"
//! client_secret = "…"
//! issuer        = "https://company.okta.com"
//!
//! # Manual endpoints (non-OIDC, e.g. Gitea):
//! [[serve.oauth]]
//! name                   = "gitea"
//! client_id              = "abc"
//! client_secret          = "def"
//! authorization_endpoint = "https://git.internal/login/oauth/authorize"
//! token_endpoint         = "https://git.internal/login/oauth/access_token"
//! userinfo_endpoint      = "https://git.internal/api/v1/user"
//! id_field               = "id"
//! username_field         = "login"
//! ```
//!
//! # OAuth env-var shortcuts (no config file needed)
//!
//! For GitHub, GitLab, and Google you can skip the config file entirely:
//!
//! ```sh
//! export GITHUB_CLIENT_ID=…     GITHUB_CLIENT_SECRET=…   # enables /auth/github
//! export GITLAB_CLIENT_ID=…     GITLAB_CLIENT_SECRET=…   # enables /auth/gitlab
//! export GITLAB_ISSUER=https://git.internal               # optional: self-hosted GitLab
//! export GOOGLE_CLIENT_ID=…     GOOGLE_CLIENT_SECRET=…   # enables /auth/google
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::oauth::OAuthProviderConfig;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Database URL. Use `sqlite:///path/to/db` for a local file or
    /// `postgres://user:pass@host/db` for a remote server.
    pub url:   Option<String>,
    pub data:  Option<String>,
    pub serve: Option<ServeConfig>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ServeConfig {
    /// Public base URL of this registry (embedded in download links).
    pub url:                Option<String>,
    pub bind:               Option<String>,
    pub max_upload_mb:      Option<u64>,
    pub mirror_upstream:    Option<String>,
    pub rate_limit_read:    Option<u32>,
    pub rate_limit_write:   Option<u32>,
    pub audit_log_ttl_days: Option<i64>,
    pub s3:                 Option<S3Config>,
    pub smtp:               Option<SmtpFileConfig>,
    /// Base URL of a separate download server (CDN, nginx, S3 public bucket, …).
    ///
    /// When set, the `/download` endpoints return a `302` redirect to
    /// `{download_url}/{name}/{version}/{name}-{version}.tar.gz` instead of
    /// streaming bytes through the registry server.
    ///
    /// When absent and the storage backend is S3, the server generates a
    /// presigned URL and redirects to that instead.
    ///
    /// When absent and the storage backend is local, bytes are streamed directly.
    pub download_url:       Option<String>,
    /// Malware scan settings.
    pub scan:               Option<ScanConfig>,
    /// CI verification pipeline settings.
    pub verify:             Option<VerifyConfig>,
    /// OAuth/OIDC providers.  Each entry becomes a `/auth/:name` login route.
    /// See [`OAuthProviderConfig`] for the full set of fields.
    ///
    /// Example (OIDC auto-discovery):
    /// ```toml
    /// [[serve.oauth]]
    /// name          = "okta"
    /// display_name  = "Okta SSO"
    /// client_id     = "0oa…"
    /// client_secret = "…"
    /// issuer        = "https://company.okta.com"
    /// ```
    #[serde(default)]
    pub oauth: Vec<OAuthProviderConfig>,
}

/// Malware scan settings under `[serve.scan]`.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ScanConfig {
    /// One of: `auto` (default), `docker`, `podman`, `clamscan`, `none`.
    pub backend: Option<String>,
}

/// CI verification pipeline settings under `[serve.verify]`.
///
/// Each platform key is a container image that will be used to build and test
/// published packages for that target platform. Packages are held as `pending`
/// until all configured platform pipelines pass.
///
/// Supported keys: `default`, `linux`, `windows`, `freebsd`, `macos`,
/// `openbsd`, `netbsd`, `dragonfly`, `solaris`, `android`.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct VerifyConfig {
    /// Fallback image used when no platform-specific image matches.
    pub default:   Option<String>,
    pub linux:     Option<String>,
    pub windows:   Option<String>,
    pub freebsd:   Option<String>,
    pub macos:     Option<String>,
    pub openbsd:   Option<String>,
    pub netbsd:    Option<String>,
    pub dragonfly: Option<String>,
    pub solaris:   Option<String>,
    pub android:   Option<String>,
}

/// SMTP settings under `[serve.smtp]` in the config file.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SmtpFileConfig {
    pub host:     Option<String>,
    pub port:     Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from:     Option<String>,
    /// `"starttls"` (default), `"tls"`, or `"none"`
    pub tls:      Option<String>,
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

/// Loaded configuration that cannot be expressed as env vars.
pub struct LoadedExtras {
    pub oauth:          Vec<OAuthProviderConfig>,
    /// Per-platform CI verification images from `[serve.verify]`.
    /// Keys are platform names (`"linux"`, `"windows"`, `"freebsd"`, …).
    /// `"default"` is the fallback for platforms not explicitly listed.
    pub verify_images:  std::collections::HashMap<String, String>,
}

/// Load a config file, inject scalar values as env vars (only for unset vars),
/// and return extras that cannot be expressed as env vars.
///
/// Returns `(path_loaded, extras)`.  `path_loaded` is `None` when no config
/// file was found.
pub fn load() -> (Option<PathBuf>, LoadedExtras) {
    let empty = LoadedExtras { oauth: vec![], verify_images: std::collections::HashMap::new() };
    let Some(path) = find_config_file() else {
        return (None, empty);
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warning: could not read config {}: {e}", path.display());
            return (None, empty);
        }
    };

    let cfg: Config = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: invalid config {}: {e}", path.display());
            std::process::exit(1);
        }
    };

    let oauth = cfg.serve.as_ref().map(|s| s.oauth.clone()).unwrap_or_default();
    let verify_images = cfg.serve.as_ref()
        .and_then(|s| s.verify.as_ref())
        .map(collect_verify_images)
        .unwrap_or_default();
    apply(cfg);
    (Some(path), LoadedExtras { oauth, verify_images })
}

fn collect_verify_images(v: &VerifyConfig) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    let mut ins = |k: &str, val: &Option<String>| {
        if let Some(img) = val { m.insert(k.to_string(), img.clone()); }
    };
    ins("default",   &v.default);
    ins("linux",     &v.linux);
    ins("windows",   &v.windows);
    ins("freebsd",   &v.freebsd);
    ins("macos",     &v.macos);
    ins("openbsd",   &v.openbsd);
    ins("netbsd",    &v.netbsd);
    ins("dragonfly", &v.dragonfly);
    ins("solaris",   &v.solaris);
    ins("android",   &v.android);
    m
}

fn find_config_file() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("FREIGHT_CONFIG") {
        return Some(PathBuf::from(p));
    }

    let system = Path::new("/etc/freight-registry.toml");
    if system.exists() {
        return Some(system.to_path_buf());
    }

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
        // Safety: single-threaded here — tokio runtime hasn't started yet.
        unsafe { std::env::set_var(key, val); }
    }
}

fn apply(cfg: Config) {
    if let Some(v) = cfg.url  { set_if_absent("DATABASE_URL", &v); }
    if let Some(v) = cfg.data { set_if_absent("FREIGHT_DATA_DIR", &v); }

    let Some(s) = cfg.serve else { return };

    if let Some(v) = s.url                { set_if_absent("FREIGHT_BASE_URL",          &v); }
    if let Some(v) = s.bind               { set_if_absent("FREIGHT_BIND",              &v); }
    if let Some(v) = s.max_upload_mb      { set_if_absent("FREIGHT_MAX_UPLOAD_MB",     &v.to_string()); }
    if let Some(v) = s.mirror_upstream    { set_if_absent("FREIGHT_MIRROR_UPSTREAM",   &v); }
    if let Some(v) = s.rate_limit_read    { set_if_absent("FREIGHT_RATE_LIMIT_READ",   &v.to_string()); }
    if let Some(v) = s.rate_limit_write   { set_if_absent("FREIGHT_RATE_LIMIT_WRITE",  &v.to_string()); }
    if let Some(v) = s.audit_log_ttl_days { set_if_absent("FREIGHT_AUDIT_LOG_TTL_DAYS",&v.to_string()); }
    if let Some(v) = s.download_url       { set_if_absent("FREIGHT_DOWNLOAD_URL",      &v); }

    if let Some(scan) = s.scan {
        if let Some(v) = scan.backend { set_if_absent("FREIGHT_SCAN_BACKEND", &v); }
    }

    if let Some(smtp) = s.smtp {
        if let Some(v) = smtp.host     { set_if_absent("FREIGHT_SMTP_HOST",     &v); }
        if let Some(v) = smtp.port     { set_if_absent("FREIGHT_SMTP_PORT",     &v.to_string()); }
        if let Some(v) = smtp.username { set_if_absent("FREIGHT_SMTP_USERNAME", &v); }
        if let Some(v) = smtp.password { set_if_absent("FREIGHT_SMTP_PASSWORD", &v); }
        if let Some(v) = smtp.from     { set_if_absent("FREIGHT_SMTP_FROM",     &v); }
        if let Some(v) = smtp.tls      { set_if_absent("FREIGHT_SMTP_TLS",      &v); }
    }

    let Some(s3) = s.s3 else { return };

    if let Some(v) = s3.bucket   { set_if_absent("FREIGHT_S3_BUCKET",   &v); }
    if let Some(v) = s3.endpoint { set_if_absent("FREIGHT_S3_ENDPOINT", &v); }
    if let Some(v) = s3.key_id   { set_if_absent("FREIGHT_S3_KEY_ID",   &v); }
    if let Some(v) = s3.secret   { set_if_absent("FREIGHT_S3_SECRET",   &v); }
    if let Some(v) = s3.region   { set_if_absent("FREIGHT_S3_REGION",   &v); }
}
