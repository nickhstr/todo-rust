use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Router};
use metrics_exporter_prometheus::PrometheusHandle;
use todo_storage::pool;

use crate::AppState;

pub fn router(prom: PrometheusHandle) -> Router<AppState> {
    Router::new()
        .route("/healthz", get(liveness))
        .route("/readyz", get(readiness))
        // /metrics is held in a closure that owns the handle; cheap to render.
        .route(
            "/metrics",
            get(move || {
                let prom = prom.clone();
                async move {
                    (
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "text/plain; version=0.0.4",
                        )],
                        prom.render(),
                    )
                }
            }),
        )
}

async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    match pool::ping(&state.db).await {
        Ok(()) => (StatusCode::OK, "ready"),
        Err(err) => {
            tracing::warn!(error = %err, "readiness failed");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready")
        }
    }
}
