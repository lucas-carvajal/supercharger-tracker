use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::application::import::{ImportOutcome, apply_import};
use crate::export::ScrapeExport;

use super::AppState;

#[derive(Deserialize)]
pub struct ImportQuery {
    #[serde(default)]
    pub force: bool,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ImportResponse {
    Applied { run_id: i64, changed: usize, opened: usize, removed: usize },
    Duplicate { run_id: i64 },
    OutOfOrder { expected: i64, got: i64 },
    SnapshotApplied { source_run_id: i64, scrape_runs: usize, chargers: usize, opened: usize },
}

pub async fn import_handler(
    State(state): State<AppState>,
    Query(query): Query<ImportQuery>,
    headers: HeaderMap,
    Json(export): Json<ScrapeExport>,
) -> Response {
    // Auth
    let Some(ref expected_token) = state.import_token else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorBody { error: "IMPORT_TOKEN not configured on server".into() }),
        ).into_response();
    };
    let provided = headers.get("X-Import-Token").and_then(|v| v.to_str().ok()).unwrap_or("");
    if provided != expected_token {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody { error: "invalid or missing X-Import-Token".into() }),
        ).into_response();
    }

    match apply_import(&state.supercharger, &state.scrape_run, export, query.force).await {
        Ok(ImportOutcome::Applied { run_id, changed, opened, removed }) => (
            StatusCode::OK,
            Json(ImportResponse::Applied { run_id, changed, opened, removed }),
        ).into_response(),
        Ok(ImportOutcome::Duplicate { run_id }) => (
            StatusCode::OK,
            Json(ImportResponse::Duplicate { run_id }),
        ).into_response(),
        Ok(ImportOutcome::OutOfOrder { expected, got }) => (
            StatusCode::CONFLICT,
            Json(ImportResponse::OutOfOrder { expected, got }),
        ).into_response(),
        Ok(ImportOutcome::SnapshotApplied { source_run_id, scrape_runs, chargers, opened }) => (
            StatusCode::OK,
            Json(ImportResponse::SnapshotApplied { source_run_id, scrape_runs, chargers, opened }),
        ).into_response(),
        Err(e) => {
            eprintln!("import error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody { error: "internal server error".into() }),
            ).into_response()
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}
