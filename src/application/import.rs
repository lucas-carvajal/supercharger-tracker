use crate::export::{DiffExport, ScrapeExport, SnapshotExport};
use crate::repository::{ScrapeRunRepository, SuperchargerRepository};

pub enum ImportOutcome {
    Applied { run_id: i64, changed: usize, opened: usize, removed: usize },
    Duplicate { run_id: i64 },
    OutOfOrder { expected: i64, got: i64 },
    SnapshotApplied { source_run_id: i64, scrape_runs: usize, chargers: usize, opened: usize },
}

/// Apply an import, returning the outcome. Used by the HTTP handler.
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
    // 1. Dedup — id is preserved from local, so a duplicate import would conflict.
    if scrape_run_repo.run_id_exists(diff.run_id).await? {
        return Ok(ImportOutcome::Duplicate { run_id: diff.run_id });
    }

    // 2. Ordering — next import must be exactly MAX(id) + 1.
    if !force {
        let max_id = scrape_run_repo.get_max_run_id().await?.unwrap_or(0);
        let expected = max_id + 1;
        if diff.run_id != expected {
            return Ok(ImportOutcome::OutOfOrder { expected, got: diff.run_id });
        }
    }

    let changed = diff.changed_chargers.len();
    let opened = diff.opened_chargers.len();
    let removed = diff.removed_ids.len();

    // Insert scrape_runs row + all charger changes atomically.
    supercharger_repo.save_chargers_from_diff(&diff).await?;

    Ok(ImportOutcome::Applied { run_id: diff.run_id, changed, opened, removed })
}

async fn apply_snapshot(
    supercharger_repo: &SuperchargerRepository,
    _scrape_run_repo: &ScrapeRunRepository,
    snap: SnapshotExport,
) -> Result<ImportOutcome, Box<dyn std::error::Error>> {
    let source_run_id = snap.source_run_id;
    let scrape_runs = snap.scrape_runs.len();
    let chargers = snap.coming_soon_superchargers.len();
    let opened = snap.opened_superchargers.len();

    // Restores all four tables including scrape_runs with original ids, then resets
    // the sequence. The first diff must have run_id == MAX(restored id) + 1.
    supercharger_repo.apply_snapshot(&snap).await?;

    Ok(ImportOutcome::SnapshotApplied { source_run_id, scrape_runs, chargers, opened })
}
