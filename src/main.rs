mod api;
mod coming_soon;
mod db;
mod loaders;
mod raw;
mod regions;
mod sync;

use std::collections::HashMap;

use clap::{Parser, Subcommand};

use coming_soon::ComingSoonSupercharger;

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
    let pool = db::connect(&database_url).await?;

    match args.command {
        Command::Scrape {
            country,
            show_browser,
        } => {
            run_scrape(&pool, country, show_browser).await?;
        }
        Command::Status => {
            run_status(&pool).await?;
        }
        Command::RetryFailed {
            show_browser,
        } => {
            run_retry_failed(&pool, show_browser).await?;
        }
        Command::Host { port } => {
            run_host(pool, port).await?;
        }
    }

    Ok(())
}

// ── Subcommand handlers ───────────────────────────────────────────────────────

async fn run_scrape(
    pool: &sqlx::PgPool,
    country: String,
    show_browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut browser, page) = loaders::launch_browser_and_wait(show_browser).await?;

    let scrape_result = async {
        let result = loaders::load_from_browser(&country, &page).await?;

        let failed_count = result.failed_detail_ids.len();
        if failed_count > 0 {
            let total_with_ids = result
                .locations
                .iter()
                .filter(|l| ComingSoonSupercharger::is_coming_soon(l))
                .filter(|l| l.location_url_slug != "null" && !l.location_url_slug.is_empty())
                .count();
            let pct = failed_count * 100 / total_with_ids.max(1);
            eprintln!(
                "  ⚠ Details fetch: {failed_count}/{total_with_ids} chargers failed ({pct}%) \
                 — existing statuses preserved for those chargers"
            );
            if pct > 50 {
                eprintln!("  ⚠ High failure rate — check for Akamai blocking or API issues");
            }
        }

        let coming_soon: Vec<ComingSoonSupercharger> = result
            .locations
            .iter()
            .filter(|l| ComingSoonSupercharger::is_coming_soon(l))
            .filter_map(|l| {
                let details = result.coming_soon_details.get(&l.location_url_slug);
                ComingSoonSupercharger::from_location(l, details)
            })
            .collect();

        let run_id = db::record_scrape_run(
            pool,
            &country,
            coming_soon.len() as i32,
            failed_count as i32,
            "full",
        )
        .await?;
        let current = db::get_current_statuses(pool).await?;
        let plan = sync::compute_sync(current, &coming_soon, &result.failed_detail_ids);

        // For disappeared chargers, check whether they have opened (gone live).
        let open_results = if plan.disappeared_ids.is_empty() {
            HashMap::new()
        } else {
            let ids: Vec<String> = plan.disappeared_ids.iter().map(|(id, _)| id.clone()).collect();
            println!("  → Checking open status for {} disappeared charger(s)…", ids.len());
            loaders::fetch_open_status_for_ids(&page, &ids).await.unwrap_or_default()
        };

        let mut removed_ids: Vec<String> = vec![];
        let mut removed_status_changes: Vec<db::StatusChange> = vec![];

        for (id, old_status) in &plan.disappeared_ids {
            if open_results.contains_key(id) {
                println!("  ✓ Charger {id} has opened — moving to opened_superchargers");
            } else {
                eprintln!("  ⚠ Disappeared charger {id} not found in Tesla API — marking as removed");
                removed_ids.push(id.clone());
                removed_status_changes.push(db::StatusChange {
                    supercharger_id: id.clone(),
                    old_status: Some(old_status.clone()),
                    new_status: coming_soon::SiteStatus::Removed,
                });
            }
        }

        let mut all_status_changes = plan.status_changes;
        all_status_changes.extend(removed_status_changes);

        db::save_chargers(
            pool,
            &plan.upserts,
            &plan.unchanged,
            &all_status_changes,
            &removed_ids,
            &open_results,
            run_id,
            &result.failed_detail_ids,
        )
        .await?;

        println!(
            "DB update: {} new/changed, {} status changes, {} opened, {} removed, {} unchanged",
            plan.upserts.len(),
            all_status_changes.len(),
            open_results.len(),
            removed_ids.len(),
            plan.unchanged.len(),
        );

        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;

    browser.close().await.ok();
    scrape_result
}

async fn run_status(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let run = db::get_last_run_stats(pool).await?;
    let stats = db::get_current_db_stats(pool).await?;

    match run {
        None => println!("No runs recorded yet."),
        Some(r) => {
            println!(
                "Last run #{} ({}) — {}",
                r.id,
                r.run_type,
                r.scraped_at.format("%Y-%m-%d %H:%M UTC")
            );
            println!(
                "  Scraped: {}  |  Detail failures: {}  |  Status changes: {}",
                r.total_count, r.details_failures, r.status_changes_count
            );
        }
    }

    println!();
    println!("Active chargers: {}", stats.active);
    println!("  In Development:     {}", stats.in_development);
    println!("  Under Construction: {}", stats.under_construction);
    if stats.unknown > 0 {
        println!("  Unknown:            {}", stats.unknown);
    }
    if stats.details_failed > 0 {
        println!(
            "  ({} with failed detail fetch — run retry-failed to resolve)",
            stats.details_failed
        );
    }

    Ok(())
}

async fn run_retry_failed(
    pool: &sqlx::PgPool,
    show_browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let failed_chargers = db::get_failed_detail_chargers(pool).await?;

    if failed_chargers.is_empty() {
        println!("No chargers with failed detail fetches. Nothing to retry.");
        return Ok(());
    }

    let total = failed_chargers.len();
    println!("Retrying details for {total} chargers…");

    let ids: Vec<String> = failed_chargers.iter().map(|c| c.id.clone()).collect();

    let (details_map, still_failed) =
        loaders::fetch_details_only_browser(ids, show_browser).await?;

    // Apply new details to each charger.
    let updated: Vec<ComingSoonSupercharger> = failed_chargers
        .iter()
        .map(|c| {
            let new_details = details_map.get(&c.id);
            c.clone().with_details(new_details)
        })
        .collect();

    // Run a partial sync against only the retried chargers.
    // disappeared_ids will be empty since we supply all retried chargers in `updated`.
    let current_map: HashMap<String, _> = failed_chargers
        .iter()
        .map(|c| (c.id.clone(), c.status.clone()))
        .collect();
    let plan = sync::compute_sync(current_map, &updated, &still_failed);

    let run_id = db::record_scrape_run(
        pool,
        "N/A",
        total as i32,
        still_failed.len() as i32,
        "retry",
    )
    .await?;

    // Pass empty removed_ids and open_results — retry-failed only updates details.
    db::save_chargers(
        pool,
        &plan.upserts,
        &plan.unchanged,
        &plan.status_changes,
        &[],
        &HashMap::new(),
        run_id,
        &still_failed,
    )
    .await?;

    let resolved = total - still_failed.len();
    println!(
        "Retry complete: {} resolved, {} still failing, {} status changes",
        resolved,
        still_failed.len(),
        plan.status_changes.len(),
    );

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
