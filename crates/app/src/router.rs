//! Router assembly. Lives in `lib.rs` so integration tests can rebuild the
//! exact same router that `main.rs` serves.

use std::time::Duration;

use axum::{
    http::{header, HeaderName},
    middleware as ax_middleware,
    routing::{delete, get, post},
    Router,
};
use metrics_exporter_prometheus::PrometheusHandle;
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer,
    normalize_path::NormalizePathLayer, sensitive_headers::SetSensitiveRequestHeadersLayer,
    services::ServeDir, timeout::TimeoutLayer, trace::TraceLayer,
};
use tower_sessions_sqlx_store::PostgresStore;

use crate::{
    auth::AuthBackend,
    middleware::{
        apply_security_headers, http_metrics_layer, make_request_id_layers, rate_limit_middleware,
        RateLimiter,
    },
    routes::{auth as auth_routes, health as health_routes, pages, todos as todo_routes},
    AppState,
};

/// Build the full app router. Same shape main.rs uses; integration tests call
/// this against a testcontainer-provided database.
pub fn build_router(
    state: AppState,
    prom: PrometheusHandle,
    auth_layer: axum_login::AuthManagerLayer<
        AuthBackend,
        PostgresStore,
        tower_sessions::service::SignedCookie,
    >,
) -> Router {
    let (set_id, propagate_id) = make_request_id_layers();
    let cookie_secure = state.config.auth.cookie_secure;
    let static_dir = state.config.static_dir.clone();

    // 5 logins per minute per IP — generous burst, slow refill.
    let login_limiter = RateLimiter::new(5.0 / 60.0, 5.0)
        .trust_forwarded_for(state.config.server.trust_forwarded_for);

    let auth_endpoints = Router::new()
        .route(
            "/login",
            get(auth_routes::login_form).post(auth_routes::login),
        )
        .route(
            "/signup",
            get(auth_routes::signup_form).post(auth_routes::signup),
        )
        .layer(ax_middleware::from_fn_with_state(
            login_limiter,
            rate_limit_middleware,
        ));

    let api = Router::new()
        .route("/", get(pages::index))
        .route("/logout", post(auth_routes::logout))
        .route("/todos", get(todo_routes::list).post(todo_routes::create))
        .route("/todos/:id/toggle", post(todo_routes::toggle))
        .route("/todos/:id", delete(todo_routes::delete))
        .merge(auth_endpoints)
        .layer(auth_layer);

    let health = health_routes::router(prom);

    let serve_static = ServeDir::new(static_dir)
        .precompressed_gzip()
        .precompressed_br();

    let merged = Router::new()
        .merge(api)
        .merge(health)
        .nest_service("/static", serve_static)
        .with_state(state)
        .layer(ax_middleware::from_fn(http_metrics_layer))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(NormalizePathLayer::trim_trailing_slash())
        .layer(SetSensitiveRequestHeadersLayer::new([
            HeaderName::from_static("authorization"),
            header::COOKIE,
        ]))
        .layer(CatchPanicLayer::new())
        .layer(set_id)
        .layer(propagate_id);

    apply_security_headers(merged, cookie_secure)
}
