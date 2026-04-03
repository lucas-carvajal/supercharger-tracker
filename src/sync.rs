use std::collections::{HashMap, HashSet};

use crate::coming_soon::{ComingSoonSupercharger, SiteStatus};
use crate::db::StatusChange;

pub struct SyncPlan {
    /// New or changed chargers — written with a full upsert.
    pub upserts: Vec<ComingSoonSupercharger>,
    /// Chargers seen in the scrape with no status change — title/city/region and last_scraped_at are updated.
    pub unchanged: Vec<ComingSoonSupercharger>,
    /// Status events to record: old_status = None means first time seen.
    pub status_changes: Vec<StatusChange>,
    /// Chargers that were in the DB (non-REMOVED) but absent from the latest scrape.
    /// Carries the old status so callers can build StatusChange records for removed ones.
    pub disappeared_ids: Vec<(String, SiteStatus)>,
}

/// Pure diff — no DB calls, no side effects.
///
/// `current` maps each active charger's ID to its current status.
/// `fresh` is everything returned by the latest scrape.
/// `failed_detail_ids` contains IDs whose details fetch failed outright.
/// For existing chargers in this set, the current DB status is preserved to
/// avoid recording a false status change caused by a fetch failure.
pub fn compute_sync(
    current: HashMap<String, SiteStatus>,
    fresh: &[ComingSoonSupercharger],
    failed_detail_ids: &HashSet<String>,
) -> SyncPlan {
    let mut upserts = Vec::new();
    let mut unchanged = Vec::new();
    let mut status_changes = Vec::new();

    let fresh_ids: HashSet<&str> = fresh.iter().map(|c| c.id.as_str()).collect();

    for charger in fresh {
        let detail_fetch_failed = failed_detail_ids.contains(&charger.id);

        match current.get(&charger.id) {
            None => {
                // Truly new charger — record first appearance.
                // If details failed, status will be UNKNOWN; that's acceptable for a new entry.
                status_changes.push(StatusChange {
                    supercharger_id: charger.id.clone(),
                    old_status: None,
                    new_status: charger.status.clone(),
                });
                upserts.push(charger.clone());
            }
            Some(old_status) => {
                // For existing chargers, if the details fetch failed use the current DB
                // status as the effective status to avoid recording a spurious change.
                let new_status = if detail_fetch_failed { old_status } else { &charger.status };

                if old_status != new_status {
                    status_changes.push(StatusChange {
                        supercharger_id: charger.id.clone(),
                        old_status: Some(old_status.clone()),
                        new_status: new_status.clone(),
                    });
                    upserts.push(ComingSoonSupercharger {
                        status: new_status.clone(),
                        ..charger.clone()
                    });
                } else {
                    unchanged.push(charger.clone());
                }
            }
        }
    }

    // Exclude REMOVED chargers — they stay absent from the feed indefinitely and
    // should not re-trigger an open-check on every scrape.
    let disappeared_ids = current
        .into_iter()
        .filter(|(id, old_status)| {
            !fresh_ids.contains(id.as_str()) && *old_status != SiteStatus::Removed
        })
        .collect();

    SyncPlan {
        upserts,
        unchanged,
        status_changes,
        disappeared_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coming_soon::SiteStatus;

    fn charger(id: &str, status: SiteStatus) -> ComingSoonSupercharger {
        ComingSoonSupercharger {
            id: id.to_string(),
            title: format!("Charger {id}"),
            city: None,
            region: None,
            latitude: 0.0,
            longitude: 0.0,
            status,
            raw_status_value: None,
            charger_category: crate::coming_soon::ChargerCategory::ComingSoon,
        }
    }

    #[test]
    fn new_charger_produces_upsert_and_status_change() {
        let current = HashMap::new();
        let fresh = vec![charger("abc", SiteStatus::InDevelopment)];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert!(plan.status_changes[0].old_status.is_none());
        assert_eq!(plan.unchanged.len(), 0);
        assert_eq!(plan.disappeared_ids.len(), 0);
    }

    #[test]
    fn unchanged_charger_goes_to_unchanged_ids() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::InDevelopment)]);
        let fresh = vec![charger("abc", SiteStatus::InDevelopment)];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
        assert_eq!(plan.unchanged.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["abc"]);
        assert_eq!(plan.disappeared_ids.len(), 0);
    }

    #[test]
    fn status_change_produces_upsert_and_status_change_with_old_status() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::InDevelopment)]);
        let fresh = vec![charger("abc", SiteStatus::UnderConstruction)];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert_eq!(plan.status_changes[0].old_status, Some(SiteStatus::InDevelopment));
        assert_eq!(plan.unchanged.len(), 0);
    }

    #[test]
    fn absent_from_scrape_goes_to_disappeared() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::UnderConstruction)]);
        let fresh = vec![];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(
            plan.disappeared_ids,
            vec![("abc".to_string(), SiteStatus::UnderConstruction)]
        );
        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
    }

    #[test]
    fn removed_charger_absent_from_scrape_not_in_disappeared() {
        // A charger already marked REMOVED should not re-appear in disappeared_ids
        // on subsequent scrapes where it is still absent from the Tesla feed.
        let current = HashMap::from([("abc".to_string(), SiteStatus::Removed)]);
        let fresh = vec![];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.disappeared_ids.len(), 0, "REMOVED charger should not re-enter disappeared_ids");
        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
    }

    #[test]
    fn failed_detail_fetch_preserves_existing_status() {
        // Charger was IN_DEVELOPMENT; details fetch failed and scraped status is UNKNOWN.
        // compute_sync should treat it as unchanged, not record a spurious status change.
        let current = HashMap::from([("abc".to_string(), SiteStatus::InDevelopment)]);
        let fresh = vec![charger("abc", SiteStatus::Unknown)];
        let failed = HashSet::from(["abc".to_string()]);
        let plan = compute_sync(current, &fresh, &failed);

        assert_eq!(plan.upserts.len(), 0, "should not upsert when details failed");
        assert_eq!(plan.status_changes.len(), 0, "should not record false status change");
        assert_eq!(plan.unchanged.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["abc"]);
    }

    #[test]
    fn failed_detail_fetch_for_new_charger_records_unknown() {
        // Brand-new charger with failed details — we have no prior data, so UNKNOWN is fine.
        let current = HashMap::new();
        let fresh = vec![charger("new", SiteStatus::Unknown)];
        let failed = HashSet::from(["new".to_string()]);
        let plan = compute_sync(current, &fresh, &failed);

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert!(plan.status_changes[0].old_status.is_none());
        assert_eq!(plan.status_changes[0].new_status, SiteStatus::Unknown);
    }
}
