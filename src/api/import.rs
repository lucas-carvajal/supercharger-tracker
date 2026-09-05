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
    Applied {
        run_id: i64,
        changed: usize,
        opened: usize,
        removed: usize,
    },
    Duplicate {
        run_id: i64,
    },
    OutOfOrder {
        expected: i64,
        got: i64,
    },
    SnapshotApplied {
        source_run_id: i64,
        scrape_runs: usize,
        chargers: usize,
        opened: usize,
    },
}

#[derive(Serialize)]
pub struct CurrentVersionResponse {
    pub current_version: i64,
    pub next_expected_version: i64,
}

pub async fn current_version_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = super::require_internal_secret(&state, &headers) {
        return response;
    }

    match state.scrape_run.get_max_run_id().await {
        Ok(max_run_id) => {
            let current_version = max_run_id.unwrap_or(0);
            Json(CurrentVersionResponse {
                current_version,
                next_expected_version: current_version + 1,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "current version lookup error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal server error".into(),
                }),
            )
                .into_response()
        }
    }
}

pub async fn import_handler(
    State(state): State<AppState>,
    Query(query): Query<ImportQuery>,
    headers: HeaderMap,
    Json(export): Json<ScrapeExport>,
) -> Response {
    if let Some(response) = super::require_internal_secret(&state, &headers) {
        return response;
    }

    match apply_import(&state.supercharger, &state.scrape_run, export, query.force).await {
        Ok(ImportOutcome::Applied {
            run_id,
            changed,
            opened,
            removed,
        }) => (
            StatusCode::OK,
            Json(ImportResponse::Applied {
                run_id,
                changed,
                opened,
                removed,
            }),
        )
            .into_response(),
        Ok(ImportOutcome::Duplicate { run_id }) => {
            (StatusCode::OK, Json(ImportResponse::Duplicate { run_id })).into_response()
        }
        Ok(ImportOutcome::OutOfOrder { expected, got }) => (
            StatusCode::CONFLICT,
            Json(ImportResponse::OutOfOrder { expected, got }),
        )
            .into_response(),
        Ok(ImportOutcome::SnapshotApplied {
            source_run_id,
            scrape_runs,
            chargers,
            opened,
        }) => (
            StatusCode::OK,
            Json(ImportResponse::SnapshotApplied {
                source_run_id,
                scrape_runs,
                chargers,
                opened,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "import error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal server error".into(),
                }),
            )
                .into_response()
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[cfg(test)]
mod tests {
    use axum::{
        Json,
        extract::{Query, State},
        http::{HeaderMap, HeaderValue, StatusCode},
        response::IntoResponse,
    };
    use sqlx::PgPool;

    use crate::{
        api::AppState,
        export::{DiffExport, ScrapeExport},
        repository::{ScrapeRunRepository, SuperchargerRepository},
    };

    use super::current_version_handler;
    use super::{ImportQuery, import_handler};
    use crate::api::has_valid_internal_secret;

    fn test_state(secret: Option<&str>) -> AppState {
        let pool = PgPool::connect_lazy("postgres://postgres:pass@localhost/test")
            .expect("lazy pool should parse");
        AppState {
            supercharger: SuperchargerRepository::new(pool.clone()),
            scrape_run: ScrapeRunRepository::new(pool.clone()),
            internal_import_secret: secret.map(str::to_owned),
            pool,
        }
    }

    fn test_export() -> ScrapeExport {
        ScrapeExport::Diff(DiffExport {
            run_id: 42,
            scraped_at: chrono::Utc::now(),
            country: "US".into(),
            changed_chargers: vec![],
            status_changes: vec![],
            opened_chargers: vec![],
            removed_ids: vec![],
        })
    }

    #[tokio::test]
    async fn import_requires_internal_secret_to_be_configured() {
        let response = import_handler(
            State(test_state(None)),
            Query(ImportQuery { force: false }),
            HeaderMap::new(),
            Json(test_export()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn import_rejects_missing_or_invalid_internal_secret() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Admin-Internal-Secret",
            HeaderValue::from_static("wrong-secret"),
        );

        let response = import_handler(
            State(test_state(Some("correct-secret"))),
            Query(ImportQuery { force: false }),
            headers,
            Json(test_export()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn current_version_requires_internal_secret_to_be_configured() {
        let response = current_version_handler(State(test_state(None)), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn current_version_rejects_missing_or_invalid_internal_secret() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Admin-Internal-Secret",
            HeaderValue::from_static("wrong-secret"),
        );

        let response = current_version_handler(State(test_state(Some("correct-secret"))), headers)
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn accepts_valid_internal_secret_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Admin-Internal-Secret",
            HeaderValue::from_static("correct-secret"),
        );

        assert!(has_valid_internal_secret(&headers, "correct-secret"));
    }
}
