use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::export::ExportScrapeRun;

use super::models::{ApiScrapeRun, RunStats};

// ── Repository ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ScrapeRunRepository {
    pool: PgPool,
}

impl ScrapeRunRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn record_run(
        &self,
        country: &str,
        total_count: i32,
        details_failures: i32,
        open_status_failures: i32,
        run_type: &str,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO scrape_runs (country, total_count, details_failures, open_status_failures, run_type) \
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(country)
        .bind(total_count)
        .bind(details_failures)
        .bind(open_status_failures)
        .bind(run_type)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    /// Returns the id of the most recent scrape run, or `None` if none exist.
    pub async fn get_last_run_id(&self) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar("SELECT id FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1")
            .fetch_optional(&self.pool)
            .await
    }

    /// Bump the retry counters on an existing scrape run.
    /// Sets `details_failures` and `open_status_failures` to the latest counts.
    pub async fn update_retry(
        &self,
        run_id: i64,
        details_failures: i32,
        open_status_failures: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE scrape_runs \
             SET retry_count = retry_count + 1, last_retry_at = NOW(), \
                 details_failures = $2, open_status_failures = $3 \
             WHERE id = $1",
        )
        .bind(run_id)
        .bind(details_failures)
        .bind(open_status_failures)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert a scrape_runs row representing an imported diff or snapshot-seed on prod.
    /// Stores `source_run_id` for dedup and ordering.
    pub async fn record_import_run(
        &self,
        country: &str,
        scraped_at: DateTime<Utc>,
        source_run_id: i64,
        run_type: &str,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO scrape_runs (country, scraped_at, total_count, details_failures, \
                                      open_status_failures, run_type, source_run_id) \
             VALUES ($1, $2, 0, 0, 0, $3, $4) RETURNING id",
        )
        .bind(country)
        .bind(scraped_at)
        .bind(run_type)
        .bind(source_run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    /// Mark a run as exported.
    pub async fn mark_exported(&self, run_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE scrape_runs SET exported = TRUE WHERE id = $1")
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Returns the id of the most recently exported scrape run, or `None` if no
    /// runs have been exported yet.
    pub async fn get_last_exported_run_id(&self) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT id FROM scrape_runs WHERE exported = TRUE ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
    }

    /// Returns the maximum `source_run_id` recorded on prod, or `None` if nothing
    /// has been imported yet.
    pub async fn get_max_source_run_id(&self) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT MAX(source_run_id) FROM scrape_runs WHERE source_run_id IS NOT NULL",
        )
        .fetch_optional(&self.pool)
        .await
        .map(|opt: Option<Option<i64>>| opt.flatten())
    }

    /// Returns true if a run with the given `source_run_id` already exists.
    pub async fn source_run_id_exists(&self, source_run_id: i64) -> Result<bool, sqlx::Error> {
        let row: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM scrape_runs WHERE source_run_id = $1 LIMIT 1",
        )
        .bind(source_run_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Returns full record for the most recent run, used by `export-diff` to build
    /// the export header.
    pub async fn get_latest_run(&self) -> Result<Option<LatestRun>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, country, scraped_at, details_failures, open_status_failures, exported \
             FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| LatestRun {
            id: r.get("id"),
            country: r.get("country"),
            scraped_at: r.get("scraped_at"),
            details_failures: r.get("details_failures"),
            open_status_failures: r.get("open_status_failures"),
            exported: r.get("exported"),
        }))
    }

    /// Returns stats for the most recent scrape run, or `None` if no runs exist yet.
    pub async fn get_last_run_stats(&self) -> Result<Option<RunStats>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT r.id, r.run_type, r.scraped_at, r.total_count, r.details_failures, \
                    (SELECT COUNT(*) FROM status_changes sc WHERE sc.scrape_run_id = r.id) \
                        AS status_changes_count \
             FROM scrape_runs r \
             ORDER BY r.scraped_at DESC \
             LIMIT 1",
        )
        .fetch_optional(&self.pool)
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

    /// Returns the most recent scrape run's `scraped_at`, or `None` if no runs exist.
    pub async fn latest_scrape_run_time(&self) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
        sqlx::query_scalar("SELECT scraped_at FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1")
            .fetch_optional(&self.pool)
            .await
    }

    /// Returns recent scrape runs ordered by `scraped_at` DESC.
    pub async fn list_scrape_runs(&self, limit: i64) -> Result<Vec<ApiScrapeRun>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, country, scraped_at, COALESCE(total_count, 0) AS total_count \
             FROM scrape_runs \
             ORDER BY scraped_at DESC \
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
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

    // ── Snapshot I/O ──────────────────────────────────────────────────────────

    /// Returns all scrape_runs for snapshot export.
    pub async fn get_all_scrape_runs(&self) -> Result<Vec<ExportScrapeRun>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, country, scraped_at, total_count, details_failures, \
                    open_status_failures, retry_count, last_retry_at, run_type \
             FROM scrape_runs ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ExportScrapeRun {
                id: r.get("id"),
                country: r.get("country"),
                scraped_at: r.get("scraped_at"),
                total_count: r.get("total_count"),
                details_failures: r.get("details_failures"),
                open_status_failures: r.get("open_status_failures"),
                retry_count: r.get("retry_count"),
                last_retry_at: r.get("last_retry_at"),
                run_type: r.get("run_type"),
            })
            .collect())
    }
}

pub struct LatestRun {
    pub id: i64,
    pub country: String,
    pub scraped_at: DateTime<Utc>,
    pub details_failures: i32,
    pub open_status_failures: i32,
    pub exported: bool,
}
