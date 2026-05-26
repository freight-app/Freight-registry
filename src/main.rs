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

use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use freight_registry::{
    api,
    auth::hash_password,
    config,
    db::Db,
    metrics::Metrics,
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
    if let Some(path) = config::load() {
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
            mirror_upstream, max_packages_per_user,
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

            let state = Arc::new(AppState {
                db,
                storage,
                base_url:             base_url.trim_end_matches('/').to_string(),
                limiters:             Limiters::new(rate_limit_read, rate_limit_write),
                metrics:              Metrics::new(),
                mirror_upstream:      mirror_upstream.map(|u| u.trim_end_matches('/').to_string()),
                max_packages_per_user,
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
