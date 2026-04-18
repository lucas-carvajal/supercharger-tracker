use std::collections::{HashMap, HashSet};

use crate::domain::{ComingSoonSupercharger, SiteStatus, StatusChange};
use crate::repository::{ScrapeRunRepository, SuperchargerRepository};
use crate::scraper;

pub async fn run_scrape(
    supercharger_repo: &SuperchargerRepository,
    scrape_run_repo: &ScrapeRunRepository,
    country: String,
    show_browser: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut browser, page) = scraper::launch_browser_and_wait(show_browser).await?;

    let scrape_result = async {
        let result = scraper::load_from_browser(&country, &page).await?;

        let failed_count = result.failed_detail_ids.len();
        if failed_count > 0 {
            let total_with_ids = result
                .locations
                .iter()
                .filter(|l| ComingSoonSupercharger::is_coming_soon(l))
                .filter(|l| l.location_url_slug != "null" && !l.location_url_slug.is_empty())
                .count();
            let pct = failed_count * 100 / total_with_ids.max(1);
            tracing::warn!(
                failed = failed_count, total = total_with_ids, pct,
                "details fetch: some chargers failed — existing statuses preserved"
            );
            if pct > 50 {
                tracing::warn!("high detail-fetch failure rate — check for Akamai blocking or API issues");
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

        let current = supercharger_repo.get_current_statuses().await?;
        let plan = crate::domain::compute_sync(current, &coming_soon, &result.failed_detail_ids);

        // For disappeared chargers, check whether they have opened (gone live).
        let (open_results, open_status_failed_ids) = if plan.disappeared_ids.is_empty() {
            (HashMap::new(), HashSet::new())
        } else {
            let ids: Vec<String> = plan.disappeared_ids.iter().map(|(id, _)| id.clone()).collect();
            tracing::info!(count = ids.len(), "checking open status for disappeared chargers");
            match scraper::fetch_open_status_for_ids(&page, &ids).await {
                Ok((results, failed)) => {
                    if !failed.is_empty() {
                        tracing::warn!(
                            failed = failed.len(), total = ids.len(),
                            "open-status check: some chargers failed — flagged for retry"
                        );
                    }
                    (results, failed)
                }
                Err(e) => {
                    // Total call failure: flag all disappeared chargers so none are
                    // falsely marked REMOVED.
                    tracing::error!(
                        error = %e, count = ids.len(),
                        "open-status check failed entirely — flagging all disappeared chargers for retry"
                    );
                    (HashMap::new(), ids.into_iter().collect())
                }
            }
        };

        let mut removed_ids: Vec<String> = vec![];
        let mut removed_status_changes: Vec<StatusChange> = vec![];

        for (id, old_status) in &plan.disappeared_ids {
            if open_results.contains_key(id) {
                tracing::info!(id, "charger has opened — moving to opened_superchargers");
            } else if open_status_failed_ids.contains(id) {
                tracing::warn!(id, "open-status check failed — flagging for retry");
            } else {
                tracing::warn!(id, "disappeared charger not found in Tesla API — marking as removed");
                removed_ids.push(id.clone());
                removed_status_changes.push(StatusChange {
                    supercharger_id: id.clone(),
                    old_status: Some(old_status.clone()),
                    new_status: SiteStatus::Removed,
                });
            }
        }

        let run_id = scrape_run_repo.record_run(
            &country,
            coming_soon.len() as i32,
            failed_count as i32,
            open_status_failed_ids.len() as i32,
            "full",
        )
        .await?;

        let mut all_status_changes = plan.status_changes;
        all_status_changes.extend(removed_status_changes);

        supercharger_repo.save_chargers(
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

        tracing::info!(
            upserts = plan.upserts.len(),
            status_changes = all_status_changes.len(),
            opened = open_results.len(),
            removed = removed_ids.len(),
            open_check_pending = open_status_failed_ids.len(),
            unchanged = plan.unchanged.len(),
            "DB update complete"
        );

        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;

    browser.close().await.ok();
    scrape_result
}
