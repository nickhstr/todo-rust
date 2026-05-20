use axum::{
    body::Body,
    extract::Request,
    http::{header, HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
    Router,
};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
use rand::RngCore;
use tower_http::set_header::SetResponseHeaderLayer;

/// 128-bit base64 nonce, attached to the request and the response CSP
/// for the lifetime of one request. Templates pull it from
/// `Extension<CspNonce>` and emit `nonce="{{ csp_nonce }}"` on inline
/// `<script>` tags.
#[derive(Clone, Debug)]
pub struct CspNonce(pub String);

const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");

pub async fn csp_nonce_middleware(mut req: Request<Body>, next: Next) -> Response {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let nonce = STANDARD_NO_PAD.encode(bytes);

    req.extensions_mut().insert(CspNonce(nonce.clone()));

    let mut response = next.run(req).await;

    let csp = format!(
        "default-src 'self'; \
         script-src 'self' 'unsafe-eval' 'nonce-{nonce}'; \
         style-src 'self' 'unsafe-inline'; \
         font-src 'self'; \
         img-src 'self' data:; \
         connect-src 'self'"
    );
    if let Ok(value) = HeaderValue::from_str(&csp) {
        response
            .headers_mut()
            .insert(header::CONTENT_SECURITY_POLICY, value);
    }
    response
}

/// Apply the static security headers (everything except CSP) plus HSTS
/// when behind TLS. CSP is set per-request by `csp_nonce_middleware`
/// (added by the router) and includes a fresh nonce.
pub fn apply_security_headers<S>(router: Router<S>, cookie_secure: bool) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let router = router
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Extension, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn each_request_gets_a_unique_nonce() {
        async fn handler(Extension(nonce): Extension<CspNonce>) -> String {
            nonce.0
        }
        let app = Router::new()
            .route("/", get(handler))
            .layer(axum::middleware::from_fn(csp_nonce_middleware));

        let r1 = app
            .clone()
            .oneshot(axum::http::Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let r2 = app
            .clone()
            .oneshot(axum::http::Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();

        let csp1 = r1.headers().get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap().to_owned();
        let csp2 = r2.headers().get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap().to_owned();

        assert!(csp1.contains("nonce-"));
        assert!(csp2.contains("nonce-"));
        assert_ne!(csp1, csp2);
    }
}
