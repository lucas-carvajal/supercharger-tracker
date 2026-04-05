use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use sqlx::PgPool;
use tower_http::cors::CorsLayer;

use crate::repository::{ScrapeRunRepository, SuperchargerRepository};

pub mod import;
pub mod regions;
pub mod scrape_runs;
pub mod superchargers;

#[derive(Clone)]
pub struct AppState {
    pub supercharger: SuperchargerRepository,
    pub scrape_run: ScrapeRunRepository,
}

pub fn router(pool: PgPool) -> Router {
    let state = AppState {
        supercharger: SuperchargerRepository::new(pool.clone()),
        scrape_run: ScrapeRunRepository::new(pool),
    };
    Router::new()
        .route("/superchargers/soon/stats", get(superchargers::stats_handler))
        .route(
            "/superchargers/soon/recent-changes",
            get(superchargers::recent_changes_handler),
        )
        .route(
            "/superchargers/soon/recent-additions",
            get(superchargers::recent_additions_handler),
        )
        .route("/superchargers/soon/{id}", get(superchargers::detail_handler))
        .route("/superchargers/soon", get(superchargers::list_handler))
        .route("/scrape-runs", get(scrape_runs::scrape_runs_handler))
        .route("/scrapes/import", post(import::import_handler))
        .with_state(state)
        .layer(CorsLayer::permissive())
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}
