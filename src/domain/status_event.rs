use chrono::{DateTime, Utc};

use super::SiteStatus;

/// One `status_changes` row plus the live coming-soon row, if it still exists.
///
/// The three recent-* feeds are views over this event, filtered in domain logic
/// rather than SQL. First-seen events have `old_status = None`.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusEvent {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub old_status: Option<SiteStatus>,
    pub new_status: SiteStatus,
    pub changed_at: DateTime<Utc>,
    pub charger: Option<StatusEventCharger>,
}

/// Fields from `coming_soon_superchargers` used by the additions feed.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusEventCharger {
    pub latitude: f64,
    pub longitude: f64,
    pub status: SiteStatus,
    pub raw_status_value: Option<String>,
    pub first_seen_at: DateTime<Utc>,
}

impl StatusEvent {
    /// `/recent-changes`: real transitions, excluding `→ UNKNOWN`.
    pub fn is_change(&self) -> bool {
        self.old_status.is_some() && self.new_status != SiteStatus::Unknown
    }

    /// `/recent-updates`: first-seen or transition, excluding `→ REMOVED` / `→ UNKNOWN`.
    pub fn is_update(&self) -> bool {
        self.new_status != SiteStatus::Removed && self.new_status != SiteStatus::Unknown
    }

    /// `/recent-additions`: first-seen and still an active coming-soon row.
    pub fn is_addition(&self) -> bool {
        self.old_status.is_none()
            && self
                .charger
                .as_ref()
                .is_some_and(|c| c.status != SiteStatus::Removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        old: Option<SiteStatus>,
        new: SiteStatus,
        charger_status: Option<SiteStatus>,
    ) -> StatusEvent {
        StatusEvent {
            id: "1".into(),
            title: "t".into(),
            city: None,
            region: None,
            old_status: old,
            new_status: new,
            changed_at: DateTime::UNIX_EPOCH,
            charger: charger_status.map(|status| StatusEventCharger {
                latitude: 0.0,
                longitude: 0.0,
                status,
                raw_status_value: None,
                first_seen_at: DateTime::UNIX_EPOCH,
            }),
        }
    }

    #[test]
    fn first_seen_is_addition_and_update_not_change() {
        let e = event(None, SiteStatus::Design, Some(SiteStatus::Design));
        assert!(!e.is_change());
        assert!(e.is_update());
        assert!(e.is_addition());
    }

    #[test]
    fn transition_is_change_and_update_not_addition() {
        let e = event(
            Some(SiteStatus::Design),
            SiteStatus::Construction,
            Some(SiteStatus::Construction),
        );
        assert!(e.is_change());
        assert!(e.is_update());
        assert!(!e.is_addition());
    }

    #[test]
    fn removed_destination_is_change_only() {
        let e = event(
            Some(SiteStatus::Design),
            SiteStatus::Removed,
            Some(SiteStatus::Removed),
        );
        assert!(e.is_change());
        assert!(!e.is_update());
        assert!(!e.is_addition());
    }

    #[test]
    fn unknown_destination_is_excluded_from_changes_and_updates() {
        let e = event(Some(SiteStatus::Design), SiteStatus::Unknown, None);
        assert!(!e.is_change());
        assert!(!e.is_update());
        assert!(!e.is_addition());
    }

    #[test]
    fn first_seen_unknown_still_counts_as_addition() {
        let e = event(None, SiteStatus::Unknown, Some(SiteStatus::Unknown));
        assert!(!e.is_change());
        assert!(!e.is_update());
        assert!(e.is_addition());
    }

    #[test]
    fn first_seen_without_active_charger_is_not_addition() {
        let opened = event(None, SiteStatus::Design, None);
        assert!(!opened.is_addition());

        let removed = event(None, SiteStatus::Design, Some(SiteStatus::Removed));
        assert!(!removed.is_addition());
    }
}
