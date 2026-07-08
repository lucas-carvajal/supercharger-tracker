use std::collections::{HashMap, HashSet};

use std::time::Duration;

use crate::domain::{ComingSoonSupercharger, OpenResult, SiteStatus, StatusChange, compute_sync};
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
        tracing::info!(
            "no chargers with failed detail fetches or open-status checks — nothing to retry"
        );
        return Ok(());
    }

    // Retries complete a scrape session, they don't start a new one: attribute
    // any new status_changes to the latest scrape_runs row.
    let parent_run_id = scrape_run_repo
        .get_last_run_id()
        .await?
        .ok_or("No scrape runs found — run `scrape` first")?;

    let detail_total = failed_detail_chargers.len();
    let open_total = failed_open_chargers.len();

    if detail_total > 0 {
        tracing::info!(count = detail_total, "retrying detail fetches");
    }
    if open_total > 0 {
        tracing::info!(count = open_total, "retrying open-status checks");
    }

    // Single browser launch — one Akamai wait covers both retry phases.
    let session = scraper::launch_browser_and_wait(show_browser).await?;

    let open_failed_ids_at_start: HashSet<String> =
        failed_open_chargers.iter().map(|c| c.id.clone()).collect();

    // ── Phase 1: Retry detail fetches ────────────────────────────────────────
    let mut still_detail_failed: HashSet<String> = HashSet::new();
    let mut detail_status_changes = 0usize;
    let mut detail_upserts = 0usize;
    let mut detail_unchanged = 0usize;
    let mut detail_blocked = false;

    let mut unknown_enum_tracker = scraper::UnknownEnumTracker::default();

    if !failed_detail_chargers.is_empty() {
        let batches: Vec<&[ComingSoonSupercharger]> = failed_detail_chargers
            .chunks(scraper::DETAILS_BATCH_SIZE)
            .collect();
        let num_batches = batches.len();

        for (batch_index, batch) in batches.iter().enumerate() {
            let ids: Vec<String> = batch.iter().map(|c| c.id.clone()).collect();
            tracing::info!(
                batch = batch_index + 1,
                num_batches,
                size = ids.len(),
                first_id = ids.first().map(String::as_str),
                last_id = ids.last().map(String::as_str),
                "retry detail batch started"
            );

            let fetch_result = scraper::fetch_detail_batch_from_page(
                &session.page,
                &ids,
                &mut unknown_enum_tracker,
            )
            .await;
            still_detail_failed.extend(fetch_result.failed_ids.iter().cloned());

            let updated: Vec<ComingSoonSupercharger> = batch
                .iter()
                .map(|c| c.clone().with_details(fetch_result.details.get(&c.id)))
                .collect();

            let current_map: HashMap<String, _> = batch
                .iter()
                .map(|c| (c.id.clone(), c.status.clone()))
                .collect();
            let plan = compute_sync(current_map, &updated, &fetch_result.failed_ids);
            let open_failed_for_batch: HashSet<String> = ids
                .iter()
                .filter(|id| open_failed_ids_at_start.contains(*id))
                .cloned()
                .collect();

            tracing::info!(
                batch = batch_index + 1,
                num_batches,
                attempted = ids.len(),
                resolved = ids.len().saturating_sub(fetch_result.failed_ids.len()),
                with_details = fetch_result.details.len(),
                resolved_without_details = fetch_result.resolved_without_details,
                still_failed = fetch_result.failed_ids.len(),
                blocked = fetch_result.blocked,
                reasons = ?fetch_result.failure_reasons,
                upserts = plan.upserts.len(),
                unchanged = plan.unchanged.len(),
                status_changes = plan.status_changes.len(),
                "retry detail batch classified"
            );

            let no_removed_ids: Vec<String> = Vec::new();
            let no_open_results: HashMap<String, OpenResult> = HashMap::new();
            supercharger_repo
                .save_chargers(
                    &plan.upserts,
                    &plan.unchanged,
                    &plan.status_changes,
                    &no_removed_ids,
                    &no_open_results,
                    parent_run_id,
                    &fetch_result.failed_ids,
                    &open_failed_for_batch,
                )
                .await?;

            detail_status_changes += plan.status_changes.len();
            detail_upserts += plan.upserts.len();
            detail_unchanged += plan.unchanged.len();

            let db_stats = supercharger_repo.get_db_stats().await?;
            scrape_run_repo
                .update_retry(
                    parent_run_id,
                    db_stats.details_failed as i32,
                    db_stats.open_status_check_failed as i32,
                )
                .await?;
            tracing::info!(
                batch = batch_index + 1,
                num_batches,
                detail_still_failing_in_db = db_stats.details_failed,
                open_still_failing_in_db = db_stats.open_status_check_failed,
                "retry detail batch saved"
            );

            if fetch_result.blocked {
                detail_blocked = true;
                still_detail_failed.extend(
                    batches[batch_index + 1..]
                        .iter()
                        .flat_map(|remaining| remaining.iter().map(|c| c.id.clone())),
                );
                tracing::warn!(
                    batch = batch_index + 1,
                    num_batches,
                    remaining = still_detail_failed.len(),
                    "detail retry saw a block response — skipping remaining detail batches"
                );
                break;
            }

            if batch_index + 1 < num_batches {
                tokio::time::sleep(Duration::from_millis(scraper::DETAILS_BATCH_DELAY_MS)).await;
            }
        }

        unknown_enum_tracker.log_summary();
    }

    // ── Phase 2: Retry open-status checks ────────────────────────────────────
    let (open_results, still_open_failed, os_removed_ids, os_removed_changes) =
        if detail_blocked && !failed_open_chargers.is_empty() {
            tracing::warn!(
                count = failed_open_chargers.len(),
                "skipping open-status retry because detail retry was blocked"
            );
            (
                HashMap::new(),
                open_failed_ids_at_start.clone(),
                vec![],
                vec![],
            )
        } else if !failed_open_chargers.is_empty() {
            let ids: Vec<String> = failed_open_chargers.iter().map(|c| c.id.clone()).collect();
            let (open_results, still_failed) =
                scraper::fetch_open_status_for_ids(&session.page, &ids).await?;

            let mut removed_ids: Vec<String> = vec![];
            let mut removed_changes: Vec<StatusChange> = vec![];

            for charger in &failed_open_chargers {
                if open_results.contains_key(&charger.id) {
                    tracing::info!(
                        id = charger.id,
                        "charger has opened — moving to opened_superchargers"
                    );
                } else if still_failed.contains(&charger.id) {
                    tracing::warn!(
                        id = charger.id,
                        "open-status check still failing — keeping flag"
                    );
                } else {
                    tracing::warn!(
                        id = charger.id,
                        "charger confirmed absent — marking as removed"
                    );
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

    session.close().await;

    // ── Save open-status results and record final counters ───────────────────
    let open_status_changes = os_removed_changes.len();

    supercharger_repo
        .save_chargers(
            &[],
            &[],
            &os_removed_changes,
            &os_removed_ids,
            &open_results,
            parent_run_id,
            &still_detail_failed,
            &still_open_failed,
        )
        .await?;

    let db_stats = supercharger_repo.get_db_stats().await?;
    scrape_run_repo
        .update_retry(
            parent_run_id,
            db_stats.details_failed as i32,
            db_stats.open_status_check_failed as i32,
        )
        .await?;

    let detail_resolved = detail_total.saturating_sub(still_detail_failed.len());
    let open_resolved = open_total.saturating_sub(still_open_failed.len());
    tracing::info!(
        detail_resolved,
        detail_still_failing = still_detail_failed.len(),
        detail_still_failing_in_db = db_stats.details_failed,
        detail_upserts,
        detail_unchanged,
        open_resolved,
        open_still_failing = still_open_failed.len(),
        open_still_failing_in_db = db_stats.open_status_check_failed,
        detail_status_changes,
        open_status_changes,
        "retry complete"
    );

    Ok(())
}
