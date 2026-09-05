use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Serialize;

use super::AppState;

#[derive(Serialize)]
pub struct BackfillCountryResponse {
    pub coming_soon_updated: i64,
    pub opened_updated: i64,
    pub failed: i64,
}

/// TODO: remove this endpoint after existing rows are filled.
pub async fn backfill_country_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = super::require_internal_secret(&state, &headers) {
        return response;
    }

    match state.supercharger.backfill_country().await {
        Ok(result) => Json(BackfillCountryResponse {
            coming_soon_updated: result.coming_soon_updated,
            opened_updated: result.opened_updated,
            failed: result.failed,
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "country backfill error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "internal server error" })),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        extract::State,
        http::{HeaderMap, HeaderValue, StatusCode},
        response::IntoResponse,
    };
    use sqlx::PgPool;

    use crate::{
        api::AppState,
        repository::{ScrapeRunRepository, SuperchargerRepository},
    };

    use super::backfill_country_handler;

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

    #[tokio::test]
    async fn backfill_requires_internal_secret_to_be_configured() {
        let response = backfill_country_handler(State(test_state(None)), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn backfill_rejects_missing_or_invalid_internal_secret() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Admin-Internal-Secret",
            HeaderValue::from_static("wrong-secret"),
        );

        let response = backfill_country_handler(State(test_state(Some("correct-secret"))), headers)
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
