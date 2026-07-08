use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;

use super::coming_soon::{
    ComingSoonSupercharger, SiteStatus, derive_status, resolve_status_transition,
};

/// A status transition event for a single supercharger.
/// `supercharger_id` references `coming_soon_superchargers.id`.
pub struct StatusChange {
    pub supercharger_id: String,
    pub old_status: Option<SiteStatus>,
    pub new_status: SiteStatus,
}

/// Data captured when a coming-soon charger is confirmed open via the Tesla API.
pub struct OpenResult {
    pub opening_date: Option<NaiveDate>,
    pub num_stalls: Option<i32>,
    pub open_to_non_tesla: Option<bool>,
}

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

fn resolved_status(
    existing: Option<&SiteStatus>,
    charger: &ComingSoonSupercharger,
    detail_fetch_failed: bool,
) -> SiteStatus {
    let derived = derive_status(
        charger.raw_project_status.as_deref(),
        charger.raw_status_value.as_deref(),
    );

    let Some(old_status) = existing else {
        return derived.status;
    };

    if detail_fetch_failed {
        return old_status.clone();
    }

    resolve_status_transition(old_status.clone(), derived.status, derived.source)
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
        let new_status = resolved_status(current.get(&charger.id), charger, detail_fetch_failed);

        match current.get(&charger.id) {
            None => {
                status_changes.push(StatusChange {
                    supercharger_id: charger.id.clone(),
                    old_status: None,
                    new_status: new_status.clone(),
                });
                upserts.push(ComingSoonSupercharger {
                    status: new_status,
                    ..charger.clone()
                });
            }
            Some(old_status) => {
                if old_status != &new_status {
                    status_changes.push(StatusChange {
                        supercharger_id: charger.id.clone(),
                        old_status: Some(old_status.clone()),
                        new_status: new_status.clone(),
                    });
                    upserts.push(ComingSoonSupercharger {
                        status: new_status,
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
    use super::super::coming_soon::{ChargerCategory, SiteStatus};
    use super::*;

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
            raw_project_status: None,
            num_charger_stalls: 0,
            charging_accessibility: None,
            street_address: None,
            county: None,
            postal_code: None,
            country_code: None,
            charger_category: ChargerCategory::ComingSoon,
        }
    }

    fn charger_with_raw(
        id: &str,
        raw_project_status: Option<&str>,
        raw_status_value: Option<&str>,
    ) -> ComingSoonSupercharger {
        ComingSoonSupercharger {
            raw_project_status: raw_project_status.map(str::to_string),
            raw_status_value: raw_status_value.map(str::to_string),
            status: derive_status(raw_project_status, raw_status_value).status,
            ..charger(id, SiteStatus::Unknown)
        }
    }

    #[test]
    fn new_charger_produces_upsert_and_status_change() {
        let current = HashMap::new();
        let fresh = vec![charger_with_raw(
            "abc",
            Some("Design"),
            Some("In Development"),
        )];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert!(plan.status_changes[0].old_status.is_none());
        assert_eq!(plan.status_changes[0].new_status, SiteStatus::Design);
        assert_eq!(plan.unchanged.len(), 0);
        assert_eq!(plan.disappeared_ids.len(), 0);
    }

    #[test]
    fn unchanged_charger_goes_to_unchanged_ids() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::Design)]);
        let fresh = vec![charger_with_raw(
            "abc",
            Some("Design"),
            Some("In Development"),
        )];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
        assert_eq!(
            plan.unchanged
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["abc"]
        );
        assert_eq!(plan.disappeared_ids.len(), 0);
    }

    #[test]
    fn status_change_produces_upsert_and_status_change_with_old_status() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::Design)]);
        let fresh = vec![charger_with_raw(
            "abc",
            Some("Construction"),
            Some("Under Construction"),
        )];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert_eq!(plan.status_changes[0].old_status, Some(SiteStatus::Design));
        assert_eq!(plan.status_changes[0].new_status, SiteStatus::Construction);
        assert_eq!(plan.unchanged.len(), 0);
    }

    #[test]
    fn absent_from_scrape_goes_to_disappeared() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::Construction)]);
        let fresh = vec![];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(
            plan.disappeared_ids,
            vec![("abc".to_string(), SiteStatus::Construction)]
        );
        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
    }

    #[test]
    fn removed_charger_absent_from_scrape_not_in_disappeared() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::Removed)]);
        let fresh = vec![];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(
            plan.disappeared_ids.len(),
            0,
            "REMOVED charger should not re-enter disappeared_ids"
        );
        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
    }

    #[test]
    fn removed_reappearance_records_transition() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::Removed)]);
        let fresh = vec![charger_with_raw(
            "abc",
            Some("Design"),
            Some("In Development"),
        )];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert_eq!(plan.status_changes[0].old_status, Some(SiteStatus::Removed));
        assert_eq!(plan.status_changes[0].new_status, SiteStatus::Design);
    }

    #[test]
    fn failed_detail_fetch_preserves_existing_status() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::Design)]);
        let fresh = vec![charger_with_raw("abc", None, None)];
        let failed = HashSet::from(["abc".to_string()]);
        let plan = compute_sync(current, &fresh, &failed);

        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
        assert_eq!(
            plan.unchanged
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["abc"]
        );
    }

    #[test]
    fn failed_detail_fetch_for_new_charger_records_unknown() {
        let current = HashMap::new();
        let fresh = vec![charger_with_raw("new", None, None)];
        let failed = HashSet::from(["new".to_string()]);
        let plan = compute_sync(current, &fresh, &failed);

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert!(plan.status_changes[0].old_status.is_none());
        assert_eq!(plan.status_changes[0].new_status, SiteStatus::Unknown);
    }

    #[test]
    fn d8_fallback_regression_not_recorded() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::Design)]);
        let fresh = vec![charger_with_raw("abc", None, Some("In Development"))];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
        assert_eq!(plan.unchanged.len(), 1);
    }
}
