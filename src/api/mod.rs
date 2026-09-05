use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use sqlx::PgPool;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::repository::{ScrapeRunRepository, SuperchargerRepository};
use crate::util::config::Config;

pub mod backfill;
pub mod import;
pub mod scrape_runs;
pub mod superchargers;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub supercharger: SuperchargerRepository,
    pub scrape_run: ScrapeRunRepository,
    /// `None` means `POST /admin/import/scrapes` is disabled (returns 503).
    pub internal_import_secret: Option<String>,
}

pub fn router(pool: PgPool, config: Config) -> Router {
    let state = AppState {
        supercharger: SuperchargerRepository::new(pool.clone()),
        scrape_run: ScrapeRunRepository::new(pool.clone()),
        internal_import_secret: config.internal_import_secret,
        pool,
    };
    Router::new()
        .route(
            "/superchargers/soon/stats",
            get(superchargers::stats_handler),
        )
        .route(
            "/superchargers/soon/recent-changes",
            get(superchargers::recent_changes_handler),
        )
        .route(
            "/superchargers/soon/recent-additions",
            get(superchargers::recent_additions_handler),
        )
        .route(
            "/superchargers/soon/recent-updates",
            get(superchargers::recent_updates_handler),
        )
        .route("/superchargers/soon/map", get(superchargers::map_handler))
        .route(
            "/superchargers/soon/{id}",
            get(superchargers::detail_handler),
        )
        .route("/superchargers/soon", get(superchargers::list_handler))
        .route("/scrape-runs", get(scrape_runs::scrape_runs_handler))
        .route(
            "/admin/import/current-version",
            get(import::current_version_handler),
        )
        .route("/admin/import/scrapes", post(import::import_handler))
        // TODO: remove POST /admin/backfill/country after existing rows are filled.
        .route(
            "/admin/backfill/country",
            post(backfill::backfill_country_handler),
        )
        .route("/health", get(health_handler))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
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

async fn health_handler(State(state): State<AppState>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "health check: database unreachable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "status": "error", "message": "database unreachable" })),
            )
                .into_response()
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(error = %e, "database error");
        ApiError::Internal("internal server error".into())
    }
}

pub(crate) fn require_internal_secret(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Option<Response> {
    let Some(ref expected_secret) = state.internal_import_secret else {
        return Some(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorBody {
                    error: "RUST_INTERNAL_IMPORT_SECRET not configured on server".into(),
                }),
            )
                .into_response(),
        );
    };
    if !has_valid_internal_secret(headers, expected_secret) {
        return Some(
            (
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "invalid or missing X-Admin-Internal-Secret".into(),
                }),
            )
                .into_response(),
        );
    }

    None
}

pub(crate) fn has_valid_internal_secret(
    headers: &axum::http::HeaderMap,
    expected_secret: &str,
) -> bool {
    headers
        .get("X-Admin-Internal-Secret")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|provided| provided == expected_secret)
}
