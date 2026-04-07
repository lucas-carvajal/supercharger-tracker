use std::collections::HashMap;

use sqlx::{PgPool, Row};

use crate::domain::{ChargerCategory, ComingSoonSupercharger, OpenResult, SiteStatus, StatusChange};
use crate::export::{DiffExport, ExportChangedCharger, ExportOpenedCharger, SnapshotExport};
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

    pub fn pool(&self) -> &PgPool {
        &self.pool
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

        // Flag disappeared chargers whose open-status check failed so retry-failed picks them up.
        if !open_status_failed_ids.is_empty() {
            let failed_ids: Vec<String> = open_status_failed_ids.iter().cloned().collect();
            sqlx::query(
                "UPDATE coming_soon_superchargers SET open_status_check_failed = TRUE WHERE id = ANY($1)",
            )
            .bind(failed_ids)
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

        // For each confirmed-opened charger: record an OPENED status_change, copy to
        // opened_superchargers, then delete. All within this transaction — if any step
        // fails, everything rolls back together.
        for (id, open_result) in open_results {
            let row = sqlx::query(
                "SELECT title, city, region, latitude, longitude, status \
                 FROM coming_soon_superchargers WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;

            let Some(row) = row else { continue };

            // Record the OPENED transition in status_changes so export-diff and
            // list_recent_changes see graduations via the normal query path.
            let old_status: SiteStatus = row.get("status");
            sqlx::query(
                "INSERT INTO status_changes (supercharger_id, scrape_run_id, old_status, new_status) \
                 VALUES ($1, $2, $3, 'OPENED'::site_status)",
            )
            .bind(id)
            .bind(scrape_run_id)
            .bind(old_status)
            .execute(&mut *tx)
            .await?;

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

    // ── Export reads ──────────────────────────────────────────────────────────

    /// Returns full records for chargers that have a status_change in the given run
    /// and still exist in `coming_soon_superchargers` (i.e. not OPENED).
    pub async fn get_changed_chargers_for_run(
        &self,
        run_id: i64,
    ) -> Result<Vec<ExportChangedCharger>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, title, city, region, latitude, longitude, status, raw_status_value, \
                    charger_category, first_seen_at, last_scraped_at \
             FROM coming_soon_superchargers \
             WHERE id IN ( \
                SELECT DISTINCT supercharger_id FROM status_changes \
                WHERE scrape_run_id = $1 AND new_status NOT IN ('OPENED', 'REMOVED') \
             )",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_export_changed).collect())
    }

    /// Returns opened-supercharger rows for chargers that graduated in the given run.
    pub async fn get_opened_chargers_for_run(
        &self,
        run_id: i64,
    ) -> Result<Vec<ExportOpenedCharger>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, title, city, region, latitude, longitude, opening_date, num_stalls, open_to_non_tesla \
             FROM opened_superchargers \
             WHERE id IN ( \
                SELECT supercharger_id FROM status_changes \
                WHERE scrape_run_id = $1 AND new_status = 'OPENED' \
             )",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_export_opened).collect())
    }

    /// Returns status_changes rows for the given run, in chronological order.
    pub async fn get_status_changes_for_run(
        &self,
        run_id: i64,
    ) -> Result<Vec<crate::export::ExportStatusChange>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT supercharger_id, old_status, new_status \
             FROM status_changes WHERE scrape_run_id = $1 \
             ORDER BY changed_at ASC, id ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| crate::export::ExportStatusChange {
                supercharger_id: r.get("supercharger_id"),
                old_status: r.get("old_status"),
                new_status: r.get("new_status"),
            })
            .collect())
    }


    // ── Import writes ─────────────────────────────────────────────────────────

    /// Apply a diff export atomically: inserts the scrape_runs row and all charger
    /// changes in a single transaction. Returns `true` if the import was applied,
    /// `false` if the run_id was already present (concurrent duplicate).
    /// Caller is responsible for the ordering check before calling this.
    pub async fn save_chargers_from_diff(
        &self,
        diff: &DiffExport,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        // 0. Insert the scrape_runs row with the local id preserved.
        //    ON CONFLICT DO NOTHING handles the TOCTOU race: if two concurrent imports
        //    both pass the dedup check above, only one will insert; the other becomes a
        //    no-op and rows_affected == 0.  We treat that as a duplicate.
        let inserted = sqlx::query(
            "INSERT INTO scrape_runs (id, country, scraped_at, total_count, details_failures, \
                                      open_status_failures, run_type) \
             OVERRIDING SYSTEM VALUE \
             VALUES ($1, $2, $3, 0, 0, 0, 'import') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(diff.run_id)
        .bind(&diff.country)
        .bind(diff.scraped_at)
        .execute(&mut *tx)
        .await?
        .rows_affected() == 1;

        if !inserted {
            // Another concurrent request already committed this run_id.
            return Ok(false);
        }

        // Reset sequence so native prod runs continue from MAX(id)+1.
        sqlx::query(
            "SELECT setval('scrape_runs_id_seq', (SELECT MAX(id) FROM scrape_runs))",
        )
        .execute(&mut *tx)
        .await?;

        // 1. Upsert changed_chargers (excluding REMOVED — those are handled via removed_ids).
        //    first_seen_at is set on INSERT and never overwritten on conflict, so prod preserves
        //    the original local timestamp rather than stamping the import time.
        if !diff.changed_chargers.is_empty() {
            let ids: Vec<String> = diff.changed_chargers.iter().map(|c| c.id.clone()).collect();
            let titles: Vec<String> = diff.changed_chargers.iter().map(|c| c.title.clone()).collect();
            let cities: Vec<Option<String>> = diff.changed_chargers.iter().map(|c| c.city.clone()).collect();
            let regions: Vec<Option<String>> = diff.changed_chargers.iter().map(|c| c.region.clone()).collect();
            let lats: Vec<f64> = diff.changed_chargers.iter().map(|c| c.latitude).collect();
            let lons: Vec<f64> = diff.changed_chargers.iter().map(|c| c.longitude).collect();
            let statuses: Vec<SiteStatus> = diff.changed_chargers.iter().map(|c| c.status.clone()).collect();
            let raw_vals: Vec<Option<String>> = diff.changed_chargers.iter().map(|c| c.raw_status_value.clone()).collect();
            let categories: Vec<ChargerCategory> = diff.changed_chargers.iter().map(|c| c.charger_category.clone()).collect();
            let first_seen: Vec<chrono::DateTime<chrono::Utc>> = diff.changed_chargers.iter().map(|c| c.first_seen_at).collect();

            sqlx::query(
                "INSERT INTO coming_soon_superchargers \
                    (id, title, city, region, latitude, longitude, status, raw_status_value, \
                     last_scraped_at, charger_category, first_seen_at) \
                 SELECT unnest($1::text[]), unnest($2::text[]), unnest($3::text[]), unnest($4::text[]), \
                        unnest($5::float8[]), unnest($6::float8[]), unnest($7::site_status[]), unnest($8::text[]), \
                        $9, unnest($10::charger_category[]), unnest($11::timestamptz[]) \
                 ON CONFLICT (id) DO UPDATE SET \
                    title = EXCLUDED.title, city = EXCLUDED.city, region = EXCLUDED.region, \
                    latitude = EXCLUDED.latitude, longitude = EXCLUDED.longitude, \
                    status = EXCLUDED.status, raw_status_value = EXCLUDED.raw_status_value, \
                    last_scraped_at = EXCLUDED.last_scraped_at, charger_category = EXCLUDED.charger_category",
            )
            .bind(ids)
            .bind(titles)
            .bind(cities)
            .bind(regions)
            .bind(lats)
            .bind(lons)
            .bind(statuses)
            .bind(raw_vals)
            .bind(diff.scraped_at)
            .bind(categories)
            .bind(first_seen)
            .execute(&mut *tx)
            .await?;
        }

        // 2. Insert status_changes attributed to the imported run id.
        //    Use diff.scraped_at as changed_at so prod shows the scrape timestamp, not the
        //    import timestamp. Without this the API would report "changed on Tuesday" for a
        //    scrape that actually ran on Monday and was only imported on Tuesday.
        if !diff.status_changes.is_empty() {
            let sc_ids: Vec<String> = diff.status_changes.iter().map(|c| c.supercharger_id.clone()).collect();
            let olds: Vec<Option<SiteStatus>> = diff.status_changes.iter().map(|c| c.old_status.clone()).collect();
            let news: Vec<SiteStatus> = diff.status_changes.iter().map(|c| c.new_status.clone()).collect();
            sqlx::query(
                "INSERT INTO status_changes (supercharger_id, scrape_run_id, old_status, new_status, changed_at) \
                 SELECT unnest($1::text[]), $2::bigint, unnest($3::site_status[]), unnest($4::site_status[]), $5",
            )
            .bind(sc_ids)
            .bind(diff.run_id)
            .bind(olds)
            .bind(news)
            .bind(diff.scraped_at)
            .execute(&mut *tx)
            .await?;
        }

        // 3. Graduate opened chargers — insert into opened_superchargers, delete from coming_soon.
        for c in &diff.opened_chargers {
            sqlx::query(
                "INSERT INTO opened_superchargers \
                 (id, title, city, region, latitude, longitude, opening_date, num_stalls, open_to_non_tesla) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(&c.id)
            .bind(&c.title)
            .bind(&c.city)
            .bind(&c.region)
            .bind(c.latitude)
            .bind(c.longitude)
            .bind(c.opening_date)
            .bind(c.num_stalls)
            .bind(c.open_to_non_tesla)
            .execute(&mut *tx)
            .await?;

            sqlx::query("DELETE FROM coming_soon_superchargers WHERE id = $1")
                .bind(&c.id)
                .execute(&mut *tx)
                .await?;
        }

        // 4. Mark removed chargers as tombstones.
        if !diff.removed_ids.is_empty() {
            sqlx::query(
                "UPDATE coming_soon_superchargers SET status = 'REMOVED' WHERE id = ANY($1)",
            )
            .bind(&diff.removed_ids)
            .execute(&mut *tx)
            .await?;
        }

        // 5. Bulk-update last_scraped_at for all non-REMOVED chargers so unchanged
        //    chargers get their scraped_at refreshed without needing to list them.
        sqlx::query(
            "UPDATE coming_soon_superchargers SET last_scraped_at = $1 WHERE status != 'REMOVED'",
        )
        .bind(diff.scraped_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(true)
    }

    /// Replace the entire DB with snapshot contents: TRUNCATE four tables, then
    /// INSERT every row with its original id. Sequence on scrape_runs is reset.
    /// scrape_run_repo should insert its own seed-chain row after this call.
    pub async fn apply_snapshot(
        &self,
        snap: &SnapshotExport,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "TRUNCATE TABLE status_changes, coming_soon_superchargers, opened_superchargers, scrape_runs RESTART IDENTITY CASCADE",
        )
        .execute(&mut *tx)
        .await?;

        // scrape_runs
        for r in &snap.scrape_runs {
            sqlx::query(
                "INSERT INTO scrape_runs (id, country, scraped_at, total_count, details_failures, \
                                          open_status_failures, retry_count, last_retry_at, run_type) \
                 OVERRIDING SYSTEM VALUE \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(r.id)
            .bind(&r.country)
            .bind(r.scraped_at)
            .bind(r.total_count)
            .bind(r.details_failures)
            .bind(r.open_status_failures)
            .bind(r.retry_count)
            .bind(r.last_retry_at)
            .bind(&r.run_type)
            .execute(&mut *tx)
            .await?;
        }

        // Reset sequence to past the max id so future inserts don't collide.
        sqlx::query(
            "SELECT setval('scrape_runs_id_seq', COALESCE((SELECT MAX(id) FROM scrape_runs), 1))",
        )
        .execute(&mut *tx)
        .await?;

        // coming_soon_superchargers
        for c in &snap.coming_soon_superchargers {
            sqlx::query(
                "INSERT INTO coming_soon_superchargers \
                    (id, title, city, region, latitude, longitude, status, raw_status_value, \
                     charger_category, first_seen_at, last_scraped_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, COALESCE($11, NOW()))",
            )
            .bind(&c.id)
            .bind(&c.title)
            .bind(&c.city)
            .bind(&c.region)
            .bind(c.latitude)
            .bind(c.longitude)
            .bind(&c.status)
            .bind(&c.raw_status_value)
            .bind(&c.charger_category)
            .bind(c.first_seen_at)
            .bind(c.last_scraped_at)
            .execute(&mut *tx)
            .await?;
        }

        // opened_superchargers
        for c in &snap.opened_superchargers {
            sqlx::query(
                "INSERT INTO opened_superchargers \
                    (id, title, city, region, latitude, longitude, opening_date, num_stalls, open_to_non_tesla) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(&c.id)
            .bind(&c.title)
            .bind(&c.city)
            .bind(&c.region)
            .bind(c.latitude)
            .bind(c.longitude)
            .bind(c.opening_date)
            .bind(c.num_stalls)
            .bind(c.open_to_non_tesla)
            .execute(&mut *tx)
            .await?;
        }

        // status_changes — preserve original scrape_run_id values as audit references.
        for sc in &snap.status_changes {
            sqlx::query(
                "INSERT INTO status_changes (supercharger_id, scrape_run_id, old_status, new_status, changed_at) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&sc.supercharger_id)
            .bind(sc.scrape_run_id)
            .bind(&sc.old_status)
            .bind(&sc.new_status)
            .bind(sc.changed_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn row_to_export_changed(r: sqlx::postgres::PgRow) -> ExportChangedCharger {
    ExportChangedCharger {
        id: r.get("id"),
        title: r.get("title"),
        city: r.get("city"),
        region: r.get("region"),
        latitude: r.get("latitude"),
        longitude: r.get("longitude"),
        status: r.get("status"),
        raw_status_value: r.get("raw_status_value"),
        charger_category: r.get("charger_category"),
        first_seen_at: r.get("first_seen_at"),
        last_scraped_at: r.get("last_scraped_at"),
    }
}

fn row_to_export_opened(r: sqlx::postgres::PgRow) -> ExportOpenedCharger {
    ExportOpenedCharger {
        id: r.get("id"),
        title: r.get("title"),
        city: r.get("city"),
        region: r.get("region"),
        latitude: r.get("latitude"),
        longitude: r.get("longitude"),
        opening_date: r.get("opening_date"),
        num_stalls: r.get("num_stalls"),
        open_to_non_tesla: r.get("open_to_non_tesla"),
    }
}

