//! freight-registry — self-hosted package registry server.
//!
//! # Usage
//!
//!   freight-registry --data /var/lib/freight-registry serve
//!   freight-registry --data /var/lib/freight-registry token add ci-bot
//!   freight-registry --data /var/lib/freight-registry token list
//!   freight-registry --data /var/lib/freight-registry token revoke ci-bot

use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod auth;
mod db;
mod storage;

use db::Db;
use storage::Storage;

pub struct AppState {
    pub db: Db,
    pub storage: Storage,
    pub base_url: String,
}

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
        /// Address to bind to
        #[arg(long, env = "FREIGHT_BIND", default_value = "0.0.0.0:7878")]
        bind: String,
        /// Publicly reachable base URL, embedded in download links
        #[arg(long, env = "FREIGHT_BASE_URL", default_value = "http://localhost:7878")]
        base_url: String,
    },
    /// Manage API tokens
    Token {
        #[command(subcommand)]
        command: TokenCmd,
    },
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Create a new API token and print it (shown only once)
    Add { name: String },
    /// List all token names
    List,
    /// Revoke a token by name
    Revoke { name: String },
}

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
        Command::Serve { bind, base_url } => {
            let state = Arc::new(AppState {
                db,
                storage: Storage::new(cli.data.join("tarballs")),
                base_url: base_url.trim_end_matches('/').to_string(),
            });
            let app = api::router(state);
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!("listening on {bind}");
            axum::serve(listener, app).await?;
        }

        Command::Token { command } => match command {
            TokenCmd::Add { name } => {
                let token = db.create_token(&name).await?;
                println!("Token '{name}' created:\n\n  {token}\n");
                println!("Store this value — it will not be shown again.");
            }
            TokenCmd::List => {
                let tokens = db.list_tokens().await?;
                if tokens.is_empty() {
                    println!("no tokens");
                } else {
                    println!("{:<6}  name", "id");
                    println!("{}", "-".repeat(30));
                    for t in tokens {
                        println!("{:<6}  {}", t.id, t.name);
                    }
                }
            }
            TokenCmd::Revoke { name } => {
                if db.revoke_token(&name).await? {
                    println!("revoked '{name}'");
                } else {
                    anyhow::bail!("no token named '{name}'");
                }
            }
        },
    }

    Ok(())
}
