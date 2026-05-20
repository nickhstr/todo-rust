//! Router assembly. Lives in `lib.rs` so integration tests can rebuild the
//! exact same router that `main.rs` serves.

use std::time::Duration;

use axum::{
    http::{header, HeaderName, HeaderValue},
    middleware as ax_middleware,
    response::IntoResponse,
    routing::{delete, get, post},
    Router,
};
use metrics_exporter_prometheus::PrometheusHandle;
use tower_http::{
    catch_panic::CatchPanicLayer, compression::CompressionLayer,
    normalize_path::NormalizePathLayer, sensitive_headers::SetSensitiveRequestHeadersLayer,
    services::ServeDir, set_header::SetResponseHeaderLayer, timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tower_sessions_sqlx_store::PostgresStore;

use crate::{
    auth::AuthBackend,
    middleware::{
        apply_security_headers, apply_version_header, http_metrics_layer, make_request_id_layers,
        rate_limit_middleware, RateLimiter,
    },
    routes::{
        auth as auth_routes, health as health_routes, pages, preferences as pref_routes,
        todos as todo_routes,
    },
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
        .trust_forwarded_for(state.config.server.trust_forwarded_for)
        .enabled(state.config.rate_limit.enabled);

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
        .route("/preferences/locale", post(pref_routes::update_locale))
        .merge(auth_endpoints);

    // Dev-only passwordless login. The route is compiled out of `--release`
    // builds entirely; the handler also re-checks `enabled_email()` so a debug
    // build with the config unset still returns 404.
    #[cfg(debug_assertions)]
    let api = api.route("/dev/login", post(crate::routes::dev::auto_login));

    let api = api.layer(auth_layer);

    let health = health_routes::router(prom);

    // Static service: try the hashed-asset manifest first, fall through to
    // ServeDir for unhashed paths. The fallthrough ServeDir is wrapped with
    // a short Cache-Control override (overriding the default private,no-cache
    // from the router-level layer) so unhashed assets are still cacheable
    // for ~5 minutes even though they're not content-addressed.
    use tower::Service as _;
    let assets_for_static = state.assets.clone();
    let serve_static_inner = ServeDir::new(static_dir.clone())
        .precompressed_gzip()
        .precompressed_br();
    let cached_static = tower::ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=300"),
        ))
        .service(serve_static_inner);
    let static_service = tower::service_fn(move |req: axum::extract::Request| {
        let assets = assets_for_static.clone();
        let mut cached_static = cached_static.clone();
        async move {
            let path = req.uri().path().trim_start_matches('/').to_owned();
            // url_path will be like "css/app.<hash>.css" (the leading "/static/"
            // was already stripped by nest_service before delegating to us)
            if let Some(on_disk) = assets.resolve_hashed_request(&path) {
                match crate::routes::assets::serve_immutable_file(&on_disk).await {
                    Ok(r) => Ok::<_, std::convert::Infallible>(r),
                    Err(_) => Ok(crate::routes::assets::not_found()),
                }
            } else {
                // Pass through to ServeDir. ServeDir is always ready so we
                // call it directly — skipping `.ready()` avoids a type
                // ambiguity the compiler can't resolve without an explicit
                // turbofish for the request-body type.
                match cached_static.call(req).await {
                    Ok(r) => Ok::<_, std::convert::Infallible>(r.into_response()),
                    Err(e) => match e {},
                }
            }
        }
    });

    let merged = Router::new()
        .merge(api)
        .merge(health)
        .nest_service("/static", static_service)
        .with_state(state)
        .layer(ax_middleware::from_fn(http_metrics_layer))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-cache"),
        ))
        .layer(ax_middleware::from_fn(crate::middleware::i18n_middleware))
        .layer(ax_middleware::from_fn(
            crate::middleware::csp_nonce_middleware,
        ))
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

    apply_version_header(apply_security_headers(merged, cookie_secure))
}
