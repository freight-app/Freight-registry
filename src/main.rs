//! freight-registry — self-hosted freight package registry.
//!
//! Quick-start:
//!
//!   # Create the first user
//!   freight-registry --data /var/lib/freight-registry user add alice
//!
//!   # Issue an API token for that user
//!   freight-registry --data /var/lib/freight-registry token add deploy --user alice
//!
//!   # Start the server
//!   freight-registry --data /var/lib/freight-registry serve \
//!       --base-url https://freight.example.com

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use sqlx::Row as _;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use freight_registry::{
    api,
    auth::hash_password,
    config,
    db::Db,
    mail::{Mailer, SmtpConfig, SmtpMailer, StdoutMailer},
    metrics::Metrics,
    oauth::OAuthProviderConfig,
    rate_limit::Limiters,
    storage::Storage,
    validate,
    AppState,
};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "freight-registry", about = "Self-hosted freight package registry")]
struct Cli {
    /// Directory for the database and stored tarballs (used when --database-url is absent)
    #[arg(long, env = "FREIGHT_DATA_DIR", default_value = "/var/lib/freight-registry")]
    data: PathBuf,

    /// Database URL (sqlite:///path/to/db or postgres://user:pass@host/db).
    /// When set, overrides the default SQLite file in --data.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP registry server
    Serve {
        /// Address and port to bind to
        #[arg(long, env = "FREIGHT_BIND", default_value = "0.0.0.0:7878")]
        bind: String,
        /// Publicly reachable base URL (embedded in download links)
        #[arg(long, env = "FREIGHT_BASE_URL", default_value = "http://localhost:7878")]
        base_url: String,
        /// Maximum upload size in megabytes (default 50)
        #[arg(long, env = "FREIGHT_MAX_UPLOAD_MB", default_value_t = 50)]
        max_upload_mb: usize,
        /// Delete audit log entries older than this many days (omit to keep forever)
        #[arg(long, env = "FREIGHT_AUDIT_LOG_TTL_DAYS")]
        audit_log_ttl_days: Option<i64>,
        /// Read rate limit in requests per minute per IP (default 120)
        #[arg(long, env = "FREIGHT_RATE_LIMIT_READ", default_value_t = 120)]
        rate_limit_read: u32,
        /// Write rate limit in requests per minute per IP (default 10)
        #[arg(long, env = "FREIGHT_RATE_LIMIT_WRITE", default_value_t = 10)]
        rate_limit_write: u32,
        // ── S3-compatible storage (all four must be set together) ────────────
        /// S3/MinIO bucket name (enables S3 storage backend)
        #[arg(long, env = "FREIGHT_S3_BUCKET")]
        s3_bucket: Option<String>,
        /// S3 endpoint URL, e.g. http://localhost:9000 for MinIO (omit for AWS)
        #[arg(long, env = "FREIGHT_S3_ENDPOINT")]
        s3_endpoint: Option<String>,
        /// S3 access key ID
        #[arg(long, env = "FREIGHT_S3_KEY_ID")]
        s3_key_id: Option<String>,
        /// S3 secret access key
        #[arg(long, env = "FREIGHT_S3_SECRET")]
        s3_secret: Option<String>,
        /// S3 region (default: us-east-1)
        #[arg(long, env = "FREIGHT_S3_REGION", default_value = "us-east-1")]
        s3_region: String,
        /// Upstream registry to proxy unknown packages from (e.g. https://freight.dev)
        #[arg(long, env = "FREIGHT_MIRROR_UPSTREAM")]
        mirror_upstream: Option<String>,
        /// Maximum number of packages a non-admin user may own (omit for no limit)
        #[arg(long, env = "FREIGHT_MAX_PACKAGES_PER_USER")]
        max_packages_per_user: Option<u32>,
        /// Comma-separated list of languages this registry accepts.  Packages
        /// that do not declare at least one matching language in their
        /// freight.toml are rejected at publish time.  Omit to allow all.
        /// Example: --allowed-languages c,cpp,fortran
        /// Env: FREIGHT_ALLOWED_LANGUAGES=c,cpp,fortran
        #[arg(long, env = "FREIGHT_ALLOWED_LANGUAGES", value_delimiter = ',')]
        allowed_languages: Vec<String>,
        /// Malware-scan backend used after each publish.
        /// One of: auto (default), docker, podman, clamscan, none.
        /// `auto` probes for Docker, then Podman, then bare clamscan at startup.
        /// Container backends (docker/podman) extract and scan inside an isolated
        /// environment with no network access and tight resource limits.
        #[arg(long, env = "FREIGHT_SCAN_BACKEND", default_value = "auto")]
        scan_backend: freight_registry::ScanBackend,
        /// Default container image for the CI verification pipeline.
        /// When set, every source publish is held as `pending` until the
        /// container builds it, runs its tests, and scans it for malware.
        /// The container must output a JSON result to stdout (see docs).
        /// Omit to publish immediately without verification.
        /// Example: registry.example.com/freight-ci:latest
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE")]
        verify_image: Option<String>,
        /// Per-platform CI images. When set, packages are verified against
        /// each matching platform before being made public.
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_LINUX")]
        verify_image_linux: Option<String>,
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_WINDOWS")]
        verify_image_windows: Option<String>,
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_FREEBSD")]
        verify_image_freebsd: Option<String>,
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_MACOS")]
        verify_image_macos: Option<String>,
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_OPENBSD")]
        verify_image_openbsd: Option<String>,
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_NETBSD")]
        verify_image_netbsd: Option<String>,
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_DRAGONFLY")]
        verify_image_dragonfly: Option<String>,
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_SOLARIS")]
        verify_image_solaris: Option<String>,
        #[arg(long, env = "FREIGHT_VERIFY_IMAGE_ANDROID")]
        verify_image_android: Option<String>,
        /// Base URL of a separate download server (CDN, nginx, public S3 bucket, …).
        /// When set, /download endpoints return a 302 redirect here instead of
        /// streaming bytes through the registry server.  When absent and the S3
        /// backend is configured, the server generates presigned URLs automatically.
        #[arg(long, env = "FREIGHT_DOWNLOAD_URL")]
        download_url: Option<String>,
        // ── SMTP email delivery (all optional; omit to log links to stdout) ──
        /// SMTP server hostname — enables real email delivery when set
        #[arg(long, env = "FREIGHT_SMTP_HOST")]
        smtp_host: Option<String>,
        /// SMTP server port (default: 587 for STARTTLS, 465 for TLS, 25 for none)
        #[arg(long, env = "FREIGHT_SMTP_PORT")]
        smtp_port: Option<u16>,
        /// SMTP login username
        #[arg(long, env = "FREIGHT_SMTP_USERNAME")]
        smtp_username: Option<String>,
        /// SMTP login password
        #[arg(long, env = "FREIGHT_SMTP_PASSWORD")]
        smtp_password: Option<String>,
        /// Sender address, e.g. "Freight <noreply@example.com>"
        #[arg(long, env = "FREIGHT_SMTP_FROM", default_value = "Freight Registry <noreply@localhost>")]
        smtp_from: String,
        /// TLS mode: starttls (default), tls, or none
        #[arg(long, env = "FREIGHT_SMTP_TLS", default_value = "starttls")]
        smtp_tls: String,
    },
    /// Manage user accounts
    User {
        #[command(subcommand)]
        command: UserCmd,
    },
    /// Manage API tokens
    Token {
        #[command(subcommand)]
        command: TokenCmd,
    },
    /// Delete stored blobs (source tarballs, prebuilts, docs) for yanked versions.
    ///
    /// The DB rows are kept so yank history is preserved; only the on-disk / S3
    /// files are removed.  Dry-run by default — pass --execute to actually delete.
    Gc {
        /// Storage directory (only used for local-filesystem backend;
        /// ignored when --s3-bucket is configured).
        #[arg(long, env = "FREIGHT_DATA_DIR", default_value = "/var/lib/freight-registry")]
        data: PathBuf,
        /// Actually delete files. Without this flag only a report is printed.
        #[arg(long)]
        execute: bool,
    },
    /// Bulk-import metadata-only package stubs from a directory of TOML files.
    ///
    /// Each .toml file must have a `[package]` section with at least `name` and
    /// `version`. All packages are inserted as metadata-only entries (no tarballs).
    Import {
        /// Directory containing the stub .toml files
        dir: PathBuf,
        /// Registry user who will own the imported packages (default: vcpkg)
        #[arg(long, default_value = "vcpkg")]
        user: String,
        /// Channel to import into (default: stable)
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Number of rows per SQL batch (default: 500)
        #[arg(long, default_value_t = 500)]
        batch: usize,
    },
}

#[derive(Subcommand)]
enum UserCmd {
    /// Create a new user account
    Add {
        username: String,
        #[arg(long)]
        email: Option<String>,
        /// Password (min 8 chars). Prompted interactively if omitted.
        #[arg(long)]
        password: Option<String>,
    },
    /// List all users
    List,
    /// Remove a user and all their tokens
    Remove { username: String },
    /// Grant admin privileges to a user
    Promote { username: String },
    /// Revoke admin privileges from a user
    Demote { username: String },
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Create a token for a user (printed once, never stored in plain text)
    Add {
        name: String,
        #[arg(long)]
        user: String,
        /// Expiry in days (default: no expiry)
        #[arg(long)]
        expires: Option<i64>,
    },
    /// List tokens (all users, or filtered by --user)
    List {
        #[arg(long)]
        user: Option<String>,
    },
    /// Revoke a token by name for a given user
    Revoke {
        name: String,
        #[arg(long)]
        user: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Load config file before clap parses, so file values appear as env vars
    // that clap picks up (CLI flags and real env vars still take priority).
    // OAuth provider configs and verify images are returned directly.
    let (config_path, config_extras) = config::load();
    let config_oauth = config_extras.oauth;
    let mut config_verify_images = config_extras.verify_images;
    if let Some(ref path) = config_path {
        eprintln!("loaded config: {}", path.display());
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "freight_registry=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.data)?;

    let db = if let Some(ref url) = cli.database_url {
        tracing::info!("using database: {url}");
        Db::open_url(url).await?
    } else {
        Db::open(&cli.data.join("registry.db")).await?
    };

    match cli.command {
        Command::Serve {
            bind, base_url, max_upload_mb, audit_log_ttl_days,
            rate_limit_read, rate_limit_write,
            s3_bucket, s3_endpoint, s3_key_id, s3_secret, s3_region,
            mirror_upstream, max_packages_per_user, allowed_languages,
            scan_backend, verify_image,
            verify_image_linux, verify_image_windows, verify_image_freebsd,
            verify_image_macos, verify_image_openbsd, verify_image_netbsd,
            verify_image_dragonfly, verify_image_solaris, verify_image_android,
            download_url,
            smtp_host, smtp_port, smtp_username, smtp_password, smtp_from, smtp_tls,
        } => {
            let storage = match s3_bucket {
                Some(ref bucket) => {
                    let key_id = s3_key_id
                        .ok_or_else(|| anyhow::anyhow!("--s3-key-id required when --s3-bucket is set"))?;
                    let secret = s3_secret
                        .ok_or_else(|| anyhow::anyhow!("--s3-secret required when --s3-bucket is set"))?;
                    tracing::info!("using S3 storage backend: bucket={bucket}");
                    Storage::s3(bucket, s3_endpoint.as_deref(), &key_id, &secret, &s3_region)?
                }
                None => {
                    tracing::info!("using local storage backend: {}", cli.data.join("tarballs").display());
                    Storage::new(cli.data.join("tarballs"))
                }
            };

            if let Some(ref upstream) = mirror_upstream {
                tracing::info!("mirror upstream: {upstream}");
            }

            if let Some(limit) = max_packages_per_user {
                tracing::info!("max packages per non-admin user: {limit}");
            }

            let download_url = download_url.map(|u| u.trim_end_matches('/').to_string());
            if let Some(ref dl) = download_url {
                tracing::info!("download server: {dl}");
            } else if storage.is_s3() {
                tracing::info!("download server: S3 presigned URLs (15 min TTL)");
            } else {
                tracing::info!("download server: streaming from local storage");
            }

            // Build mailer: real SMTP when --smtp-host is provided, stdout otherwise.
            let mailer: Arc<dyn Mailer> = if let Some(ref host) = smtp_host {
                let cfg = SmtpConfig {
                    host:     host.clone(),
                    port:     smtp_port,
                    username: smtp_username,
                    password: smtp_password,
                    from:     smtp_from,
                    tls:      Some(smtp_tls),
                };
                match SmtpMailer::new(&cfg) {
                    Ok(m) => {
                        tracing::info!("smtp: delivering email via {host}");
                        Arc::new(m)
                    }
                    Err(e) => {
                        tracing::error!("smtp: failed to initialise ({e:#}); falling back to stdout logging");
                        Arc::new(StdoutMailer)
                    }
                }
            } else {
                tracing::info!("smtp: no host configured — email links will be logged to stdout");
                Arc::new(StdoutMailer)
            };

            // ── OAuth providers ──────────────────────────────────────────────
            // Start with any providers declared in the config file.
            let mut provider_configs: Vec<OAuthProviderConfig> = config_oauth;

            // Add env-var presets for well-known services (only when the same
            // provider wasn't already declared in the config file).
            let existing_names: Vec<String> = provider_configs.iter().map(|p| p.name.clone()).collect();
            if !existing_names.iter().any(|n| n == "github") {
                if let Some(gh) = OAuthProviderConfig::github_from_env() {
                    provider_configs.push(gh);
                }
            }
            if !existing_names.iter().any(|n| n == "gitlab") {
                if let Some(gl) = OAuthProviderConfig::gitlab_from_env() {
                    provider_configs.push(gl);
                }
            }
            if !existing_names.iter().any(|n| n == "google") {
                if let Some(go) = OAuthProviderConfig::google_from_env() {
                    provider_configs.push(go);
                }
            }

            // Resolve all providers (OIDC discovery runs here at startup).
            let mut oauth_providers = Vec::new();
            for cfg in provider_configs {
                let name = cfg.name.clone();
                match cfg.resolve().await {
                    Ok(p) => {
                        tracing::info!(
                            "OAuth provider enabled: {} (/{}/…)",
                            p.display_name, p.name
                        );
                        oauth_providers.push(p);
                    }
                    Err(e) => {
                        tracing::warn!("OAuth provider '{name}' failed to resolve: {e:#}");
                    }
                }
            }

            if oauth_providers.is_empty() {
                tracing::info!("OAuth: no providers configured — password login only");
            }

            // Merge per-platform verify images: CLI flags override config file.
            {
                let mut ins = |k: &str, v: Option<String>| {
                    if let Some(img) = v { config_verify_images.insert(k.to_string(), img); }
                };
                ins("linux",     verify_image_linux);
                ins("windows",   verify_image_windows);
                ins("freebsd",   verify_image_freebsd);
                ins("macos",     verify_image_macos);
                ins("openbsd",   verify_image_openbsd);
                ins("netbsd",    verify_image_netbsd);
                ins("dragonfly", verify_image_dragonfly);
                ins("solaris",   verify_image_solaris);
                ins("android",   verify_image_android);
                // Single --verify-image also acts as "default"
                if let Some(ref img) = verify_image {
                    config_verify_images.entry("default".to_string())
                        .or_insert_with(|| img.clone());
                }
            }

            if !config_verify_images.is_empty() {
                tracing::info!("verify pipelines: {} platform(s) configured: {}",
                    config_verify_images.len(),
                    config_verify_images.keys().cloned().collect::<Vec<_>>().join(", "));
            }

            let state = Arc::new(AppState {
                db,
                storage,
                base_url:             base_url.trim_end_matches('/').to_string(),
                limiters:             Limiters::new(rate_limit_read, rate_limit_write),
                metrics:              Metrics::new(),
                mailer,
                mirror_upstream:      mirror_upstream.map(|u| u.trim_end_matches('/').to_string()),
                max_packages_per_user,
                allowed_languages:    if allowed_languages.is_empty() { None } else { Some(allowed_languages) },
                scan_backend,
                verify_image,
                verify_images:        config_verify_images,
                download_url,
                oauth_providers,
                oauth_states:         std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            });

            // Spawn audit log pruning task if a TTL was configured.
            if let Some(ttl_days) = audit_log_ttl_days {
                let db_clone = state.db.clone();
                tokio::spawn(async move {
                    let mut interval =
                        tokio::time::interval(tokio::time::Duration::from_secs(24 * 3_600));
                    loop {
                        interval.tick().await;
                        match db_clone.prune_audit_log(ttl_days).await {
                            Ok(n) => tracing::info!(
                                "audit log pruned: {n} entries older than {ttl_days} day(s) removed"
                            ),
                            Err(e) => tracing::error!("audit log prune failed: {e:#}"),
                        }
                    }
                });
            }

            let max_bytes = max_upload_mb * 1024 * 1024;
            let app = api::router(state, max_bytes);
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!("listening on {bind} (max upload {max_upload_mb} MB)");
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await?;
        }

        Command::User { command } => match command {
            UserCmd::Add { username, email, password } => {
                validate::username(&username).map_err(|e| anyhow::anyhow!("{e}"))?;
                let pw = match password {
                    Some(p) => p,
                    None => prompt_password()?,
                };
                validate::password(&pw).map_err(|e| anyhow::anyhow!("{e}"))?;
                let hash = hash_password(&pw)?;
                db.create_user(&username, email.as_deref(), &hash).await?;
                println!("User '{username}' created.");
            }
            UserCmd::List => {
                let users = db.list_users().await?;
                if users.is_empty() {
                    println!("no users");
                } else {
                    println!("{:<6}  {:<24}  {:<6}  email", "id", "username", "admin");
                    println!("{}", "-".repeat(65));
                    for u in users {
                        println!(
                            "{:<6}  {:<24}  {:<6}  {}",
                            u.id,
                            u.username,
                            if u.is_admin != 0 { "yes" } else { "no" },
                            u.email.as_deref().unwrap_or("-")
                        );
                    }
                }
            }
            UserCmd::Remove { username } => {
                if db.delete_user(&username).await? {
                    println!("removed user '{username}' and all their tokens");
                } else {
                    anyhow::bail!("no user named '{username}'");
                }
            }
            UserCmd::Promote { username } => {
                if db.set_admin(&username, true).await? {
                    println!("'{username}' is now an admin");
                } else {
                    anyhow::bail!("no user named '{username}'");
                }
            }
            UserCmd::Demote { username } => {
                if db.set_admin(&username, false).await? {
                    println!("removed admin from '{username}'");
                } else {
                    anyhow::bail!("no user named '{username}'");
                }
            }
        },

        Command::Token { command } => match command {
            TokenCmd::Add { name, user, expires } => {
                let u = db
                    .get_user_by_username(&user)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no user named '{user}'"))?;
                let token = db.create_token(u.id, &name, expires, "api", "publish").await?;
                println!("Token '{name}' created for '{user}':\n\n  {token}\n");
                println!("Store this value — it will not be shown again.");
                if let Some(days) = expires {
                    println!("Expires in {days} day(s).");
                }
            }
            TokenCmd::List { user } => {
                let uid = if let Some(uname) = user {
                    Some(
                        db.get_user_by_username(&uname)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("no user named '{uname}'"))?
                            .id,
                    )
                } else {
                    None
                };
                let tokens = db.list_tokens(uid).await?;
                if tokens.is_empty() {
                    println!("no tokens");
                } else {
                    println!("{:<6}  {:<24}  {:<20}  {:<8}  expires", "id", "user", "name", "kind");
                    println!("{}", "-".repeat(75));
                    for t in tokens {
                        let exp = t
                            .expires_at
                            .map(|ts| {
                                let days_left = (ts
                                    - std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs() as i64)
                                    / 86_400;
                                format!("{days_left}d")
                            })
                            .unwrap_or_else(|| "never".into());
                        println!(
                            "{:<6}  {:<24}  {:<20}  {:<8}  {exp}",
                            t.id, t.username, t.name, t.kind
                        );
                    }
                }
            }
            TokenCmd::Revoke { name, user } => {
                let u = db
                    .get_user_by_username(&user)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no user named '{user}'"))?;
                if db.revoke_token(u.id, &name).await? {
                    println!("revoked token '{name}' for '{user}'");
                } else {
                    anyhow::bail!("no token named '{name}' for user '{user}'");
                }
            }
        },

        Command::Import { dir, user, channel, batch: batch_size } => {
            cmd_import(&db, &dir, &user, &channel, batch_size).await?;
        }
        Command::Gc { data, execute } => {
            cmd_gc(&db, &data, execute).await?;
        }
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn prompt_password() -> Result<String> {
    use std::io::{self, Write};
    print!("Password: ");
    io::stdout().flush()?;
    let mut pw = String::new();
    io::stdin().read_line(&mut pw)?;
    Ok(pw.trim_end_matches('\n').to_string())
}

// ── Bulk import ───────────────────────────────────────────────────────────────

/// TOML stub file format (as produced by vcpkg-scraper).
#[derive(Deserialize)]
struct StubPackage {
    name:        String,
    version:     String,
    description: Option<String>,
    license:     Option<String>,
    url:         Option<String>,
    build:       Option<String>,
    #[serde(default)]
    supports:    Option<String>,
    #[allow(dead_code)]
    keywords:    Option<Vec<String>>,
}

#[derive(Deserialize)]
struct StubFile {
    package:     StubPackage,
    #[serde(default)]
    dependencies: HashMap<String, toml::Value>,
}

async fn cmd_import(
    db:         &Db,
    dir:        &PathBuf,
    user:       &str,
    channel:    &str,
    batch_size: usize,
) -> Result<()> {
    // Resolve the owner account.
    let owner = db.get_user_by_username(user).await?
        .ok_or_else(|| anyhow::anyhow!("no user named '{user}'"))?;
    let owner_id = owner.id;
    let is_pg = db.is_postgres();
    let pool  = db.pool().clone();

    // ── Parse all .toml stub files ──────────────────────────────────────────
    let entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "toml"))
        .collect();
    let total_files = entries.len();
    println!("Scanning {total_files} files in {} …", dir.display());

    struct Stub { pkg: StubPackage, deps_json: String }

    let mut stubs: Vec<Stub> = Vec::with_capacity(total_files);
    let mut parse_errors = 0usize;

    for entry in &entries {
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c)  => c,
            Err(e) => { tracing::warn!("read error {}: {e}", entry.path().display()); parse_errors += 1; continue; }
        };
        match toml::from_str::<StubFile>(&content) {
            Ok(sf) => {
                // Convert [dependencies] to a JSON object {"name":"version",...}.
                let deps: HashMap<String, String> = sf.dependencies.iter()
                    .map(|(k, v)| {
                        let ver = match v {
                            toml::Value::String(s) => s.clone(),
                            _ => "*".to_string(),
                        };
                        (k.clone(), ver)
                    })
                    .collect();
                let deps_json = serde_json::to_string(&deps).unwrap_or_else(|_| "{}".to_string());
                stubs.push(Stub { pkg: sf.package, deps_json });
            }
            Err(e) => {
                tracing::debug!("parse error {}: {e}", entry.path().display());
                parse_errors += 1;
            }
        }
    }

    let total = stubs.len();
    println!("Parsed {total}/{total_files} stubs ({parse_errors} skipped).");

    // ── Phase 1: batch-upsert package rows ──────────────────────────────────
    // Deduplicate by (lower(name), channel): Postgres rejects "ON CONFLICT DO UPDATE"
    // when the same target row would be touched twice in a single batch.
    // We keep one representative stub per package (the first one encountered).
    println!("Phase 1/3 — upserting packages …");
    let mut seen: HashMap<String, ()> = HashMap::with_capacity(total);
    let unique_pkgs: Vec<&Stub> = stubs.iter()
        .filter(|s| seen.insert(s.pkg.name.to_lowercase(), ()).is_none())
        .collect();
    let unique_count = unique_pkgs.len();
    println!("  {unique_count} distinct package names");

    let mut pkg_done = 0usize;
    for chunk in unique_pkgs.chunks(batch_size) {
        let n   = chunk.len();
        let sql = batch_pkg_upsert_sql(n, is_pg);
        let mut q = sqlx::query(&sql);
        for stub in chunk {
            let kw = stub.pkg.keywords.as_ref().map(|ks| ks.join(","));
            q = q
                .bind(&stub.pkg.name)
                .bind(channel)
                .bind(stub.pkg.description.as_deref())
                .bind(stub.pkg.license.as_deref())
                .bind(kw);
        }
        q.execute(&pool).await?;
        pkg_done += n;
        if pkg_done % 5000 < batch_size || pkg_done == unique_count {
            println!("  {pkg_done}/{unique_count}");
        }
    }
    println!("Package upserts done.");

    // ── Phase 2: resolve package IDs ────────────────────────────────────────
    println!("Phase 2/3 — resolving package IDs …");
    let all_lower: Vec<String> = stubs.iter().map(|s| s.pkg.name.to_lowercase()).collect();
    let mut name_to_id: HashMap<String, i64> = HashMap::with_capacity(total);

    for chunk in all_lower.chunks(batch_size) {
        let n   = chunk.len();
        let sql = batch_id_select_sql(n, is_pg);
        let mut q = sqlx::query(&sql).bind(channel);
        for name in chunk {
            q = q.bind(name.as_str());
        }
        let rows = q.fetch_all(&pool).await?;
        for row in rows {
            let id:   i64   = row.try_get("id")?;
            let name: String = row.try_get("name")?;
            name_to_id.insert(name.to_lowercase(), id);
        }
    }
    println!("Resolved {}/{total} IDs.", name_to_id.len());

    // ── Phase 3a: batch-insert versions ─────────────────────────────────────
    println!("Phase 3/3 — inserting versions and owners …");
    let mut ver_inserted = 0i64;
    let mut own_inserted = 0i64;

    for chunk in stubs.chunks(batch_size) {
        // Collect (package_id, stub) pairs for rows that resolved.
        let resolved: Vec<(i64, &Stub)> = chunk.iter()
            .filter_map(|s| {
                let id = *name_to_id.get(&s.pkg.name.to_lowercase())?;
                Some((id, s))
            })
            .collect();
        if resolved.is_empty() { continue; }

        let n = resolved.len();

        // Version insert
        let ver_sql = batch_version_insert_sql(n, is_pg);
        let mut vq = sqlx::query(&ver_sql);
        for (pkg_id, stub) in &resolved {
            vq = vq
                .bind(*pkg_id)
                .bind(&stub.pkg.version)
                .bind("")                              // checksum — empty for metadata-only
                .bind(stub.deps_json.as_str())
                .bind(stub.pkg.url.as_deref())
                .bind(stub.pkg.build.as_deref())
                .bind(stub.pkg.supports.as_deref());
        }
        let vr = vq.execute(&pool).await?;
        ver_inserted += vr.rows_affected() as i64;

        // Owner insert
        let own_sql = batch_owner_insert_sql(n, is_pg);
        let mut oq = sqlx::query(&own_sql);
        for (pkg_id, _) in &resolved {
            oq = oq.bind(*pkg_id).bind(owner_id);
        }
        let or = oq.execute(&pool).await?;
        own_inserted += or.rows_affected() as i64;
    }

    println!("Versions inserted: {ver_inserted}");
    println!("Owners assigned:   {own_inserted}");

    // ── Phase 4: set latest_version using semantic version ordering ─────────
    println!("Phase 4/4 — computing latest_version per package …");
    let updated = db.update_all_latest_versions().await?;
    println!("latest_version set for {updated} packages.");

    println!("Import complete.");
    Ok(())
}

// ── SQL builders ──────────────────────────────────────────────────────────────

fn batch_pkg_upsert_sql(n: usize, pg: bool) -> String {
    let mut s = String::from(
        "INSERT INTO packages (name, channel, description, license, keywords) VALUES "
    );
    for i in 0..n {
        if i > 0 { s.push(','); }
        if pg {
            let b = i * 5;
            s.push_str(&format!("(${},${},${},${},${}) ", b+1, b+2, b+3, b+4, b+5));
        } else {
            s.push_str("(?,?,?,?,?)");
        }
    }
    if pg {
        s.push_str(
            " ON CONFLICT (lower(name), lower(channel)) DO UPDATE SET \
             description = COALESCE(excluded.description, packages.description), \
             license     = COALESCE(excluded.license,     packages.license), \
             keywords    = COALESCE(excluded.keywords,    packages.keywords)"
        );
    } else {
        s.push_str(
            " ON CONFLICT(name, channel) DO UPDATE SET \
             description = COALESCE(excluded.description, description), \
             license     = COALESCE(excluded.license,     license), \
             keywords    = COALESCE(excluded.keywords,    keywords)"
        );
    }
    s
}

fn batch_id_select_sql(n: usize, pg: bool) -> String {
    let mut s = if pg {
        "SELECT id, name FROM packages WHERE lower(channel) = lower($1) AND lower(name) IN (".to_string()
    } else {
        "SELECT id, name FROM packages WHERE lower(channel) = lower(?) AND lower(name) IN (".to_string()
    };
    for i in 0..n {
        if i > 0 { s.push(','); }
        if pg { s.push_str(&format!("${}", i + 2)); } else { s.push('?'); }
    }
    s.push(')');
    s
}

fn batch_version_insert_sql(n: usize, pg: bool) -> String {
    let mut s = String::from(
        "INSERT INTO versions \
         (package_id, version, checksum, dependencies, upstream_url, build_system, supports) VALUES "
    );
    for i in 0..n {
        if i > 0 { s.push(','); }
        if pg {
            let b = i * 7;
            s.push_str(&format!("(${},${},${},${},${},${},${}) ", b+1, b+2, b+3, b+4, b+5, b+6, b+7));
        } else {
            s.push_str("(?,?,?,?,?,?,?)");
        }
    }
    // On conflict: update supports (allows re-importing to populate this field for existing rows).
    if pg {
        s.push_str(" ON CONFLICT (package_id, version) DO UPDATE SET supports = excluded.supports");
    } else {
        s.push_str(" ON CONFLICT (package_id, version) DO UPDATE SET supports = excluded.supports");
    }
    s
}

fn batch_owner_insert_sql(n: usize, pg: bool) -> String {
    let mut s = String::from("INSERT INTO package_owners (package_id, user_id) VALUES ");
    for i in 0..n {
        if i > 0 { s.push(','); }
        if pg {
            let b = i * 2;
            s.push_str(&format!("(${},${}) ", b+1, b+2));
        } else {
            s.push_str("(?,?)");
        }
    }
    s.push_str(" ON CONFLICT DO NOTHING");
    s
}

// ── GC ────────────────────────────────────────────────────────────────────────

async fn cmd_gc(db: &Db, data: &PathBuf, execute: bool) -> Result<()> {
    let storage = Storage::new(data.join("tarballs"));
    let yanked = db.list_yanked_versions().await?;

    if yanked.is_empty() {
        println!("No yanked versions found — nothing to collect.");
        return Ok(());
    }

    let mut freed: u64 = 0;
    for (name, version, channel) in &yanked {
        println!(
            "{} {name}@{version} ({channel})",
            if execute { "deleting" } else { "would delete" }
        );
        if execute {
            storage.delete_version(name, version).await?;
            freed += 1;
        }
    }

    if execute {
        println!("\nDeleted blobs for {freed} yanked version(s).");
    } else {
        println!(
            "\n{} yanked version(s) found. Re-run with --execute to delete their blobs.",
            yanked.len()
        );
    }
    Ok(())
}
