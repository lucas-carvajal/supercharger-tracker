use std::collections::{HashMap, HashSet};

use crate::coming_soon::{ComingSoonSupercharger, SiteStatus};
use crate::db::StatusChange;

pub struct SyncPlan {
    /// New or changed chargers — written with a full upsert.
    pub upserts: Vec<ComingSoonSupercharger>,
    /// Chargers seen in the scrape with no changes — only last_scraped_at is touched.
    pub unchanged_uuids: Vec<String>,
    /// Status events to record: old_status = None means first time seen.
    pub status_changes: Vec<StatusChange>,
    /// Chargers that were active in the DB but absent from the latest scrape.
    pub disappeared_uuids: Vec<String>,
}

/// Pure diff — no DB calls, no side effects.
///
/// `current` is the full set of active chargers from the DB (uuid → status).
/// `fresh` is everything returned by the latest scrape.
pub fn compute_sync(current: HashMap<String, SiteStatus>, fresh: &[ComingSoonSupercharger]) -> SyncPlan {
    let mut upserts = Vec::new();
    let mut unchanged_uuids = Vec::new();
    let mut status_changes = Vec::new();

    let fresh_uuids: HashSet<&str> = fresh.iter().map(|c| c.uuid.as_str()).collect();

    for charger in fresh {
        match current.get(&charger.uuid) {
            None => {
                // First time we've seen this charger
                status_changes.push(StatusChange {
                    supercharger_uuid: charger.uuid.clone(),
                    old_status: None,
                    new_status: charger.status.clone(),
                });
                upserts.push(charger.clone());
            }
            Some(old_status) if old_status != &charger.status => {
                // Status has changed since last scrape
                status_changes.push(StatusChange {
                    supercharger_uuid: charger.uuid.clone(),
                    old_status: Some(old_status.clone()),
                    new_status: charger.status.clone(),
                });
                upserts.push(charger.clone());
            }
            Some(_) => {
                // Seen before, nothing changed
                unchanged_uuids.push(charger.uuid.clone());
            }
        }
    }

    let disappeared_uuids = current
        .into_keys()
        .filter(|uuid| !fresh_uuids.contains(uuid.as_str()))
        .collect();

    SyncPlan {
        upserts,
        unchanged_uuids,
        status_changes,
        disappeared_uuids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coming_soon::SiteStatus;

    fn charger(uuid: &str, status: SiteStatus) -> ComingSoonSupercharger {
        ComingSoonSupercharger {
            uuid: uuid.to_string(),
            title: format!("Charger {uuid}"),
            latitude: 0.0,
            longitude: 0.0,
            status,
            location_url_slug: None,
            raw_status_value: None,
        }
    }

    #[test]
    fn new_charger_produces_upsert_and_status_change() {
        let current = HashMap::new();
        let fresh = vec![charger("abc", SiteStatus::InDevelopment)];
        let plan = compute_sync(current, &fresh);

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert!(plan.status_changes[0].old_status.is_none());
        assert_eq!(plan.unchanged_uuids.len(), 0);
        assert_eq!(plan.disappeared_uuids.len(), 0);
    }

    #[test]
    fn unchanged_charger_goes_to_unchanged_uuids() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::InDevelopment)]);
        let fresh = vec![charger("abc", SiteStatus::InDevelopment)];
        let plan = compute_sync(current, &fresh);

        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
        assert_eq!(plan.unchanged_uuids, vec!["abc"]);
        assert_eq!(plan.disappeared_uuids.len(), 0);
    }

    #[test]
    fn status_change_produces_upsert_and_status_change_with_old_status() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::InDevelopment)]);
        let fresh = vec![charger("abc", SiteStatus::UnderConstruction)];
        let plan = compute_sync(current, &fresh);

        assert_eq!(plan.upserts.len(), 1);
        assert_eq!(plan.status_changes.len(), 1);
        assert_eq!(plan.status_changes[0].old_status, Some(SiteStatus::InDevelopment));
        assert_eq!(plan.unchanged_uuids.len(), 0);
    }

    #[test]
    fn absent_from_scrape_goes_to_disappeared() {
        let current = HashMap::from([("abc".to_string(), SiteStatus::UnderConstruction)]);
        let fresh = vec![];
        let plan = compute_sync(current, &fresh);

        assert_eq!(plan.disappeared_uuids, vec!["abc"]);
        assert_eq!(plan.upserts.len(), 0);
        assert_eq!(plan.status_changes.len(), 0);
    }
}
