use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

// ── Types ─────────────────────────────────────────────────────────────────────

/// Summary of a single scrape run, including how many status changes it produced.
pub struct RunStats {
    pub id: i64,
    pub run_type: String,
    pub scraped_at: DateTime<Utc>,
    pub total_count: i32,
    pub details_failures: i32,
    pub status_changes_count: i64,
}

pub struct ApiScrapeRun {
    pub id: i64,
    pub country: String,
    pub scraped_at: DateTime<Utc>,
    pub total_count: i32,
}

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
}
