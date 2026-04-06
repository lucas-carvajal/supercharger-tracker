use std::io::Write;
use std::path::{Path, PathBuf};

use crate::export::{DiffExport, ScrapeExport};
use crate::repository::{ScrapeRunRepository, SuperchargerRepository};

pub async fn run_export_diff(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    file: Option<PathBuf>,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let latest = scrape_run_repo.get_latest_run().await?
        .ok_or("No scrape runs found — run `scrape` first")?;

    if !force && (latest.details_failures > 0 || latest.open_status_failures > 0) {
        return Err(format!(
            "scrape incomplete — run {} still has {} detail failure(s) and {} open-status failure(s). \
             Run `retry-failed` first (or use --force to export anyway).",
            latest.id, latest.details_failures, latest.open_status_failures,
        ).into());
    }

    let (status_changes, changed_chargers, opened_chargers) = tokio::try_join!(
        supercharger_repo.get_status_changes_since_run(latest.id),
        supercharger_repo.get_changed_chargers_since_run(latest.id),
        supercharger_repo.get_opened_chargers_since_run(latest.id),
    )?;

    let removed_ids: Vec<String> = status_changes
        .iter()
        .filter(|sc| matches!(sc.new_status, crate::domain::SiteStatus::Removed))
        .map(|sc| sc.supercharger_id.clone())
        .collect();

    let changed_count = changed_chargers.len();
    let status_changes_count = status_changes.len();
    let opened_count = opened_chargers.len();
    let removed_count = removed_ids.len();

    let diff = DiffExport {
        run_id: latest.id,
        scraped_at: latest.scraped_at,
        country: latest.country.clone(),
        changed_chargers,
        status_changes,
        opened_chargers,
        removed_ids,
    };

    let path = file.unwrap_or_else(|| PathBuf::from(format!("scrape_export_{}.json", latest.id)));
    write_atomically(&path, &ScrapeExport::Diff(diff))?;

    println!(
        "Wrote {}: {} changed chargers, {} status changes, {} opened, {} removed",
        path.display(), changed_count, status_changes_count, opened_count, removed_count,
    );

    Ok(())
}

pub(crate) fn write_atomically(path: &Path, export: &ScrapeExport) -> Result<(), Box<dyn std::error::Error>> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        serde_json::to_writer_pretty(&mut f, export)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}
