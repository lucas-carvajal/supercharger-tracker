use std::path::PathBuf;

use sqlx::Row;

use crate::export::{
    ExportChangedCharger, ExportOpenedCharger, ExportScrapeRun, ExportSnapshotStatusChange,
    ScrapeExport, SnapshotExport,
};
use crate::repository::{ScrapeRunRepository, SuperchargerRepository};

pub async fn run_export_snapshot(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    file: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    // All four reads share a REPEATABLE READ transaction so the snapshot is
    // consistent — no scrape/retry can commit between queries and produce a
    // partially-updated view.
    let pool = supercharger_repo.pool();
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;

    let scrape_runs: Vec<ExportScrapeRun> = sqlx::query(
        "SELECT id, country, scraped_at, total_count, details_failures, \
                open_status_failures, retry_count, last_retry_at, run_type \
         FROM scrape_runs ORDER BY id ASC",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| ExportScrapeRun {
        id: r.get("id"),
        country: r.get("country"),
        scraped_at: r.get("scraped_at"),
        total_count: r.get("total_count"),
        details_failures: r.get("details_failures"),
        open_status_failures: r.get("open_status_failures"),
        retry_count: r.get("retry_count"),
        last_retry_at: r.get("last_retry_at"),
        run_type: r.get("run_type"),
    })
    .collect();

    let coming_soon_superchargers: Vec<ExportChangedCharger> = sqlx::query(
        "SELECT id, title, city, region, latitude, longitude, status, raw_status_value, \
                charger_category, first_seen_at, last_scraped_at \
         FROM coming_soon_superchargers",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(row_to_export_changed)
    .collect();

    let opened_superchargers: Vec<ExportOpenedCharger> = sqlx::query(
        "SELECT id, title, city, region, latitude, longitude, opening_date, num_stalls, \
                open_to_non_tesla \
         FROM opened_superchargers",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| ExportOpenedCharger {
        id: r.get("id"),
        title: r.get("title"),
        city: r.get("city"),
        region: r.get("region"),
        latitude: r.get("latitude"),
        longitude: r.get("longitude"),
        opening_date: r.get("opening_date"),
        num_stalls: r.get("num_stalls"),
        open_to_non_tesla: r.get("open_to_non_tesla"),
    })
    .collect();

    let status_changes: Vec<ExportSnapshotStatusChange> = sqlx::query(
        "SELECT supercharger_id, scrape_run_id, old_status, new_status, changed_at \
         FROM status_changes ORDER BY id ASC",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| ExportSnapshotStatusChange {
        supercharger_id: r.get("supercharger_id"),
        scrape_run_id: r.get("scrape_run_id"),
        old_status: r.get("old_status"),
        new_status: r.get("new_status"),
        changed_at: r.get("changed_at"),
    })
    .collect();

    // Read-only transaction — no commit needed.
    drop(tx);

    let _ = scrape_run_repo; // pool came from supercharger_repo; scrape_run_repo not needed here

    let sr = scrape_runs.len();
    let cs = coming_soon_superchargers.len();
    let op = opened_superchargers.len();
    let sc = status_changes.len();
    let max_run_id = scrape_runs.iter().map(|r| r.id).max().unwrap_or(0);

    let snap = SnapshotExport {
        scrape_runs,
        coming_soon_superchargers,
        opened_superchargers,
        status_changes,
    };

    super::export_diff::write_atomically(&file, &ScrapeExport::Snapshot(snap))?;

    println!(
        "Wrote snapshot {}: {} scrape_runs, {} coming-soon, {} opened, {} status_changes \
         (ordering anchor: run_id {})",
        file.display(), sr, cs, op, sc, max_run_id,
    );

    Ok(())
}

fn row_to_export_changed(r: sqlx::postgres::PgRow) -> ExportChangedCharger {
    ExportChangedCharger {
        id: r.get("id"),
        title: r.get("title"),
        city: r.get("city"),
        region: r.get("region"),
        latitude: r.get("latitude"),
        longitude: r.get("longitude"),
        status: r.get("status"),
        raw_status_value: r.get("raw_status_value"),
        charger_category: r.get("charger_category"),
        first_seen_at: r.get("first_seen_at"),
        last_scraped_at: r.get("last_scraped_at"),
    }
}
