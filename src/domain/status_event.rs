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
    pub country: Option<String>,
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
