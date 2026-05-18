use axum::{
    http::{header, HeaderName, HeaderValue},
    Router,
};
use tower_http::set_header::SetResponseHeaderLayer;

// Scripts ship vendored under /static/vendor/. `'unsafe-eval'` is required
// because Alpine.js compiles each directive expression (`x-data`, `@click`,
// `x-show`, ...) at runtime, and htmx 4's `hx-on::*` attributes do the same.
// We deliberately do NOT add `'unsafe-inline'` to script-src, so an injected
// inline `<script>` is still blocked.
// `'unsafe-inline'` on style-src covers `style="..."` attributes in templates.
const CSP: &str = "default-src 'self'; \
    script-src 'self' 'unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; \
    font-src 'self'; \
    img-src 'self' data:; \
    connect-src 'self'";

const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");

/// Apply baseline hardening response headers. HSTS only when `cookie_secure`
/// (i.e. when we're behind TLS); shipping it over plain HTTP would lock users
/// out for the max-age duration.
pub fn apply_security_headers<S>(router: Router<S>, cookie_secure: bool) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let router = router
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            PERMISSIONS_POLICY,
            HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
        ));

    if cookie_secure {
        router.layer(SetResponseHeaderLayer::if_not_present(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
    } else {
        router
    }
}
