use std::path::Path;

use crate::export::{DiffExport, ScrapeExport, SnapshotExport};
use crate::repository::{ScrapeRunRepository, SuperchargerRepository};

pub enum ImportOutcome {
    Applied { run_id: i64, changed: usize, opened: usize, removed: usize },
    Duplicate { run_id: i64 },
    OutOfOrder { expected: i64, got: i64 },
    SnapshotApplied { source_run_id: i64, scrape_runs: usize, chargers: usize, opened: usize },
}

pub async fn run_import(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    path: &Path,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let export: ScrapeExport = serde_json::from_slice(&bytes)?;

    let outcome = apply_import(supercharger_repo, scrape_run_repo, export, force).await?;

    match outcome {
        ImportOutcome::Applied { run_id, changed, opened, removed } => {
            println!("Applied import: run_id = {run_id}, {changed} changed, {opened} opened, {removed} removed");
        }
        ImportOutcome::Duplicate { run_id } => {
            println!("Already imported: source_run_id = {run_id}");
        }
        ImportOutcome::OutOfOrder { expected, got } => {
            return Err(format!(
                "out-of-order import: expected run_id {expected}, got {got}. \
                 Use --force to override.",
            ).into());
        }
        ImportOutcome::SnapshotApplied { source_run_id, scrape_runs, chargers, opened } => {
            println!(
                "Snapshot applied: source_run_id = {source_run_id}, {scrape_runs} scrape_runs, \
                 {chargers} coming-soon, {opened} opened",
            );
        }
    }

    Ok(())
}

/// Apply an import, returning the outcome without printing. Used by both the CLI
/// and HTTP handlers.
pub async fn apply_import(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    export: ScrapeExport,
    force: bool,
) -> Result<ImportOutcome, Box<dyn std::error::Error>> {
    match export {
        ScrapeExport::Diff(diff) => apply_diff(supercharger_repo, scrape_run_repo, diff, force).await,
        ScrapeExport::Snapshot(snap) => apply_snapshot(supercharger_repo, scrape_run_repo, snap).await,
    }
}

async fn apply_diff(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    diff: DiffExport,
    force: bool,
) -> Result<ImportOutcome, Box<dyn std::error::Error>> {
    // 1. Dedup
    if scrape_run_repo.source_run_id_exists(diff.run_id).await? {
        return Ok(ImportOutcome::Duplicate { run_id: diff.run_id });
    }

    // 2. Ordering
    if !force {
        let max_source = scrape_run_repo.get_max_source_run_id().await?.unwrap_or(0);
        let expected = max_source + 1;
        if diff.run_id != expected {
            return Ok(ImportOutcome::OutOfOrder { expected, got: diff.run_id });
        }
    }

    // 3. Record a prod scrape_runs row so status_changes have a parent, with
    //    source_run_id = diff.run_id for ordering/dedup.
    let prod_run_id = scrape_run_repo.record_import_run(
        &diff.country,
        diff.scraped_at,
        diff.run_id,
        "import",
    ).await?;

    let changed = diff.changed_chargers.len();
    let opened = diff.opened_chargers.len();
    let removed = diff.removed_ids.len();

    supercharger_repo.save_chargers_from_diff(&diff, prod_run_id).await?;

    Ok(ImportOutcome::Applied { run_id: diff.run_id, changed, opened, removed })
}

async fn apply_snapshot(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    snap: SnapshotExport,
) -> Result<ImportOutcome, Box<dyn std::error::Error>> {
    let source_run_id = snap.source_run_id;
    let scrape_runs = snap.scrape_runs.len();
    let chargers = snap.coming_soon_superchargers.len();
    let opened = snap.opened_superchargers.len();

    supercharger_repo.apply_snapshot(&snap).await?;

    // Seed row anchoring the ordering chain: after snapshot, the first diff must
    // have run_id == source_run_id + 1.
    scrape_run_repo.record_import_run(
        "snapshot",
        chrono::Utc::now(),
        source_run_id,
        "snapshot-seed",
    ).await?;

    Ok(ImportOutcome::SnapshotApplied { source_run_id, scrape_runs, chargers, opened })
}
