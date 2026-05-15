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

mod api;
mod auth;
mod db;
mod rate_limit;
mod storage;
mod totp;
mod validate;

use auth::hash_password;
use db::Db;
use rate_limit::Limiters;
use storage::Storage;

pub struct AppState {
    pub db:       Db,
    pub storage:  Storage,
    pub base_url: String,
    pub limiters: Limiters,
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "freight-registry", about = "Self-hosted freight package registry")]
struct Cli {
    /// Directory for the database and stored tarballs
    #[arg(long, env = "FREIGHT_DATA_DIR", default_value = "/var/lib/freight-registry")]
    data: PathBuf,

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
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "freight_registry=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.data)?;

    let db = Db::open(&cli.data.join("registry.db")).await?;

    match cli.command {
        Command::Serve { bind, base_url, max_upload_mb, audit_log_ttl_days, rate_limit_read, rate_limit_write } => {
            let state = Arc::new(AppState {
                db,
                storage:  Storage::new(cli.data.join("tarballs")),
                base_url: base_url.trim_end_matches('/').to_string(),
                limiters: Limiters::new(rate_limit_read, rate_limit_write),
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
                let token = db.create_token(u.id, &name, expires, "api").await?;
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
