use std::path::PathBuf;

use crate::export::{ScrapeExport, SnapshotExport};
use crate::repository::{ScrapeRunRepository, SuperchargerRepository};

pub async fn run_export_snapshot(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    file: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_run_id = scrape_run_repo.get_last_run_id().await?
        .ok_or("No scrape runs found — run `scrape` first")?;

    let (scrape_runs, coming_soon_superchargers, opened_superchargers, status_changes) = tokio::try_join!(
        scrape_run_repo.get_all_scrape_runs(),
        supercharger_repo.get_all_coming_soon(),
        supercharger_repo.get_all_opened(),
        supercharger_repo.get_all_status_changes(),
    )?;

    let snap = SnapshotExport {
        source_run_id,
        scrape_runs,
        coming_soon_superchargers,
        opened_superchargers,
        status_changes,
    };

    let cs = snap.coming_soon_superchargers.len();
    let op = snap.opened_superchargers.len();
    let sc = snap.status_changes.len();
    let sr = snap.scrape_runs.len();

    super::export_diff::write_atomically(&file, &ScrapeExport::Snapshot(snap))?;

    println!(
        "Wrote snapshot {}: {} scrape_runs, {} coming-soon, {} opened, {} status_changes (source_run_id = {})",
        file.display(), sr, cs, op, sc, source_run_id,
    );

    Ok(())
}
