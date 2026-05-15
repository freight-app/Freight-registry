mod app;
mod client;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use app::{App, DataEvent};
use client::Client;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "freight-registry-tui", about = "Terminal UI for a freight registry")]
struct Args {
    /// Registry base URL
    #[arg(long, env = "FREIGHT_REGISTRY_URL", default_value = "http://localhost:7878")]
    url: String,

    /// API token (omit to use the login screen)
    #[arg(long, env = "FREIGHT_REGISTRY_TOKEN")]
    token: Option<String>,
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let client = Client::new(args.url.clone(), args.token.clone());
    let mut app = App::new(client, args.url);

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend  = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    let result = run(&mut term, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;

    if let Err(ref e) = result {
        eprintln!("Error: {e:#}");
    }
    result
}

// ── Event loop ────────────────────────────────────────────────────────────────

async fn run(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app:  &mut App,
) -> Result<()> {
    let (data_tx, mut data_rx) = mpsc::channel::<DataEvent>(64);
    let (key_tx,  mut key_rx)  = mpsc::channel::<crossterm::event::KeyEvent>(32);

    // Spawn blocking key-reader in a separate OS thread so we don't stall tokio.
    let key_tx2 = key_tx.clone();
    tokio::task::spawn_blocking(move || {
        loop {
            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                if let Ok(Event::Key(k)) = event::read() {
                    if key_tx2.blocking_send(k).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Initial load: fetch /me + first tab data.
    app.load_me(data_tx.clone());
    app.load_current_tab(data_tx.clone());

    loop {
        term.draw(|f| ui::draw(f, app))?;

        tokio::select! {
            key = key_rx.recv() => {
                if let Some(k) = key {
                    if app.handle_key(k, &data_tx) { break; }
                }
            }
            data = data_rx.recv() => {
                if let Some(d) = data { app.handle_data(d); }
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                // periodic redraw tick (updates spinner, relative timestamps)
            }
        }
    }

    Ok(())
}
