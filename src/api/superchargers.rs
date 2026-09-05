use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::api::regions;
use crate::api::{ApiError, AppState};
use crate::domain::SiteStatus;

// ── Query param structs ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SuperchargerItem {
    /// Stable system identifier — the Tesla location URL slug.
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub status: String,
    pub raw_status_value: Option<String>,
    pub raw_project_status: Option<String>,
    pub num_charger_stalls: i32,
    pub charging_accessibility: Option<String>,
    pub tesla_url: String,
    pub first_seen_at: DateTime<Utc>,
    pub last_scraped_at: DateTime<Utc>,
    pub details_fetch_failed: bool,
}

#[derive(Serialize)]
pub struct ListResponse {
    pub total: i64,
    pub items: Vec<SuperchargerItem>,
}

#[derive(Serialize)]
pub struct StatsResponse {
    pub total_active: i64,
    pub by_status: HashMap<String, i64>,
    pub as_of: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct StatusHistoryEntry {
    pub old_status: Option<String>,
    pub new_status: String,
    pub changed_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct DetailResponse {
    /// Stable system identifier — the Tesla location URL slug.
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub status: String,
    pub raw_status_value: Option<String>,
    pub raw_project_status: Option<String>,
    pub num_charger_stalls: i32,
    pub charging_accessibility: Option<String>,
    pub tesla_url: String,
    pub first_seen_at: DateTime<Utc>,
    pub last_scraped_at: DateTime<Utc>,
    pub details_fetch_failed: bool,
    pub status_history: Vec<StatusHistoryEntry>,
}

#[derive(Serialize)]
pub struct RecentChangeItem {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub old_status: String,
    pub new_status: String,
    pub changed_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct RecentChangesResponse {
    pub total: i64,
    pub items: Vec<RecentChangeItem>,
}

#[derive(Serialize)]
pub struct RecentAdditionItem {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub status: String,
    pub raw_status_value: Option<String>,
    pub tesla_url: String,
    pub first_seen_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct RecentAdditionsResponse {
    pub total: i64,
    pub items: Vec<RecentAdditionItem>,
}

#[derive(Serialize)]
pub struct RecentUpdateItem {
    pub id: String,
    pub title: String,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub old_status: Option<String>,
    pub new_status: String,
    pub changed_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct RecentUpdatesResponse {
    pub total: i64,
    pub items: Vec<RecentUpdateItem>,
}

#[derive(Serialize)]
pub struct MapItem {
    pub id: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
    pub status: String,
    /// Stall count; `0` means unknown / not yet published by Tesla.
    pub num_charger_stalls: i32,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tesla_url(id: &str) -> String {
    format!("https://www.tesla.com/findus?location={id}")
}

fn page(params: &PaginationQuery) -> (i64, i64) {
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);
    (limit, offset)
}

fn status_api(status: &SiteStatus) -> String {
    status.as_db_str().to_string()
}

fn validate_status(s: &str) -> Option<String> {
    let upper = s.to_uppercase();
    match upper.as_str() {
        "PRELIMINARY" | "DESIGN" | "CONSTRUCTION" | "UNKNOWN" => Some(upper),
        _ => None,
    }
}

fn parse_country_query(raw: &str) -> Result<String, ApiError> {
    if raw.len() == 2 && raw.bytes().all(|b| b.is_ascii_alphabetic()) {
        Ok(raw.to_ascii_uppercase())
    } else {
        Err(ApiError::BadRequest(format!("invalid country: {raw}")))
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /superchargers/soon/map
pub async fn map_handler(State(state): State<AppState>) -> Result<Json<Vec<MapItem>>, ApiError> {
    let rows = state.supercharger.list_coming_soon_map_items().await?;

    let items = rows
        .into_iter()
        .map(|r| MapItem {
            id: r.id,
            title: r.title,
            latitude: r.latitude,
            longitude: r.longitude,
            country: r.country,
            status: r.status,
            num_charger_stalls: r.num_charger_stalls,
        })
        .collect();

    Ok(Json(items))
}

/// GET /superchargers/soon
pub async fn list_handler(
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let limit = params.limit.unwrap_or(200).clamp(1, 1000);
    let offset = params.offset.unwrap_or(0).max(0);

    let status_filter = params
        .status
        .as_deref()
        .map(|s| {
            validate_status(s).ok_or_else(|| ApiError::BadRequest(format!("invalid status: {s}")))
        })
        .transpose()?;

    let region_filter: Vec<String> = match params.region.as_deref() {
        None => vec![],
        Some(r) => regions::resolve(r)
            .ok_or_else(|| ApiError::BadRequest(format!("unknown region: {r}")))?,
    };

    let country_filter = params
        .country
        .as_deref()
        .map(parse_country_query)
        .transpose()?;

    let (total, rows) = state
        .supercharger
        .list_coming_soon(
            status_filter.as_deref(),
            &region_filter,
            country_filter.as_deref(),
            limit,
            offset,
        )
        .await?;

    let items = rows
        .into_iter()
        .map(|r| SuperchargerItem {
            tesla_url: tesla_url(&r.id),
            id: r.id,
            title: r.title,
            city: r.city,
            region: r.region,
            country: r.country,
            latitude: r.latitude,
            longitude: r.longitude,
            status: r.status,
            raw_status_value: r.raw_status_value,
            raw_project_status: r.raw_project_status,
            num_charger_stalls: r.num_charger_stalls,
            charging_accessibility: r.charging_accessibility,
            first_seen_at: r.first_seen_at,
            last_scraped_at: r.last_scraped_at,
            details_fetch_failed: r.details_fetch_failed,
        })
        .collect();

    Ok(Json(ListResponse { total, items }))
}

/// GET /superchargers/soon/stats
pub async fn stats_handler(State(state): State<AppState>) -> Result<Json<StatsResponse>, ApiError> {
    let counts = state.supercharger.count_coming_soon_by_status().await?;
    let as_of = state.scrape_run.latest_scrape_run_time().await?;

    let mut by_status: HashMap<String, i64> = HashMap::new();
    for key in &["PRELIMINARY", "DESIGN", "CONSTRUCTION", "UNKNOWN"] {
        by_status.insert(key.to_string(), *counts.get(*key).unwrap_or(&0));
    }

    let total_active = by_status.values().sum();

    Ok(Json(StatsResponse {
        total_active,
        by_status,
        as_of,
    }))
}

/// GET /superchargers/soon/:id
pub async fn detail_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DetailResponse>, ApiError> {
    let charger = state
        .supercharger
        .get_coming_soon(&id)
        .await?
        .ok_or_else(|| ApiError::NotFound("supercharger not found".to_string()))?;

    let history = state.supercharger.get_status_history(&id).await?;

    let status_history = history
        .into_iter()
        .map(|h| StatusHistoryEntry {
            old_status: h.old_status,
            new_status: h.new_status,
            changed_at: h.changed_at,
        })
        .collect();

    Ok(Json(DetailResponse {
        tesla_url: tesla_url(&charger.id),
        id: charger.id,
        title: charger.title,
        city: charger.city,
        region: charger.region,
        country: charger.country,
        latitude: charger.latitude,
        longitude: charger.longitude,
        status: charger.status,
        raw_status_value: charger.raw_status_value,
        raw_project_status: charger.raw_project_status,
        num_charger_stalls: charger.num_charger_stalls,
        charging_accessibility: charger.charging_accessibility,
        first_seen_at: charger.first_seen_at,
        last_scraped_at: charger.last_scraped_at,
        details_fetch_failed: charger.details_fetch_failed,
        status_history,
    }))
}

/// GET /superchargers/soon/recent-changes
pub async fn recent_changes_handler(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Result<Json<RecentChangesResponse>, ApiError> {
    let (limit, offset) = page(&params);
    let (total, rows) = state
        .supercharger
        .list_recent_changes(limit, offset)
        .await?;

    let items = rows
        .into_iter()
        .map(|e| RecentChangeItem {
            id: e.id,
            title: e.title,
            city: e.city,
            region: e.region,
            country: e.country,
            // Changes WHERE requires old_status IS NOT NULL.
            old_status: status_api(
                e.old_status
                    .as_ref()
                    .expect("changes feed excludes null old_status"),
            ),
            new_status: status_api(&e.new_status),
            changed_at: e.changed_at,
        })
        .collect();

    Ok(Json(RecentChangesResponse { total, items }))
}

/// GET /superchargers/soon/recent-updates
pub async fn recent_updates_handler(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Result<Json<RecentUpdatesResponse>, ApiError> {
    let (limit, offset) = page(&params);
    let (total, rows) = state
        .supercharger
        .list_recent_updates(limit, offset)
        .await?;

    let items = rows
        .into_iter()
        .map(|e| RecentUpdateItem {
            id: e.id,
            title: e.title,
            city: e.city,
            region: e.region,
            country: e.country,
            old_status: e.old_status.as_ref().map(status_api),
            new_status: status_api(&e.new_status),
            changed_at: e.changed_at,
        })
        .collect();

    Ok(Json(RecentUpdatesResponse { total, items }))
}

/// GET /superchargers/soon/recent-additions
pub async fn recent_additions_handler(
    State(state): State<AppState>,
    Query(params): Query<PaginationQuery>,
) -> Result<Json<RecentAdditionsResponse>, ApiError> {
    let (limit, offset) = page(&params);
    let (total, rows) = state
        .supercharger
        .list_recent_additions(limit, offset)
        .await?;

    let items = rows
        .into_iter()
        .map(|r| RecentAdditionItem {
            tesla_url: tesla_url(&r.id),
            id: r.id,
            title: r.title,
            city: r.city,
            region: r.region,
            country: r.country,
            latitude: r.latitude,
            longitude: r.longitude,
            status: r.status,
            raw_status_value: r.raw_status_value,
            first_seen_at: r.first_seen_at,
        })
        .collect();

    Ok(Json(RecentAdditionsResponse { total, items }))
}

#[cfg(test)]
mod tests {
    use super::parse_country_query;

    #[test]
    fn parse_country_query_accepts_two_ascii_letters() {
        assert_eq!(parse_country_query("gb").ok().as_deref(), Some("GB"));
        assert_eq!(parse_country_query("US").ok().as_deref(), Some("US"));
        assert_eq!(parse_country_query("ZZ").ok().as_deref(), Some("ZZ"));
    }

    #[test]
    fn parse_country_query_rejects_non_iso2() {
        assert!(parse_country_query("UK ").is_err());
        assert!(parse_country_query("germany").is_err());
        assert!(parse_country_query("U").is_err());
        assert!(parse_country_query("US-TX").is_err());
        assert!(parse_country_query("12").is_err());
    }
}
