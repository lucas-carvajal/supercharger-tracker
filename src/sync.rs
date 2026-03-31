use std::collections::{HashMap, HashSet};

use crate::coming_soon::{ComingSoonSupercharger, SiteStatus};
use crate::db::StatusChange;

pub struct SyncPlan {
    /// New or changed chargers — written with a full upsert.
    pub upserts: Vec<ComingSoonSupercharger>,
    /// Chargers seen in the scrape with no changes — only last_scraped_at is touched.
    pub unchanged_slugs: Vec<String>,
    /// Status events to record: old_status = None means first time seen.
    pub status_changes: Vec<StatusChange>,
    /// Chargers that were active in the DB but absent from the latest scrape.
    pub disappeared_slugs: Vec<String>,
}

/// Pure diff — no DB calls, no side effects.
///
/// `current` is the full set of active chargers from the DB (slug → status).
/// `fresh` is everything returned by the latest scrape.
/// `failed_detail_slugs` contains slugs whose details fetch failed outright.
/// For existing chargers whose slug is in this set, the current DB status is
/// preserved to avoid recording a false status change caused by a fetch failure.
pub fn compute_sync(
    current: HashMap<String, SiteStatus>,
    fresh: &[ComingSoonSupercharger],
    failed_detail_slugs: &HashSet<String>,
) -> SyncPlan {
    let mut upserts = Vec::new();
    let mut unchanged_slugs = Vec::new();
    let mut status_changes = Vec::new();

    let fresh_slugs: HashSet<&str> = fresh.iter().map(|c| c.slug.as_str()).collect();

    for charger in fresh {
        let detail_fetch_failed = failed_detail_slugs.contains(&charger.slug);

        match current.get(&charger.slug) {
            None => {
                // Truly new charger — record first appearance.
                // If details failed, status will be UNKNOWN; that's acceptable for a new entry.
                status_changes.push(StatusChange {
                    slug: charger.slug.clone(),
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
                        slug: charger.slug.clone(),
                        old_status: Some(old_status.clone()),
                        new_status: new_status.clone(),
                    });
                    upserts.push(ComingSoonSupercharger {
                        status: new_status.clone(),
                        ..charger.clone()
                    });
                } else {
                    unchanged_slugs.push(charger.slug.clone());
                }
            }
        }
    }

    let disappeared_slugs = current
        .into_keys()
        .filter(|slug| !fresh_slugs.contains(slug.as_str()))
        .collect();

    SyncPlan {
        upserts,
        unchanged_slugs,
        status_changes,
        disappeared_slugs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coming_soon::SiteStatus;

    fn charger(slug: &str, status: SiteStatus) -> ComingSoonSupercharger {
        ComingSoonSupercharger {
            slug: slug.to_string(),
            title: format!("Charger {slug}"),
            latitude: 0.0,
            longitude: 0.0,
            status,
            raw_status_value: None,
        }
    }

    #[test]
    fn new_charger_produces_upsert_and_status_change() {
        let current = HashMap::new();
        let fresh = vec![charger("slug-abc", SiteStatus::InDevelopment)];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert!(plan.status_changes[0].old_status.is_none());
        assert_eq!(plan.unchanged_slugs.len(), 0);
        assert_eq!(plan.disappeared_slugs.len(), 0);
    }

    #[test]
    fn unchanged_charger_goes_to_unchanged_slugs() {
        let current = HashMap::from([("slug-abc".to_string(), SiteStatus::InDevelopment)]);
        let fresh = vec![charger("slug-abc", SiteStatus::InDevelopment)];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
        assert_eq!(plan.unchanged_slugs, vec!["slug-abc"]);
        assert_eq!(plan.disappeared_slugs.len(), 0);
    }

    #[test]
    fn status_change_produces_upsert_and_status_change_with_old_status() {
        let current = HashMap::from([("slug-abc".to_string(), SiteStatus::InDevelopment)]);
        let fresh = vec![charger("slug-abc", SiteStatus::UnderConstruction)];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert_eq!(plan.status_changes[0].old_status, Some(SiteStatus::InDevelopment));
        assert_eq!(plan.unchanged_slugs.len(), 0);
    }

    #[test]
    fn absent_from_scrape_goes_to_disappeared() {
        let current = HashMap::from([("slug-abc".to_string(), SiteStatus::UnderConstruction)]);
        let fresh = vec![];
        let plan = compute_sync(current, &fresh, &HashSet::new());

        assert_eq!(plan.disappeared_slugs, vec!["slug-abc"]);
        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
    }

    #[test]
    fn failed_detail_fetch_preserves_existing_status() {
        // Charger was IN_DEVELOPMENT; details fetch failed and scraped status is UNKNOWN.
        // compute_sync should treat it as unchanged, not record a spurious status change.
        let current = HashMap::from([("slug-1".to_string(), SiteStatus::InDevelopment)]);
        let fresh = vec![charger("slug-1", SiteStatus::Unknown)];
        let failed = HashSet::from(["slug-1".to_string()]);
        let plan = compute_sync(current, &fresh, &failed);

        assert_eq!(plan.upserts.len(), 0, "should not upsert when details failed");
        assert_eq!(plan.status_changes.len(), 0, "should not record false status change");
        assert_eq!(plan.unchanged_slugs, vec!["slug-1"]);
    }

    #[test]
    fn failed_detail_fetch_for_new_charger_records_unknown() {
        // Brand-new charger with failed details — we have no prior data, so UNKNOWN is fine.
        let current = HashMap::new();
        let fresh = vec![charger("slug-2", SiteStatus::Unknown)];
        let failed = HashSet::from(["slug-2".to_string()]);
        let plan = compute_sync(current, &fresh, &failed);

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert!(plan.status_changes[0].old_status.is_none());
        assert_eq!(plan.status_changes[0].new_status, SiteStatus::Unknown);
    }
}
