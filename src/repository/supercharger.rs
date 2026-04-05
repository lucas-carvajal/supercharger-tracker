use std::collections::HashMap;

use sqlx::{PgPool, Row};

use crate::domain::{ChargerCategory, ComingSoonSupercharger, OpenResult, SiteStatus, StatusChange};
use super::models::{ApiRecentAddition, ApiRecentChange, ApiStatusHistory, ApiSupercharger, DbStats};

// ── Repository ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SuperchargerRepository {
    pool: PgPool,
}

impl SuperchargerRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ── Scraper reads ─────────────────────────────────────────────────────────

    /// Returns ALL chargers from the DB as an `id → status` map (including REMOVED).
    /// Used by the sync layer to diff against the fresh scrape.
    /// REMOVED chargers are included so that if they reappear in the feed, a
    /// `Removed → InDevelopment` status change is recorded rather than a spurious
    /// first-appearance event.
    pub async fn get_current_statuses(&self) -> Result<HashMap<String, SiteStatus>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, status FROM coming_soon_superchargers",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| (r.get::<String, _>("id"), r.get::<SiteStatus, _>("status")))
            .collect())
    }

    /// Returns all active chargers where the last details fetch failed.
    pub async fn get_failed_detail_chargers(&self) -> Result<Vec<ComingSoonSupercharger>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, title, city, region, latitude, longitude, status, raw_status_value, charger_category \
             FROM coming_soon_superchargers \
             WHERE status != 'REMOVED' AND details_fetch_failed = TRUE",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ComingSoonSupercharger {
                id: r.get("id"),
                title: r.get("title"),
                city: r.get("city"),
                region: r.get("region"),
                latitude: r.get("latitude"),
                longitude: r.get("longitude"),
                status: r.get("status"),
                raw_status_value: r.get("raw_status_value"),
                charger_category: r.get("charger_category"),
            })
            .collect())
    }

    /// Returns all active chargers where the last open-status check failed.
    /// These disappeared from the Tesla feed but their open/removed state is unconfirmed.
    pub async fn get_failed_open_status_chargers(&self) -> Result<Vec<ComingSoonSupercharger>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, title, city, region, latitude, longitude, status, raw_status_value, charger_category \
             FROM coming_soon_superchargers \
             WHERE status != 'REMOVED' AND open_status_check_failed = TRUE",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ComingSoonSupercharger {
                id: r.get("id"),
                title: r.get("title"),
                city: r.get("city"),
                region: r.get("region"),
                latitude: r.get("latitude"),
                longitude: r.get("longitude"),
                status: r.get("status"),
                raw_status_value: r.get("raw_status_value"),
                charger_category: r.get("charger_category"),
            })
            .collect())
    }

    // ── CLI reads ─────────────────────────────────────────────────────────────

    /// Returns aggregate counts over all currently active chargers.
    pub async fn get_db_stats(&self) -> Result<DbStats, sqlx::Error> {
        let row = sqlx::query(
            "SELECT \
                COUNT(*)                                                         AS active, \
                COUNT(*) FILTER (WHERE details_fetch_failed = TRUE)              AS details_failed, \
                COUNT(*) FILTER (WHERE open_status_check_failed = TRUE)          AS open_status_check_failed, \
                COUNT(*) FILTER (WHERE status = 'IN_DEVELOPMENT')                AS in_development, \
                COUNT(*) FILTER (WHERE status = 'UNDER_CONSTRUCTION')            AS under_construction, \
                COUNT(*) FILTER (WHERE status = 'UNKNOWN')                       AS unknown \
             FROM coming_soon_superchargers \
             WHERE status != 'REMOVED'",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(DbStats {
            active: row.get("active"),
            details_failed: row.get("details_failed"),
            open_status_check_failed: row.get("open_status_check_failed"),
            in_development: row.get("in_development"),
            under_construction: row.get("under_construction"),
            unknown: row.get("unknown"),
        })
    }

    // ── Write ─────────────────────────────────────────────────────────────────

    pub async fn save_chargers(
        &self,
        upserts: &[ComingSoonSupercharger],
        unchanged: &[ComingSoonSupercharger],
        status_changes: &[StatusChange],
        removed_ids: &[String],
        open_results: &HashMap<String, OpenResult>,
        scrape_run_id: i64,
        failed_detail_ids: &std::collections::HashSet<String>,
        open_status_failed_ids: &std::collections::HashSet<String>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        // Full upsert for new or changed chargers
        if !upserts.is_empty() {
            let ids: Vec<String> = upserts.iter().map(|c| c.id.clone()).collect();
            let titles: Vec<String> = upserts.iter().map(|c| c.title.clone()).collect();
            let cities: Vec<Option<String>> = upserts.iter().map(|c| c.city.clone()).collect();
            let regions: Vec<Option<String>> = upserts.iter().map(|c| c.region.clone()).collect();
            let lats: Vec<f64> = upserts.iter().map(|c| c.latitude).collect();
            let lons: Vec<f64> = upserts.iter().map(|c| c.longitude).collect();
            let statuses: Vec<SiteStatus> = upserts.iter().map(|c| c.status.clone()).collect();
            let raw_vals: Vec<Option<String>> = upserts.iter().map(|c| c.raw_status_value.clone()).collect();
            let fetch_failed: Vec<bool> = upserts
                .iter()
                .map(|c| failed_detail_ids.contains(&c.id))
                .collect();
            let open_check_failed: Vec<bool> = upserts
                .iter()
                .map(|c| open_status_failed_ids.contains(&c.id))
                .collect();
            let categories: Vec<ChargerCategory> = upserts.iter().map(|c| c.charger_category.clone()).collect();

            sqlx::query(
                r#"
                INSERT INTO coming_soon_superchargers
                    (id, title, city, region, latitude, longitude, status, raw_status_value, details_fetch_failed, open_status_check_failed, last_scraped_at, charger_category)
                SELECT
                    unnest($1::text[]),
                    unnest($2::text[]),
                    unnest($3::text[]),
                    unnest($4::text[]),
                    unnest($5::float8[]),
                    unnest($6::float8[]),
                    unnest($7::site_status[]),
                    unnest($8::text[]),
                    unnest($9::bool[]),
                    unnest($10::bool[]),
                    NOW(),
                    unnest($11::charger_category[])
                ON CONFLICT (id) DO UPDATE SET
                    title                    = CASE WHEN EXCLUDED.details_fetch_failed
                                                   THEN coming_soon_superchargers.title
                                                   ELSE EXCLUDED.title END,
                    city                     = CASE WHEN EXCLUDED.details_fetch_failed
                                                   THEN coming_soon_superchargers.city
                                                   ELSE EXCLUDED.city END,
                    region                   = CASE WHEN EXCLUDED.details_fetch_failed
                                                   THEN coming_soon_superchargers.region
                                                   ELSE EXCLUDED.region END,
                    latitude                 = EXCLUDED.latitude,
                    longitude                = EXCLUDED.longitude,
                    status                   = EXCLUDED.status,
                    raw_status_value         = EXCLUDED.raw_status_value,
                    details_fetch_failed     = EXCLUDED.details_fetch_failed,
                    open_status_check_failed = EXCLUDED.open_status_check_failed,
                    last_scraped_at          = EXCLUDED.last_scraped_at,
                    charger_category         = EXCLUDED.charger_category
                "#,
            )
            .bind(ids)
            .bind(titles)
            .bind(cities)
            .bind(regions)
            .bind(lats)
            .bind(lons)
            .bind(statuses)
            .bind(raw_vals)
            .bind(fetch_failed)
            .bind(open_check_failed)
            .bind(categories)
            .execute(&mut *tx)
            .await?;
        }

        // Update title/city/region and touch last_scraped_at for unchanged chargers.
        if !unchanged.is_empty() {
            let ids: Vec<String> = unchanged.iter().map(|c| c.id.clone()).collect();
            let titles: Vec<String> = unchanged.iter().map(|c| c.title.clone()).collect();
            let cities: Vec<Option<String>> = unchanged.iter().map(|c| c.city.clone()).collect();
            let regions: Vec<Option<String>> = unchanged.iter().map(|c| c.region.clone()).collect();
            let categories: Vec<ChargerCategory> = unchanged.iter().map(|c| c.charger_category.clone()).collect();
            let failed_ids_vec: Vec<String> = failed_detail_ids.iter().cloned().collect();
            let open_failed_ids_vec: Vec<String> = open_status_failed_ids.iter().cloned().collect();
            sqlx::query(
                "UPDATE coming_soon_superchargers AS cs \
                 SET title                    = CASE WHEN cs.id = ANY($6::text[]) THEN cs.title  ELSE v.title  END, \
                     city                     = CASE WHEN cs.id = ANY($6::text[]) THEN cs.city   ELSE v.city   END, \
                     region                   = CASE WHEN cs.id = ANY($6::text[]) THEN cs.region ELSE v.region END, \
                     charger_category         = v.charger_category, \
                     last_scraped_at          = NOW(), \
                     details_fetch_failed     = (cs.id = ANY($6::text[])), \
                     open_status_check_failed = (cs.id = ANY($7::text[])) \
                 FROM (SELECT unnest($1::text[]) AS id, \
                              unnest($2::text[]) AS title, \
                              unnest($3::text[]) AS city, \
                              unnest($4::text[]) AS region, \
                              unnest($5::charger_category[]) AS charger_category) AS v \
                 WHERE cs.id = v.id",
            )
            .bind(ids)
            .bind(titles)
            .bind(cities)
            .bind(regions)
            .bind(categories)
            .bind(failed_ids_vec)
            .bind(open_failed_ids_vec)
            .execute(&mut *tx)
            .await?;
        }

        // Bulk-insert status change events
        if !status_changes.is_empty() {
            let sc_ids: Vec<String> = status_changes.iter().map(|sc| sc.supercharger_id.clone()).collect();
            let old_statuses: Vec<Option<SiteStatus>> = status_changes.iter().map(|sc| sc.old_status.clone()).collect();
            let new_statuses: Vec<SiteStatus> = status_changes.iter().map(|sc| sc.new_status.clone()).collect();

            sqlx::query(
                "INSERT INTO status_changes (supercharger_id, scrape_run_id, old_status, new_status) \
                 SELECT unnest($1::text[]), $2::bigint, unnest($3::site_status[]), unnest($4::site_status[])",
            )
            .bind(sc_ids)
            .bind(scrape_run_id)
            .bind(old_statuses)
            .bind(new_statuses)
            .execute(&mut *tx)
            .await?;
        }

        // Mark chargers absent from the latest scrape as REMOVED
        if !removed_ids.is_empty() {
            sqlx::query(
                "UPDATE coming_soon_superchargers SET status = 'REMOVED' WHERE id = ANY($1)",
            )
            .bind(removed_ids)
            .execute(&mut *tx)
            .await?;
        }

        // For each confirmed-opened charger: copy to opened_superchargers, then delete.
        // Both happen within this transaction — if the INSERT fails, the DELETE rolls back.
        for (id, open_result) in open_results {
            let row = sqlx::query(
                "SELECT title, city, region, latitude, longitude \
                 FROM coming_soon_superchargers WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;

            let Some(row) = row else { continue };

            sqlx::query(
                "INSERT INTO opened_superchargers \
                 (id, title, city, region, latitude, longitude, opening_date, num_stalls, open_to_non_tesla) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(id)
            .bind(row.get::<String, _>("title"))
            .bind(row.get::<Option<String>, _>("city"))
            .bind(row.get::<Option<String>, _>("region"))
            .bind(row.get::<f64, _>("latitude"))
            .bind(row.get::<f64, _>("longitude"))
            .bind(open_result.opening_date)
            .bind(open_result.num_stalls)
            .bind(open_result.open_to_non_tesla)
            .execute(&mut *tx)
            .await?;

            sqlx::query("DELETE FROM coming_soon_superchargers WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    // ── API reads ─────────────────────────────────────────────────────────────

    /// Returns (total, items) for active coming-soon chargers, optionally filtered by status
    /// and/or region. `status_filter` must already be uppercased and validated (e.g.
    /// "IN_DEVELOPMENT"). `region_filter` is a list of exact DB `region` values; an empty
    /// slice means no region filter (all regions returned).
    pub async fn list_coming_soon(
        &self,
        status_filter: Option<&str>,
        region_filter: &[String],
        limit: i64,
        offset: i64,
    ) -> Result<(i64, Vec<ApiSupercharger>), sqlx::Error> {
        let (total, rows) = if let Some(status) = status_filter {
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM coming_soon_superchargers \
                 WHERE status != 'REMOVED' \
                   AND status = $1::site_status \
                   AND (cardinality($2::text[]) = 0 OR region = ANY($2::text[]))",
            )
            .bind(status)
            .bind(region_filter)
            .fetch_one(&self.pool)
            .await?;

            let rows = sqlx::query(
                "SELECT id, title, city, region, latitude, longitude, status::text AS status, \
                        raw_status_value, first_seen_at, last_scraped_at, \
                        details_fetch_failed \
                 FROM coming_soon_superchargers \
                 WHERE status != 'REMOVED' \
                   AND status = $1::site_status \
                   AND (cardinality($2::text[]) = 0 OR region = ANY($2::text[])) \
                 ORDER BY status, region \
                 LIMIT $3 OFFSET $4",
            )
            .bind(status)
            .bind(region_filter)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

            (total, rows)
        } else {
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM coming_soon_superchargers \
                 WHERE status != 'REMOVED' \
                   AND (cardinality($1::text[]) = 0 OR region = ANY($1::text[]))",
            )
            .bind(region_filter)
            .fetch_one(&self.pool)
            .await?;

            let rows = sqlx::query(
                "SELECT id, title, city, region, latitude, longitude, status::text AS status, \
                        raw_status_value, first_seen_at, last_scraped_at, \
                        details_fetch_failed \
                 FROM coming_soon_superchargers \
                 WHERE status != 'REMOVED' \
                   AND (cardinality($1::text[]) = 0 OR region = ANY($1::text[])) \
                 ORDER BY status, region \
                 LIMIT $2 OFFSET $3",
            )
            .bind(region_filter)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

            (total, rows)
        };

        let items = rows
            .into_iter()
            .map(|r| ApiSupercharger {
                id: r.get("id"),
                title: r.get("title"),
                city: r.get("city"),
                region: r.get("region"),
                latitude: r.get("latitude"),
                longitude: r.get("longitude"),
                status: r.get("status"),
                raw_status_value: r.get("raw_status_value"),
                first_seen_at: r.get("first_seen_at"),
                last_scraped_at: r.get("last_scraped_at"),
                details_fetch_failed: r.get("details_fetch_failed"),
            })
            .collect();

        Ok((total, items))
    }

    /// Returns counts grouped by status for active chargers.
    pub async fn count_coming_soon_by_status(&self) -> Result<HashMap<String, i64>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT status::text AS status, COUNT(*) AS cnt \
             FROM coming_soon_superchargers \
             WHERE status != 'REMOVED' \
             GROUP BY status",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut map: HashMap<String, i64> = HashMap::new();
        for row in rows {
            let status: String = row.get("status");
            let cnt: i64 = row.get("cnt");
            map.insert(status, cnt);
        }
        Ok(map)
    }

    /// Returns a single charger by its ID, or `None` if not found.
    pub async fn get_coming_soon(&self, id: &str) -> Result<Option<ApiSupercharger>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, title, city, region, latitude, longitude, status::text AS status, \
                    raw_status_value, first_seen_at, last_scraped_at, \
                    details_fetch_failed \
             FROM coming_soon_superchargers \
             WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ApiSupercharger {
            id: r.get("id"),
            title: r.get("title"),
            city: r.get("city"),
            region: r.get("region"),
            latitude: r.get("latitude"),
            longitude: r.get("longitude"),
            status: r.get("status"),
            raw_status_value: r.get("raw_status_value"),
            first_seen_at: r.get("first_seen_at"),
            last_scraped_at: r.get("last_scraped_at"),
            details_fetch_failed: r.get("details_fetch_failed"),
        }))
    }

    /// Returns the status change history for a single charger, ordered by `changed_at` ASC.
    pub async fn get_status_history(&self, id: &str) -> Result<Vec<ApiStatusHistory>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT old_status::text AS old_status, new_status::text AS new_status, changed_at \
             FROM status_changes \
             WHERE supercharger_id = $1 \
             ORDER BY changed_at ASC",
        )
        .bind(id)
        .fetch_all(&self.pool)
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
    /// Uses LEFT JOINs against both tables so that status changes for opened (deleted) chargers
    /// remain visible — title/city/region fall back to opened_superchargers if available.
    pub async fn list_recent_changes(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<(i64, Vec<ApiRecentChange>), sqlx::Error> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM status_changes WHERE old_status IS NOT NULL",
        )
        .fetch_one(&self.pool)
        .await?;

        let rows = sqlx::query(
            "SELECT sc.old_status::text AS old_status, sc.new_status::text AS new_status, \
                    sc.changed_at, \
                    COALESCE(cs.id, os.id, sc.supercharger_id) AS id, \
                    COALESCE(cs.title, os.title, '') AS title, \
                    COALESCE(cs.city, os.city) AS city, \
                    COALESCE(cs.region, os.region) AS region \
             FROM status_changes sc \
             LEFT JOIN coming_soon_superchargers cs ON cs.id = sc.supercharger_id \
             LEFT JOIN opened_superchargers os ON os.id = sc.supercharger_id \
             WHERE sc.old_status IS NOT NULL \
             ORDER BY sc.changed_at DESC \
             LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let items = rows
            .into_iter()
            .map(|r| ApiRecentChange {
                id: r.get("id"),
                title: r.get("title"),
                city: r.get("city"),
                region: r.get("region"),
                old_status: r.get("old_status"),
                new_status: r.get("new_status"),
                changed_at: r.get("changed_at"),
            })
            .collect();

        Ok((total, items))
    }

    /// Returns (total, items) for recently first-seen active chargers.
    pub async fn list_recent_additions(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<(i64, Vec<ApiRecentAddition>), sqlx::Error> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM coming_soon_superchargers WHERE status != 'REMOVED'",
        )
        .fetch_one(&self.pool)
        .await?;

        let rows = sqlx::query(
            "SELECT id, title, city, region, latitude, longitude, status::text AS status, \
                    raw_status_value, first_seen_at \
             FROM coming_soon_superchargers \
             WHERE status != 'REMOVED' \
             ORDER BY first_seen_at DESC \
             LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let items = rows
            .into_iter()
            .map(|r| ApiRecentAddition {
                id: r.get("id"),
                title: r.get("title"),
                city: r.get("city"),
                region: r.get("region"),
                latitude: r.get("latitude"),
                longitude: r.get("longitude"),
                status: r.get("status"),
                raw_status_value: r.get("raw_status_value"),
                first_seen_at: r.get("first_seen_at"),
            })
            .collect();

        Ok((total, items))
    }
}
