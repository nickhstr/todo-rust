//! Per-request middleware: read Accept-Language and `locale`/`tz` cookies,
//! then stash `RequestLocale` and `RequestTz` on request extensions for
//! handlers and templates to read. Authenticated handlers may re-resolve
//! locale from `users.locale`/`users.timezone` and overwrite these
//! extensions before rendering.

use axum::{
    body::Body,
    extract::Request,
    http::{header, HeaderMap},
    middleware::Next,
    response::Response,
};
use time_tz::TimeZone as _;
use todo_i18n::{negotiate, parse_tz, Tz, UTC};
use unic_langid::LanguageIdentifier;

#[derive(Clone, Debug)]
pub struct RequestLocale(pub LanguageIdentifier);

#[derive(Clone, Debug)]
pub struct RequestTz(pub Tz);

pub async fn i18n_middleware(mut req: Request<Body>, next: Next) -> Response {
    let (locale_str, tz_str) = read_cookies(req.headers());
    let locale = negotiate(
        req.headers()
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok()),
        locale_str.as_deref(),
        None,
    );
    let tz = tz_str.as_deref().and_then(parse_tz).unwrap_or_else(|| {
        if tz_str.is_some() {
            metrics::counter!("i18n_invalid_tz_total").increment(1);
        }
        UTC
    });
    metrics::counter!("i18n_locale_total", "locale" => locale.to_string()).increment(1);
    tracing::Span::current()
        .record("request.locale", locale.to_string().as_str())
        .record("request.tz", tz.name());
    req.extensions_mut().insert(RequestLocale(locale));
    req.extensions_mut().insert(RequestTz(tz));
    next.run(req).await
}

fn read_cookies(headers: &HeaderMap) -> (Option<String>, Option<String>) {
    let mut locale = None;
    let mut tz = None;
    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for chunk in cookie_hdr.split(';') {
            let chunk = chunk.trim();
            if let Some(rest) = chunk.strip_prefix("locale=") {
                locale = Some(rest.to_owned());
            } else if let Some(rest) = chunk.strip_prefix("tz=") {
                tz = Some(rest.to_owned());
            }
        }
    }
    (locale, tz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn read_cookies_extracts_both() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("tz=America/Los_Angeles; locale=es"),
        );
        let (locale, tz) = read_cookies(&h);
        assert_eq!(locale.as_deref(), Some("es"));
        assert_eq!(tz.as_deref(), Some("America/Los_Angeles"));
    }

    #[test]
    fn read_cookies_handles_missing() {
        let h = HeaderMap::new();
        let (locale, tz) = read_cookies(&h);
        assert_eq!(locale, None);
        assert_eq!(tz, None);
    }
}
