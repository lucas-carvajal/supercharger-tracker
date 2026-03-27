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

pub async fn get_current_statuses(
    pool: &PgPool,
    uuids: &[String],
) -> Result<HashMap<String, SiteStatus>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT uuid, status FROM coming_soon_superchargers WHERE uuid = ANY($1)",
    )
    .bind(uuids)
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
    status_changes: &[StatusChange],
    opened_uuids: &[String],
    scrape_run_id: i64,
) -> Result<(), sqlx::Error> {
    for c in upserts {
        sqlx::query(
            r#"
            INSERT INTO coming_soon_superchargers
                (uuid, title, latitude, longitude, status, location_url_slug, raw_status_value, last_scraped_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
            ON CONFLICT (uuid) DO UPDATE SET
                title             = EXCLUDED.title,
                latitude          = EXCLUDED.latitude,
                longitude         = EXCLUDED.longitude,
                status            = EXCLUDED.status,
                location_url_slug = EXCLUDED.location_url_slug,
                raw_status_value  = EXCLUDED.raw_status_value,
                last_scraped_at   = EXCLUDED.last_scraped_at
            "#,
        )
        .bind(&c.uuid)
        .bind(&c.title)
        .bind(c.latitude)
        .bind(c.longitude)
        .bind(&c.status)
        .bind(&c.location_url_slug)
        .bind(&c.raw_status_value)
        .execute(pool)
        .await?;
    }

    for sc in status_changes {
        sqlx::query(
            "INSERT INTO status_changes (supercharger_uuid, scrape_run_id, old_status, new_status) VALUES ($1, $2, $3, $4)",
        )
        .bind(&sc.supercharger_uuid)
        .bind(scrape_run_id)
        .bind(&sc.old_status)
        .bind(&sc.new_status)
        .execute(pool)
        .await?;
    }

    if !opened_uuids.is_empty() {
        sqlx::query(
            "UPDATE coming_soon_superchargers SET opened_at = NOW() WHERE uuid = ANY($1)",
        )
        .bind(opened_uuids)
        .execute(pool)
        .await?;
    }

    Ok(())
}
