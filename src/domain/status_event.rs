use chrono::{DateTime, Utc};

use super::SiteStatus;

/// One `status_changes` row with coming-soon / opened title fallback.
/// First-seen events have `old_status = None`.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusEvent {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub old_status: Option<SiteStatus>,
    pub new_status: SiteStatus,
    pub changed_at: DateTime<Utc>,
}

/// Which recent-* feed to load from `status_changes`.
#[derive(Debug, Clone, Copy)]
pub enum StatusEventFeed {
    /// `/recent-changes`: transitions only, not `→ UNKNOWN`.
    Changes,
    /// `/recent-updates`: first-seen + transitions, not `→ REMOVED` / `→ UNKNOWN`.
    Updates,
}

impl StatusEventFeed {
    pub fn sql_predicate(self) -> &'static str {
        match self {
            Self::Changes => "old_status IS NOT NULL AND new_status != 'UNKNOWN'",
            Self::Updates => "new_status != 'REMOVED' AND new_status != 'UNKNOWN'",
        }
    }
}

impl StatusEvent {
    pub fn is_change(&self) -> bool {
        self.old_status.is_some() && self.new_status != SiteStatus::Unknown
    }

    pub fn is_update(&self) -> bool {
        self.new_status != SiteStatus::Removed && self.new_status != SiteStatus::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(old: Option<SiteStatus>, new: SiteStatus) -> StatusEvent {
        StatusEvent {
            id: "1".into(),
            title: "t".into(),
            city: None,
            region: None,
            old_status: old,
            new_status: new,
            changed_at: DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn first_seen_is_update_not_change() {
        let e = event(None, SiteStatus::Design);
        assert!(!e.is_change());
        assert!(e.is_update());
    }

    #[test]
    fn transition_is_change_and_update() {
        let e = event(Some(SiteStatus::Design), SiteStatus::Construction);
        assert!(e.is_change());
        assert!(e.is_update());
    }

    #[test]
    fn removed_destination_is_change_only() {
        let e = event(Some(SiteStatus::Design), SiteStatus::Removed);
        assert!(e.is_change());
        assert!(!e.is_update());
    }

    #[test]
    fn unknown_destination_is_excluded_from_both_feeds() {
        let e = event(Some(SiteStatus::Design), SiteStatus::Unknown);
        assert!(!e.is_change());
        assert!(!e.is_update());
    }
}
