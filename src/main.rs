mod api;
mod application;
mod domain;
mod repository;
mod scraper;
mod util;

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
        /// Port to listen on (default: 8080).
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let args = Args::parse();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = repository::connect(&database_url).await?;

    let supercharger_repo = repository::SuperchargerRepository::new(pool.clone());
    let scrape_run_repo = repository::ScrapeRunRepository::new(pool.clone());

    match args.command {
        Command::Scrape { country, show_browser } => {
            application::scrape::run_scrape(&supercharger_repo, &scrape_run_repo, country, show_browser).await?;
        }
        Command::Status => {
            application::status::run_status(&supercharger_repo, &scrape_run_repo).await?;
        }
        Command::RetryFailed { show_browser } => {
            application::retry::run_retry_failed(&supercharger_repo, &scrape_run_repo, show_browser).await?;
        }
        Command::Host { port } => {
            run_host(pool, port).await?;
        }
    }

    Ok(())
}

async fn run_host(pool: sqlx::PgPool, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let router = api::router(pool);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("API server listening on http://{addr}");
    axum::serve(listener, router).await?;
    Ok(())
}
