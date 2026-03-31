use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::coming_soon::{ComingSoonSupercharger, SiteStatus};

// ── Shared types ──────────────────────────────────────────────────────────────

pub struct StatusChange {
    pub slug: String,
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
/// Only includes chargers that have a slug (i.e. retryable).
pub async fn get_failed_detail_chargers(
    pool: &PgPool,
) -> Result<Vec<ComingSoonSupercharger>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT slug, title, latitude, longitude, status, raw_status_value \
         FROM coming_soon_superchargers \
         WHERE is_active = TRUE AND details_fetch_failed = TRUE",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ComingSoonSupercharger {
            slug: r.get("slug"),
            title: r.get("title"),
            latitude: r.get("latitude"),
            longitude: r.get("longitude"),
            status: r.get("status"),
            raw_status_value: r.get("raw_status_value"),
        })
        .collect())
}

// ── Sync helpers ──────────────────────────────────────────────────────────────

/// Returns all active chargers from the DB as a `slug → status` map.
/// Used by the sync layer to diff against the fresh scrape.
pub async fn get_current_statuses(
    pool: &PgPool,
) -> Result<HashMap<String, SiteStatus>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT slug, status FROM coming_soon_superchargers WHERE is_active = TRUE",
    )
    .fetch_all(pool)
    .await?;

    let mut slug_to_status = HashMap::new();
    for r in rows {
        let slug: String = r.get("slug");
        let status: SiteStatus = r.get("status");
        slug_to_status.insert(slug, status);
    }
    Ok(slug_to_status)
}

// ── API read types ────────────────────────────────────────────────────────────

pub struct ApiSupercharger {
    pub slug: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub status: String,
    pub raw_status_value: Option<String>,
    pub first_seen_at: DateTime<Utc>,
    pub last_scraped_at: DateTime<Utc>,
    pub is_active: bool,
    pub details_fetch_failed: bool,
}

pub struct ApiStatusHistory {
    pub old_status: Option<String>,
    pub new_status: String,
    pub changed_at: DateTime<Utc>,
}

pub struct ApiRecentChange {
    pub slug: String,
    pub title: String,
    pub old_status: String,
    pub new_status: String,
    pub changed_at: DateTime<Utc>,
}

pub struct ApiRecentAddition {
    pub slug: String,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub status: String,
    pub raw_status_value: Option<String>,
    pub first_seen_at: DateTime<Utc>,
}

pub struct ApiScrapeRun {
    pub id: i64,
    pub country: String,
    pub scraped_at: DateTime<Utc>,
    pub total_count: i32,
}

// ── API read queries ──────────────────────────────────────────────────────────

/// Returns (total, items) for active coming-soon chargers, optionally filtered by status.
/// `status_filter` must already be uppercased and validated (e.g. "IN_DEVELOPMENT").
pub async fn list_coming_soon(
    pool: &PgPool,
    status_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<ApiSupercharger>), sqlx::Error> {
    let (total, rows) = if let Some(status) = status_filter {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM coming_soon_superchargers \
             WHERE is_active = true AND status = $1::site_status",
        )
        .bind(status)
        .fetch_one(pool)
        .await?;

        let rows = sqlx::query(
            "SELECT slug, title, latitude, longitude, status::text AS status, \
                    raw_status_value, first_seen_at, last_scraped_at, \
                    is_active, details_fetch_failed \
             FROM coming_soon_superchargers \
             WHERE is_active = true AND status = $1::site_status \
             ORDER BY title \
             LIMIT $2 OFFSET $3",
        )
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        (total, rows)
    } else {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM coming_soon_superchargers WHERE is_active = true",
        )
        .fetch_one(pool)
        .await?;

        let rows = sqlx::query(
            "SELECT slug, title, latitude, longitude, status::text AS status, \
                    raw_status_value, first_seen_at, last_scraped_at, \
                    is_active, details_fetch_failed \
             FROM coming_soon_superchargers \
             WHERE is_active = true \
             ORDER BY title \
             LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        (total, rows)
    };

    let items = rows
        .into_iter()
        .map(|r| ApiSupercharger {
            slug: r.get("slug"),
            title: r.get("title"),
            latitude: r.get("latitude"),
            longitude: r.get("longitude"),
            status: r.get("status"),
            raw_status_value: r.get("raw_status_value"),
            first_seen_at: r.get("first_seen_at"),
            last_scraped_at: r.get("last_scraped_at"),
            is_active: r.get("is_active"),
            details_fetch_failed: r.get("details_fetch_failed"),
        })
        .collect();

    Ok((total, items))
}

/// Returns counts grouped by status for active chargers.
pub async fn count_coming_soon_by_status(
    pool: &PgPool,
) -> Result<HashMap<String, i64>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT status::text AS status, COUNT(*) AS cnt \
         FROM coming_soon_superchargers \
         WHERE is_active = true \
         GROUP BY status",
    )
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<String, i64> = HashMap::new();
    for row in rows {
        let status: String = row.get("status");
        let cnt: i64 = row.get("cnt");
        map.insert(status, cnt);
    }
    Ok(map)
}

/// Returns the most recent scrape run's `scraped_at`, or `None` if no runs exist.
pub async fn latest_scrape_run_time(pool: &PgPool) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar("SELECT scraped_at FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1")
        .fetch_optional(pool)
        .await
}

/// Returns a single charger by slug (active or inactive), or `None` if not found.
pub async fn get_coming_soon(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<ApiSupercharger>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT slug, title, latitude, longitude, status::text AS status, \
                raw_status_value, first_seen_at, last_scraped_at, \
                is_active, details_fetch_failed \
         FROM coming_soon_superchargers \
         WHERE slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ApiSupercharger {
        slug: r.get("slug"),
        title: r.get("title"),
        latitude: r.get("latitude"),
        longitude: r.get("longitude"),
        status: r.get("status"),
        raw_status_value: r.get("raw_status_value"),
        first_seen_at: r.get("first_seen_at"),
        last_scraped_at: r.get("last_scraped_at"),
        is_active: r.get("is_active"),
        details_fetch_failed: r.get("details_fetch_failed"),
    }))
}

/// Returns the status change history for a single charger, ordered by `changed_at` ASC.
pub async fn get_status_history(
    pool: &PgPool,
    slug: &str,
) -> Result<Vec<ApiStatusHistory>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT old_status::text AS old_status, new_status::text AS new_status, changed_at \
         FROM status_changes \
         WHERE slug = $1 \
         ORDER BY changed_at ASC",
    )
    .bind(slug)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ApiStatusHistory {
            old_status: r.get("old_status"),
            new_status: r.get("new_status"),
            changed_at: r.get("changed_at"),
        })
        .collect())
}

/// Returns (total, items) for recent status transitions (excluding first appearances).
pub async fn list_recent_changes(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<ApiRecentChange>), sqlx::Error> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM status_changes WHERE old_status IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query(
        "SELECT sc.old_status::text AS old_status, sc.new_status::text AS new_status, \
                sc.changed_at, cs.title, cs.slug \
         FROM status_changes sc \
         JOIN coming_soon_superchargers cs ON cs.slug = sc.slug \
         WHERE sc.old_status IS NOT NULL \
         ORDER BY sc.changed_at DESC \
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let items = rows
        .into_iter()
        .map(|r| ApiRecentChange {
            slug: r.get("slug"),
            title: r.get("title"),
            old_status: r.get("old_status"),
            new_status: r.get("new_status"),
            changed_at: r.get("changed_at"),
        })
        .collect();

    Ok((total, items))
}

/// Returns (total, items) for recently first-seen active chargers.
pub async fn list_recent_additions(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<ApiRecentAddition>), sqlx::Error> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM coming_soon_superchargers WHERE is_active = true",
    )
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query(
        "SELECT slug, title, latitude, longitude, status::text AS status, \
                raw_status_value, first_seen_at \
         FROM coming_soon_superchargers \
         WHERE is_active = true \
         ORDER BY first_seen_at DESC \
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let items = rows
        .into_iter()
        .map(|r| ApiRecentAddition {
            slug: r.get("slug"),
            title: r.get("title"),
            latitude: r.get("latitude"),
            longitude: r.get("longitude"),
            status: r.get("status"),
            raw_status_value: r.get("raw_status_value"),
            first_seen_at: r.get("first_seen_at"),
        })
        .collect();

    Ok((total, items))
}

/// Returns recent scrape runs ordered by `scraped_at` DESC.
pub async fn list_scrape_runs(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<ApiScrapeRun>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, country, scraped_at, COALESCE(total_count, 0) AS total_count \
         FROM scrape_runs \
         ORDER BY scraped_at DESC \
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ApiScrapeRun {
            id: r.get("id"),
            country: r.get("country"),
            scraped_at: r.get("scraped_at"),
            total_count: r.get("total_count"),
        })
        .collect())
}

// ── Charger write operations ──────────────────────────────────────────────────

pub async fn save_chargers(
    pool: &PgPool,
    upserts: &[ComingSoonSupercharger],
    unchanged_slugs: &[String],
    status_changes: &[StatusChange],
    disappeared_slugs: &[String],
    scrape_run_id: i64,
    failed_detail_slugs: &std::collections::HashSet<String>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Full upsert for new or changed chargers
    if !upserts.is_empty() {
        let slugs: Vec<String> = upserts.iter().map(|c| c.slug.clone()).collect();
        let titles: Vec<String> = upserts.iter().map(|c| c.title.clone()).collect();
        let lats: Vec<f64> = upserts.iter().map(|c| c.latitude).collect();
        let lons: Vec<f64> = upserts.iter().map(|c| c.longitude).collect();
        let statuses: Vec<SiteStatus> = upserts.iter().map(|c| c.status.clone()).collect();
        let raw_vals: Vec<Option<String>> = upserts.iter().map(|c| c.raw_status_value.clone()).collect();
        let fetch_failed: Vec<bool> = upserts
            .iter()
            .map(|c| failed_detail_slugs.contains(&c.slug))
            .collect();

        sqlx::query(
            r#"
            INSERT INTO coming_soon_superchargers
                (slug, title, latitude, longitude, status, raw_status_value, details_fetch_failed, last_scraped_at, is_active)
            SELECT
                unnest($1::text[]),
                unnest($2::text[]),
                unnest($3::float8[]),
                unnest($4::float8[]),
                unnest($5::site_status[]),
                unnest($6::text[]),
                unnest($7::bool[]),
                NOW(),
                TRUE
            ON CONFLICT (slug) DO UPDATE SET
                title                = EXCLUDED.title,
                latitude             = EXCLUDED.latitude,
                longitude            = EXCLUDED.longitude,
                status               = EXCLUDED.status,
                raw_status_value     = EXCLUDED.raw_status_value,
                details_fetch_failed = EXCLUDED.details_fetch_failed,
                last_scraped_at      = EXCLUDED.last_scraped_at,
                is_active            = TRUE
            "#,
        )
        .bind(slugs)
        .bind(titles)
        .bind(lats)
        .bind(lons)
        .bind(statuses)
        .bind(raw_vals)
        .bind(fetch_failed)
        .execute(&mut *tx)
        .await?;
    }

    // Touch last_scraped_at and update details_fetch_failed for unchanged chargers.
    // The flag is computed in SQL: true if the charger's slug is in the failed set.
    if !unchanged_slugs.is_empty() {
        let failed_slugs_vec: Vec<String> = failed_detail_slugs.iter().cloned().collect();
        sqlx::query(
            "UPDATE coming_soon_superchargers \
             SET last_scraped_at = NOW(), \
                 details_fetch_failed = (slug = ANY($2::text[])) \
             WHERE slug = ANY($1)",
        )
        .bind(unchanged_slugs)
        .bind(failed_slugs_vec)
        .execute(&mut *tx)
        .await?;
    }

    // Bulk-insert status change events
    if !status_changes.is_empty() {
        let sc_slugs: Vec<String> = status_changes.iter().map(|sc| sc.slug.clone()).collect();
        let old_statuses: Vec<Option<SiteStatus>> = status_changes.iter().map(|sc| sc.old_status.clone()).collect();
        let new_statuses: Vec<SiteStatus> = status_changes.iter().map(|sc| sc.new_status.clone()).collect();

        sqlx::query(
            "INSERT INTO status_changes (slug, scrape_run_id, old_status, new_status) \
             SELECT unnest($1::text[]), $2::bigint, unnest($3::site_status[]), unnest($4::site_status[])",
        )
        .bind(sc_slugs)
        .bind(scrape_run_id)
        .bind(old_statuses)
        .bind(new_statuses)
        .execute(&mut *tx)
        .await?;
    }

    // Mark chargers absent from the latest scrape as inactive
    if !disappeared_slugs.is_empty() {
        sqlx::query(
            "UPDATE coming_soon_superchargers SET is_active = FALSE WHERE slug = ANY($1)",
        )
        .bind(disappeared_slugs)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
