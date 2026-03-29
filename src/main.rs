mod coming_soon;
mod db;
mod loaders;
mod raw;
mod sync;

use clap::Parser;

use coming_soon::ComingSoonSupercharger;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "tesla-superchargers",
    version,
    about = "Fetch Tesla Supercharger locations"
)]
struct Args {
    /// Read from a local JSON file instead of fetching live data.
    #[arg(short, long, value_name = "PATH")]
    file: Option<String>,

    /// Use a raw cookie string instead of launching a browser.
    /// Can also be set via TESLA_COOKIE env var.
    #[arg(short, long, value_name = "COOKIE_STRING", env = "TESLA_COOKIE")]
    cookie: Option<String>,

    /// Country code (default: US — actually returns worldwide data).
    #[arg(long, default_value = "US")]
    country: String,

    /// Show the browser window while fetching (default: headless).
    #[arg(long)]
    show_browser: bool,
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let args = Args::parse();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = db::connect(&database_url).await?;

    let result = if let Some(ref path) = args.file {
        loaders::load_from_file(path).await?
    } else if let Some(ref cookie) = args.cookie {
        loaders::load_with_cookie(&args.country, cookie).await?
    } else {
        loaders::load_from_browser(&args.country, args.show_browser).await?
    };

    let failed_count = result.details_fetch_failed_slugs.len();
    if failed_count > 0 {
        let total_with_slugs = result
            .locations
            .iter()
            .filter(|l| ComingSoonSupercharger::is_coming_soon(l))
            .filter(|l| l.location_url_slug != "null" && !l.location_url_slug.is_empty())
            .count();
        let pct = failed_count * 100 / total_with_slugs.max(1);
        eprintln!(
            "  ⚠ Details fetch: {failed_count}/{total_with_slugs} slugs failed ({pct}%) \
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
        .map(|l| {
            let details = result.coming_soon_details.get(&l.location_url_slug);
            ComingSoonSupercharger::from_location(l, details)
        })
        .collect();

    let run_id = db::record_scrape_run(
        &pool,
        &args.country,
        coming_soon.len() as i32,
        failed_count as i32,
    )
    .await?;
    let current = db::get_current_statuses(&pool).await?;
    let plan = sync::compute_sync(current, &coming_soon, &result.details_fetch_failed_slugs);
    db::save_chargers(
        &pool,
        &plan.upserts,
        &plan.unchanged_uuids,
        &plan.status_changes,
        &plan.disappeared_uuids,
        run_id,
    )
    .await?;

    println!(
        "DB update: {} new/changed, {} status changes, {} disappeared, {} unchanged",
        plan.upserts.len(),
        plan.status_changes.len(),
        plan.disappeared_uuids.len(),
        plan.unchanged_uuids.len(),
    );

    Ok(())
}
