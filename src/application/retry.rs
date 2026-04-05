use std::collections::{HashMap, HashSet};

use crate::domain::{ComingSoonSupercharger, SiteStatus, StatusChange, compute_sync};
use crate::repository::{ScrapeRunRepository, SuperchargerRepository};
use crate::scraper;

pub async fn run_retry_failed(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    show_browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let failed_detail_chargers = supercharger_repo.get_failed_detail_chargers().await?;
    let failed_open_chargers = supercharger_repo.get_failed_open_status_chargers().await?;

    if failed_detail_chargers.is_empty() && failed_open_chargers.is_empty() {
        println!("No chargers with failed detail fetches or open-status checks. Nothing to retry.");
        return Ok(());
    }

    // Retries complete a scrape session, they don't start a new one: attribute
    // any new status_changes to the latest scrape_runs row.
    let parent_run_id = scrape_run_repo.get_last_run_id().await?
        .ok_or("No scrape runs found — run `scrape` first")?;

    let detail_total = failed_detail_chargers.len();
    let open_total = failed_open_chargers.len();

    if detail_total > 0 {
        println!("Retrying details for {detail_total} charger(s)…");
    }
    if open_total > 0 {
        println!("Retrying open-status checks for {open_total} charger(s)…");
    }

    // Single browser launch — one Akamai wait covers both retry phases.
    let (mut browser, page) = scraper::launch_browser_and_wait(show_browser).await?;

    // ── Phase 1: Retry detail fetches ────────────────────────────────────────
    let (plan, still_detail_failed) = if !failed_detail_chargers.is_empty() {
        let ids: Vec<String> = failed_detail_chargers.iter().map(|c| c.id.clone()).collect();
        let (details_map, still_failed) =
            scraper::fetch_batch_details_from_page(&page, ids).await;

        let updated: Vec<ComingSoonSupercharger> = failed_detail_chargers
            .iter()
            .map(|c| c.clone().with_details(details_map.get(&c.id)))
            .collect();

        let current_map: HashMap<String, _> = failed_detail_chargers
            .iter()
            .map(|c| (c.id.clone(), c.status.clone()))
            .collect();
        let plan = compute_sync(current_map, &updated, &still_failed);
        (plan, still_failed)
    } else {
        (compute_sync(HashMap::new(), &[], &HashSet::new()), HashSet::new())
    };

    // ── Phase 2: Retry open-status checks ────────────────────────────────────
    let (open_results, still_open_failed, os_removed_ids, os_removed_changes) =
        if !failed_open_chargers.is_empty() {
            let ids: Vec<String> = failed_open_chargers.iter().map(|c| c.id.clone()).collect();
            let (open_results, still_failed) =
                scraper::fetch_open_status_for_ids(&page, &ids).await?;

            let mut removed_ids: Vec<String> = vec![];
            let mut removed_changes: Vec<StatusChange> = vec![];

            for charger in &failed_open_chargers {
                if open_results.contains_key(&charger.id) {
                    println!("  ✓ Charger {} has opened — moving to opened_superchargers", charger.id);
                } else if still_failed.contains(&charger.id) {
                    eprintln!("  ⚠ Charger {} open-status check still failing — keeping flag", charger.id);
                } else {
                    eprintln!("  ⚠ Charger {} confirmed absent — marking as removed", charger.id);
                    removed_ids.push(charger.id.clone());
                    removed_changes.push(StatusChange {
                        supercharger_id: charger.id.clone(),
                        old_status: Some(charger.status.clone()),
                        new_status: SiteStatus::Removed,
                    });
                }
            }

            (open_results, still_failed, removed_ids, removed_changes)
        } else {
            (HashMap::new(), HashSet::new(), vec![], vec![])
        };

    browser.close().await.ok();

    // ── Record and save ───────────────────────────────────────────────────────
    scrape_run_repo.update_retry(
        parent_run_id,
        still_detail_failed.len() as i32,
        still_open_failed.len() as i32,
    )
    .await?;

    let mut all_status_changes = plan.status_changes;
    all_status_changes.extend(os_removed_changes);

    supercharger_repo.save_chargers(
        &plan.upserts,
        &plan.unchanged,
        &all_status_changes,
        &os_removed_ids,
        &open_results,
        parent_run_id,
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
