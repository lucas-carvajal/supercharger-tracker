mod api;
mod coming_soon;
mod db;
mod loaders;
mod raw;
mod regions;
mod sync;

use std::collections::{HashMap, HashSet};

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

        let current = db::get_current_statuses(pool).await?;
        let plan = sync::compute_sync(current, &coming_soon, &result.failed_detail_ids);

        // For disappeared chargers, check whether they have opened (gone live).
        let (open_results, open_status_failed_ids) = if plan.disappeared_ids.is_empty() {
            (HashMap::new(), HashSet::new())
        } else {
            let ids: Vec<String> = plan.disappeared_ids.iter().map(|(id, _)| id.clone()).collect();
            println!("  → Checking open status for {} disappeared charger(s)…", ids.len());
            match loaders::fetch_open_status_for_ids(&page, &ids).await {
                Ok((results, failed)) => {
                    if !failed.is_empty() {
                        eprintln!(
                            "  ⚠ Open-status check: {}/{} chargers failed — flagged for retry",
                            failed.len(), ids.len()
                        );
                    }
                    (results, failed)
                }
                Err(e) => {
                    // Total call failure: flag all disappeared chargers so none are
                    // falsely marked REMOVED.
                    eprintln!(
                        "  ✗ Open-status check failed entirely: {e} \
                         — flagging all {} disappeared charger(s) for retry",
                        ids.len()
                    );
                    (HashMap::new(), ids.into_iter().collect())
                }
            }
        };

        let mut removed_ids: Vec<String> = vec![];
        let mut removed_status_changes: Vec<db::StatusChange> = vec![];

        for (id, old_status) in &plan.disappeared_ids {
            if open_results.contains_key(id) {
                println!("  ✓ Charger {id} has opened — moving to opened_superchargers");
            } else if open_status_failed_ids.contains(id) {
                eprintln!("  ⚠ Charger {id} open-status check failed — flagging for retry");
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

        let run_id = db::record_scrape_run(
            pool,
            &country,
            coming_soon.len() as i32,
            failed_count as i32,
            open_status_failed_ids.len() as i32,
            "full",
        )
        .await?;

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
            &open_status_failed_ids,
        )
        .await?;

        println!(
            "DB update: {} new/changed, {} status changes, {} opened, {} removed, \
             {} open-check pending, {} unchanged",
            plan.upserts.len(),
            all_status_changes.len(),
            open_results.len(),
            removed_ids.len(),
            open_status_failed_ids.len(),
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
    if stats.open_status_check_failed > 0 {
        println!(
            "  ({} with failed open-status check — run retry-failed to resolve)",
            stats.open_status_check_failed
        );
    }

    Ok(())
}

async fn run_retry_failed(
    pool: &sqlx::PgPool,
    show_browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let failed_detail_chargers = db::get_failed_detail_chargers(pool).await?;
    let failed_open_chargers = db::get_failed_open_status_chargers(pool).await?;

    if failed_detail_chargers.is_empty() && failed_open_chargers.is_empty() {
        println!("No chargers with failed detail fetches or open-status checks. Nothing to retry.");
        return Ok(());
    }

    let detail_total = failed_detail_chargers.len();
    let open_total = failed_open_chargers.len();

    if detail_total > 0 {
        println!("Retrying details for {detail_total} charger(s)…");
    }
    if open_total > 0 {
        println!("Retrying open-status checks for {open_total} charger(s)…");
    }

    // Single browser launch — one Akamai wait covers both retry phases.
    let (mut browser, page) = loaders::launch_browser_and_wait(show_browser).await?;

    // ── Phase 1: Retry detail fetches ────────────────────────────────────────
    let (plan, still_detail_failed) = if !failed_detail_chargers.is_empty() {
        let ids: Vec<String> = failed_detail_chargers.iter().map(|c| c.id.clone()).collect();
        let (details_map, still_failed) =
            loaders::fetch_batch_details_from_page(&page, ids).await;

        let updated: Vec<ComingSoonSupercharger> = failed_detail_chargers
            .iter()
            .map(|c| c.clone().with_details(details_map.get(&c.id)))
            .collect();

        let current_map: HashMap<String, _> = failed_detail_chargers
            .iter()
            .map(|c| (c.id.clone(), c.status.clone()))
            .collect();
        let plan = sync::compute_sync(current_map, &updated, &still_failed);
        (plan, still_failed)
    } else {
        (sync::compute_sync(HashMap::new(), &[], &HashSet::new()), HashSet::new())
    };

    // ── Phase 2: Retry open-status checks ────────────────────────────────────
    let (open_results, still_open_failed, os_removed_ids, os_removed_changes) =
        if !failed_open_chargers.is_empty() {
            let ids: Vec<String> = failed_open_chargers.iter().map(|c| c.id.clone()).collect();
            let (open_results, still_failed) =
                loaders::fetch_open_status_for_ids(&page, &ids).await?;

            let mut removed_ids: Vec<String> = vec![];
            let mut removed_changes: Vec<db::StatusChange> = vec![];

            for charger in &failed_open_chargers {
                if open_results.contains_key(&charger.id) {
                    println!("  ✓ Charger {} has opened — moving to opened_superchargers", charger.id);
                } else if still_failed.contains(&charger.id) {
                    eprintln!("  ⚠ Charger {} open-status check still failing — keeping flag", charger.id);
                } else {
                    eprintln!("  ⚠ Charger {} confirmed absent — marking as removed", charger.id);
                    removed_ids.push(charger.id.clone());
                    removed_changes.push(db::StatusChange {
                        supercharger_id: charger.id.clone(),
                        old_status: Some(charger.status.clone()),
                        new_status: coming_soon::SiteStatus::Removed,
                    });
                }
            }

            (open_results, still_failed, removed_ids, removed_changes)
        } else {
            (HashMap::new(), HashSet::new(), vec![], vec![])
        };

    browser.close().await.ok();

    // ── Record and save ───────────────────────────────────────────────────────
    let run_id = db::record_scrape_run(
        pool,
        "N/A",
        (detail_total + open_total) as i32,
        still_detail_failed.len() as i32,
        still_open_failed.len() as i32,
        "retry",
    )
    .await?;

    let mut all_status_changes = plan.status_changes;
    all_status_changes.extend(os_removed_changes);

    db::save_chargers(
        pool,
        &plan.upserts,
        &plan.unchanged,
        &all_status_changes,
        &os_removed_ids,
        &open_results,
        run_id,
        &still_detail_failed,
        &still_open_failed,
    )
    .await?;

    let detail_resolved = detail_total - still_detail_failed.len();
    let open_resolved = open_total - still_open_failed.len();
    println!(
        "Retry complete: {} detail(s) resolved ({} still failing), \
         {} open-status check(s) resolved ({} still failing), \
         {} status changes",
        detail_resolved,
        still_detail_failed.len(),
        open_resolved,
        still_open_failed.len(),
        all_status_changes.len(),
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
