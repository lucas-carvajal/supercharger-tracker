use chrono::{DateTime, Utc};

// ── Supercharger read models ──────────────────────────────────────────────────

pub struct ApiSupercharger {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub status: String,
    pub raw_status_value: Option<String>,
    pub first_seen_at: DateTime<Utc>,
    pub last_scraped_at: DateTime<Utc>,
    pub details_fetch_failed: bool,
}

pub struct ApiStatusHistory {
    pub old_status: Option<String>,
    pub new_status: String,
    pub changed_at: DateTime<Utc>,
}

pub struct ApiRecentChange {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub old_status: String,
    pub new_status: String,
    pub changed_at: DateTime<Utc>,
}

pub struct ApiRecentAddition {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub status: String,
    pub raw_status_value: Option<String>,
    pub first_seen_at: DateTime<Utc>,
}

pub struct ApiMapItem {
    pub id: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub status: String,
}

/// Aggregate counts over all currently active chargers.
pub struct DbStats {
    pub active: i64,
    pub details_failed: i64,
    pub open_status_check_failed: i64,
    pub in_development: i64,
    pub under_construction: i64,
    pub unknown: i64,
}

// ── Scrape run read models ────────────────────────────────────────────────────

/// Summary of a single scrape run, including how many status changes it produced.
pub struct RunStats {
    pub id: i64,
    pub run_type: String,
    pub scraped_at: DateTime<Utc>,
    pub total_count: i32,
    pub details_failures: i32,
    pub status_changes_count: i64,
}

pub struct ApiScrapeRun {
    pub id: i64,
    pub country: String,
    pub scraped_at: DateTime<Utc>,
    pub total_count: i32,
}
