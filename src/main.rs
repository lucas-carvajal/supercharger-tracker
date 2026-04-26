mod api;
mod application;
mod domain;
mod export;
mod repository;
mod scraper;
mod util;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "tesla-superchargers",
    version,
    about = "Fetch and track Tesla coming-soon Supercharger locations"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Fetch all coming-soon supercharger locations and their details, then update the DB.
    Scrape {
        /// Country code (default: US — actually returns worldwide data).
        #[arg(long, default_value = "US")]
        country: String,

        /// Show the browser window while fetching (default: headless).
        #[arg(long)]
        show_browser: bool,
    },

    /// Show a summary of the last scrape run and current DB state.
    Status,

    /// Re-fetch details only for chargers where the last details fetch failed.
    /// Skips the full locations download and only hits the details endpoint.
    RetryFailed {
        /// Show the browser window while fetching (default: headless).
        #[arg(long)]
        show_browser: bool,
    },

    /// Start the HTTP API server.
    Host {
        /// Port to listen on (default: 8080, or `PORT` env var if set).
        #[arg(short, long, env = "PORT", default_value = "8080")]
        port: u16,
    },

    /// Write a diff export file for the latest scrape run.
    /// Errors if the scrape still has unresolved failures (unless --force).
    ExportDiff {
        /// Output file path. Defaults to `scrape_export_{run_id}.json` in CWD.
        #[arg(long)]
        file: Option<PathBuf>,

        /// Export even if the scrape is incomplete.
        #[arg(long)]
        force: bool,
    },

    /// Write a full snapshot of the local DB for initial prod setup or recovery.
    ExportSnapshot {
        /// Output file path.
        #[arg(long)]
        file: PathBuf,
    },
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    init_tracing();
    let args = Args::parse();
    let config = util::config::Config::from_env();

    let pool = repository::connect(&config.database_url, config.db_max_connections)
        .await
        .map_err(|err| {
            std::io::Error::other(format!(
                "failed to connect to Postgres using DATABASE_URL: {err}"
            ))
        })?;

    let supercharger_repo = repository::SuperchargerRepository::new(pool.clone());
    let scrape_run_repo = repository::ScrapeRunRepository::new(pool.clone());

    match args.command {
        Command::Scrape {
            country,
            show_browser,
        } => {
            application::scrape::run_scrape(
                &supercharger_repo,
                &scrape_run_repo,
                country,
                show_browser,
            )
            .await?;
        }
        Command::Status => {
            application::status::run_status(&supercharger_repo, &scrape_run_repo).await?;
        }
        Command::RetryFailed { show_browser } => {
            application::retry::run_retry_failed(
                &supercharger_repo,
                &scrape_run_repo,
                show_browser,
            )
            .await?;
        }
        Command::Host { port } => {
            run_host(pool, config, port).await?;
        }
        Command::ExportDiff { file, force } => {
            application::export_diff::run_export_diff(
                &supercharger_repo,
                &scrape_run_repo,
                file,
                force,
            )
            .await?;
        }
        Command::ExportSnapshot { file } => {
            application::export_snapshot::run_export_snapshot(
                &supercharger_repo,
                &scrape_run_repo,
                file,
            )
            .await?;
        }
    }

    Ok(())
}

async fn run_host(
    pool: sqlx::PgPool,
    config: util::config::Config,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let router = api::router(pool, config);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|err| {
        std::io::Error::other(format!("failed to bind API server to {addr}: {err}"))
    })?;
    tracing::info!(addr = %addr, "API server listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, draining connections");
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();
}
