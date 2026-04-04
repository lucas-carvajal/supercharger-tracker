use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, AppState};

#[derive(Deserialize)]
pub struct ScrapeRunsQuery {
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct ScrapeRunItem {
    pub id: i64,
    pub country: String,
    pub scraped_at: DateTime<Utc>,
    pub total_count: i32,
}

#[derive(Serialize)]
pub struct ScrapeRunsResponse {
    pub items: Vec<ScrapeRunItem>,
}

/// GET /scrape-runs
pub async fn scrape_runs_handler(
    State(state): State<AppState>,
    Query(params): Query<ScrapeRunsQuery>,
) -> Result<Json<ScrapeRunsResponse>, ApiError> {
    let limit = params.limit.unwrap_or(10).clamp(1, 50);

    let rows = state.scrape_run.list_scrape_runs(limit).await?;

    let items = rows
        .into_iter()
        .map(|r| ScrapeRunItem {
            id: r.id,
            country: r.country,
            scraped_at: r.scraped_at,
            total_count: r.total_count,
        })
        .collect();

    Ok(Json(ScrapeRunsResponse { items }))
}
