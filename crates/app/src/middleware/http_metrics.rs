//! Records `http_requests_total` + `http_request_duration_seconds` per response.
//! TraceLayer emits spans + logs but no Prometheus counters; this fills that gap.

use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};

pub async fn http_metrics_layer(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().clone();
    // Use the matched route pattern (e.g. `/todos/:id`) to keep cardinality low.
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| req.uri().path().to_owned());

    // `nest_service("/static", ...)` doesn't set MatchedPath, so raw URIs would
    // explode cardinality (one label per filename + 404 probe). Collapse the
    // whole tree to a single label.
    let label_path = if path.starts_with("/static") {
        "/static/*".to_owned()
    } else {
        path
    };

    let response = next.run(req).await;
    let status = response.status().as_u16().to_string();
    let elapsed = start.elapsed().as_secs_f64();

    let labels = [
        ("method", method.as_str().to_owned()),
        ("path", label_path),
        ("status", status),
    ];
    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_request_duration_seconds", &labels).record(elapsed);

    response
}
