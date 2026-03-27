use std::collections::HashMap;

use sqlx::{PgPool, Row};

use crate::coming_soon::{ComingSoonSupercharger, SiteStatus};

pub struct StatusChange {
    pub supercharger_uuid: String,
    pub old_status: Option<SiteStatus>,
    pub new_status: SiteStatus,
}

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPool::connect(database_url).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

pub async fn record_scrape_run(
    pool: &PgPool,
    country: &str,
    total_count: i32,
    error: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "INSERT INTO scrape_runs (country, total_count, error) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(country)
    .bind(total_count)
    .bind(error)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

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

        sqlx::query(
            r#"
            INSERT INTO coming_soon_superchargers
                (uuid, title, latitude, longitude, status, location_url_slug, raw_status_value, last_scraped_at, is_active)
            SELECT
                unnest($1::text[]),
                unnest($2::text[]),
                unnest($3::float8[]),
                unnest($4::float8[]),
                unnest($5::site_status[]),
                unnest($6::text[]),
                unnest($7::text[]),
                NOW(),
                TRUE
            ON CONFLICT (uuid) DO UPDATE SET
                title             = EXCLUDED.title,
                latitude          = EXCLUDED.latitude,
                longitude         = EXCLUDED.longitude,
                status            = EXCLUDED.status,
                location_url_slug = EXCLUDED.location_url_slug,
                raw_status_value  = EXCLUDED.raw_status_value,
                last_scraped_at   = EXCLUDED.last_scraped_at,
                is_active         = TRUE
            "#,
        )
        .bind(uuids)
        .bind(titles)
        .bind(lats)
        .bind(lons)
        .bind(statuses)
        .bind(slugs)
        .bind(raw_vals)
        .execute(&mut *tx)
        .await?;
    }

    // Touch last_scraped_at for unchanged chargers
    if !unchanged_uuids.is_empty() {
        sqlx::query(
            "UPDATE coming_soon_superchargers SET last_scraped_at = NOW() WHERE uuid = ANY($1)",
        )
        .bind(unchanged_uuids)
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
