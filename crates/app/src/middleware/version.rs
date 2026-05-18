//! Stamp every response with `X-App-Version: <git sha>`. The SHA is captured
//! at build time by `build.rs`; the Docker build threads it in via `--build-arg
//! GIT_SHA=...` since `.git/` is excluded from the build context.

use axum::{
    http::{HeaderName, HeaderValue},
    Router,
};
use tower_http::set_header::SetResponseHeaderLayer;

const X_APP_VERSION: HeaderName = HeaderName::from_static("x-app-version");

/// Git SHA the binary was built from; `"unknown"` if neither `$GIT_SHA` nor
/// `git rev-parse HEAD` was available at build time.
pub const GIT_SHA: &str = env!("GIT_SHA");

pub fn apply_version_header<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.layer(SetResponseHeaderLayer::if_not_present(
        X_APP_VERSION,
        HeaderValue::from_static(GIT_SHA),
    ))
}
