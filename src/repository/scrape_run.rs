use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

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
        sqlx::query_scalar("SELECT id FROM scrape_runs ORDER BY id DESC LIMIT 1")
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

    /// Returns the maximum `id` in scrape_runs, or `None` if the table is empty.
    /// Used on prod for ordering checks: next import must have run_id == MAX(id) + 1.
    pub async fn get_max_run_id(&self) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar("SELECT MAX(id) FROM scrape_runs")
            .fetch_optional(&self.pool)
            .await
            .map(|opt: Option<Option<i64>>| opt.flatten())
    }

    /// Returns true if a run with the given id already exists (dedup check).
    pub async fn run_id_exists(&self, id: i64) -> Result<bool, sqlx::Error> {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM scrape_runs WHERE id = $1 LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(exists.is_some())
    }

    /// Returns full record for the most recent run, used by `export-diff` to build
    /// the export header.
    pub async fn get_latest_run(&self) -> Result<Option<LatestRun>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, country, scraped_at, details_failures, open_status_failures \
             FROM scrape_runs ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| LatestRun {
            id: r.get("id"),
            country: r.get("country"),
            scraped_at: r.get("scraped_at"),
            details_failures: r.get("details_failures"),
            open_status_failures: r.get("open_status_failures"),
        }))
    }

    /// Returns stats for the most recent scrape run, or `None` if no runs exist yet.
    pub async fn get_last_run_stats(&self) -> Result<Option<RunStats>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT r.id, r.run_type, r.scraped_at, r.total_count, r.details_failures, \
                    (SELECT COUNT(*) FROM status_changes sc WHERE sc.scrape_run_id = r.id) \
                        AS status_changes_count \
             FROM scrape_runs r \
             ORDER BY r.id DESC \
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
        sqlx::query_scalar("SELECT scraped_at FROM scrape_runs ORDER BY id DESC LIMIT 1")
            .fetch_optional(&self.pool)
            .await
    }

    /// Returns recent scrape runs ordered by `scraped_at` DESC.
    pub async fn list_scrape_runs(&self, limit: i64) -> Result<Vec<ApiScrapeRun>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, country, scraped_at, COALESCE(total_count, 0) AS total_count \
             FROM scrape_runs \
             ORDER BY id DESC \
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

}

pub struct LatestRun {
    pub id: i64,
    pub country: String,
    pub scraped_at: DateTime<Utc>,
    pub details_failures: i32,
    pub open_status_failures: i32,
}
