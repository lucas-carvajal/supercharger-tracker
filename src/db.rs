use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::coming_soon::{ComingSoonSupercharger, SiteStatus};

// ── Shared types ──────────────────────────────────────────────────────────────

pub struct StatusChange {
    pub supercharger_uuid: String,
    pub old_status: Option<SiteStatus>,
    pub new_status: SiteStatus,
}

/// Summary of a single scrape run, including how many status changes it produced.
pub struct RunStats {
    pub id: i64,
    pub run_type: String,
    pub scraped_at: DateTime<Utc>,
    pub total_count: i32,
    pub details_failures: i32,
    pub status_changes_count: i64,
}

/// Aggregate counts over all currently active chargers.
pub struct DbStats {
    pub active: i64,
    pub details_failed: i64,
    pub in_development: i64,
    pub under_construction: i64,
    pub unknown: i64,
}

// ── Connection ────────────────────────────────────────────────────────────────

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPool::connect(database_url).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

// ── Scrape-run tracking ───────────────────────────────────────────────────────

pub async fn record_scrape_run(
    pool: &PgPool,
    country: &str,
    total_count: i32,
    details_failures: i32,
    run_type: &str,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "INSERT INTO scrape_runs (country, total_count, details_failures, run_type) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(country)
    .bind(total_count)
    .bind(details_failures)
    .bind(run_type)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

// ── Status queries ────────────────────────────────────────────────────────────

/// Returns stats for the most recent scrape run, or `None` if no runs exist yet.
pub async fn get_last_run_stats(pool: &PgPool) -> Result<Option<RunStats>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT r.id, r.run_type, r.scraped_at, r.total_count, r.details_failures, \
                (SELECT COUNT(*) FROM status_changes sc WHERE sc.scrape_run_id = r.id) \
                    AS status_changes_count \
         FROM scrape_runs r \
         ORDER BY r.scraped_at DESC \
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| RunStats {
        id: r.get("id"),
        run_type: r.get("run_type"),
        scraped_at: r.get("scraped_at"),
        total_count: r.get("total_count"),
        details_failures: r.get("details_failures"),
        status_changes_count: r.get("status_changes_count"),
    }))
}

/// Returns aggregate counts over all currently active chargers.
pub async fn get_current_db_stats(pool: &PgPool) -> Result<DbStats, sqlx::Error> {
    let row = sqlx::query(
        "SELECT \
            COUNT(*)                                                AS active, \
            COUNT(*) FILTER (WHERE details_fetch_failed = TRUE)    AS details_failed, \
            COUNT(*) FILTER (WHERE status = 'IN_DEVELOPMENT')      AS in_development, \
            COUNT(*) FILTER (WHERE status = 'UNDER_CONSTRUCTION')  AS under_construction, \
            COUNT(*) FILTER (WHERE status = 'UNKNOWN')             AS unknown \
         FROM coming_soon_superchargers \
         WHERE is_active = TRUE",
    )
    .fetch_one(pool)
    .await?;

    Ok(DbStats {
        active: row.get("active"),
        details_failed: row.get("details_failed"),
        in_development: row.get("in_development"),
        under_construction: row.get("under_construction"),
        unknown: row.get("unknown"),
    })
}

/// Returns all active chargers where the last details fetch failed.
/// Only includes chargers that have a `location_url_slug` (i.e. retryable).
pub async fn get_failed_detail_chargers(
    pool: &PgPool,
) -> Result<Vec<ComingSoonSupercharger>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT uuid, title, latitude, longitude, status, location_url_slug, raw_status_value \
         FROM coming_soon_superchargers \
         WHERE is_active = TRUE AND details_fetch_failed = TRUE AND location_url_slug IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ComingSoonSupercharger {
            uuid: r.get("uuid"),
            title: r.get("title"),
            latitude: r.get("latitude"),
            longitude: r.get("longitude"),
            status: r.get("status"),
            location_url_slug: r.get("location_url_slug"),
            raw_status_value: r.get("raw_status_value"),
        })
        .collect())
}

// ── Sync helpers ──────────────────────────────────────────────────────────────

/// Returns all active chargers from the DB. Used by the sync layer to diff
/// against the fresh scrape and detect new, changed, and disappeared chargers.
pub async fn get_current_statuses(
    pool: &PgPool,
) -> Result<HashMap<String, SiteStatus>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT uuid, status FROM coming_soon_superchargers WHERE is_active = TRUE",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.get("uuid"), r.get("status")))
        .collect())
}

pub async fn save_chargers(
    pool: &PgPool,
    upserts: &[ComingSoonSupercharger],
    unchanged_uuids: &[String],
    status_changes: &[StatusChange],
    disappeared_uuids: &[String],
    scrape_run_id: i64,
    failed_detail_slugs: &HashSet<String>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Full upsert for new or changed chargers
    if !upserts.is_empty() {
        let uuids: Vec<String> = upserts.iter().map(|c| c.uuid.clone()).collect();
        let titles: Vec<String> = upserts.iter().map(|c| c.title.clone()).collect();
        let lats: Vec<f64> = upserts.iter().map(|c| c.latitude).collect();
        let lons: Vec<f64> = upserts.iter().map(|c| c.longitude).collect();
        let statuses: Vec<SiteStatus> = upserts.iter().map(|c| c.status.clone()).collect();
        let slugs: Vec<Option<String>> = upserts.iter().map(|c| c.location_url_slug.clone()).collect();
        let raw_vals: Vec<Option<String>> = upserts.iter().map(|c| c.raw_status_value.clone()).collect();
        let fetch_failed: Vec<bool> = upserts
            .iter()
            .map(|c| c.location_url_slug.as_deref().map_or(false, |s| failed_detail_slugs.contains(s)))
            .collect();

        sqlx::query(
            r#"
            INSERT INTO coming_soon_superchargers
                (uuid, title, latitude, longitude, status, location_url_slug, raw_status_value, details_fetch_failed, last_scraped_at, is_active)
            SELECT
                unnest($1::text[]),
                unnest($2::text[]),
                unnest($3::float8[]),
                unnest($4::float8[]),
                unnest($5::site_status[]),
                unnest($6::text[]),
                unnest($7::text[]),
                unnest($8::bool[]),
                NOW(),
                TRUE
            ON CONFLICT (uuid) DO UPDATE SET
                title                = EXCLUDED.title,
                latitude             = EXCLUDED.latitude,
                longitude            = EXCLUDED.longitude,
                status               = EXCLUDED.status,
                location_url_slug    = EXCLUDED.location_url_slug,
                raw_status_value     = EXCLUDED.raw_status_value,
                details_fetch_failed = EXCLUDED.details_fetch_failed,
                last_scraped_at      = EXCLUDED.last_scraped_at,
                is_active            = TRUE
            "#,
        )
        .bind(uuids)
        .bind(titles)
        .bind(lats)
        .bind(lons)
        .bind(statuses)
        .bind(slugs)
        .bind(raw_vals)
        .bind(fetch_failed)
        .execute(&mut *tx)
        .await?;
    }

    // Touch last_scraped_at and update details_fetch_failed for unchanged chargers.
    // The flag is computed in SQL: true if the charger's slug is in the failed set.
    if !unchanged_uuids.is_empty() {
        let failed_slugs_vec: Vec<String> = failed_detail_slugs.iter().cloned().collect();
        sqlx::query(
            "UPDATE coming_soon_superchargers \
             SET last_scraped_at = NOW(), \
                 details_fetch_failed = (location_url_slug = ANY($2::text[])) \
             WHERE uuid = ANY($1)",
        )
        .bind(unchanged_uuids)
        .bind(failed_slugs_vec)
        .execute(&mut *tx)
        .await?;
    }

    // Bulk-insert status change events
    if !status_changes.is_empty() {
        let sc_uuids: Vec<String> = status_changes.iter().map(|sc| sc.supercharger_uuid.clone()).collect();
        let old_statuses: Vec<Option<SiteStatus>> = status_changes.iter().map(|sc| sc.old_status.clone()).collect();
        let new_statuses: Vec<SiteStatus> = status_changes.iter().map(|sc| sc.new_status.clone()).collect();

        sqlx::query(
            "INSERT INTO status_changes (supercharger_uuid, scrape_run_id, old_status, new_status) \
             SELECT unnest($1::text[]), $2::bigint, unnest($3::site_status[]), unnest($4::site_status[])",
        )
        .bind(sc_uuids)
        .bind(scrape_run_id)
        .bind(old_statuses)
        .bind(new_statuses)
        .execute(&mut *tx)
        .await?;
    }

    // Mark chargers absent from the latest scrape as inactive
    if !disappeared_uuids.is_empty() {
        sqlx::query(
            "UPDATE coming_soon_superchargers SET is_active = FALSE WHERE uuid = ANY($1)",
        )
        .bind(disappeared_uuids)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
