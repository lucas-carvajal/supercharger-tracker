//! JSON export/import types for transferring scrape results from local to prod.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::{ChargerCategory, SiteStatus};

/// Top-level export envelope. The `type` field discriminates between a
/// sequential diff (applied incrementally on prod) and a full snapshot
/// (used for initial setup or recovery — TRUNCATE + INSERT).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScrapeExport {
    Diff(DiffExport),
    Snapshot(SnapshotExport),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiffExport {
    /// Local `scrape_runs.id`. Prod inserts this as the row id
    /// (OVERRIDING SYSTEM VALUE) so ordering checks use `MAX(id) + 1` and
    /// dedup is a plain `WHERE id = $1`. Also used as the filename basis.
    pub run_id: i64,
    /// Timestamp of the local scrape run. Prod bulk-updates `last_scraped_at`
    /// on all non-REMOVED chargers using this value.
    pub scraped_at: DateTime<Utc>,
    pub country: String,
    /// Chargers with a status_change entry in this run (excluding OPENED — those
    /// live in `opened_chargers`). Full record for upsert.
    pub changed_chargers: Vec<ExportChangedCharger>,
    /// Every status_changes row attributed to this run, including OPENED and REMOVED.
    pub status_changes: Vec<ExportStatusChange>,
    /// Chargers that graduated this run — full opened-supercharger data for insertion.
    pub opened_chargers: Vec<ExportOpenedCharger>,
    /// Charger IDs where `new_status = 'REMOVED'`. Also present in `status_changes`;
    /// included separately so prod can apply the tombstone update in one pass.
    pub removed_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotExport {
    pub scrape_runs: Vec<ExportScrapeRun>,
    pub coming_soon_superchargers: Vec<ExportChangedCharger>,
    pub opened_superchargers: Vec<ExportOpenedCharger>,
    pub status_changes: Vec<ExportSnapshotStatusChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportChangedCharger {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub status: SiteStatus,
    pub raw_status_value: Option<String>,
    pub charger_category: ChargerCategory,
    /// Preserved so prod records the real first-seen time, not the import timestamp.
    pub first_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportStatusChange {
    pub supercharger_id: String,
    pub old_status: Option<SiteStatus>,
    pub new_status: SiteStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSnapshotStatusChange {
    pub supercharger_id: String,
    pub scrape_run_id: i64,
    pub old_status: Option<SiteStatus>,
    pub new_status: SiteStatus,
    pub changed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOpenedCharger {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub opening_date: Option<NaiveDate>,
    pub num_stalls: Option<i32>,
    pub open_to_non_tesla: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportScrapeRun {
    pub id: i64,
    pub country: String,
    pub scraped_at: DateTime<Utc>,
    pub total_count: Option<i32>,
    pub details_failures: i32,
    pub open_status_failures: i32,
    pub retry_count: i32,
    pub last_retry_at: Option<DateTime<Utc>>,
    pub run_type: String,
}
