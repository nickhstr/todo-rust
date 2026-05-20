//! Centralized template context construction and the shared
//! validation-error localizer.

use minijinja::{context, value::Value};
use time_tz::TimeZone as _;
use todo_i18n::Locales;
use unic_langid::LanguageIdentifier;
use validator::ValidationErrors;

use crate::middleware::{CspNonce, RequestLocale, RequestTz};

/// Build the standard "ambient" context. Templates always see
/// `_locale`, `_tz`, and `csp_nonce`. Handlers extend it with their
/// own values via `context! { ..base_context(...), foo, bar }`.
pub fn base_context(locale: &RequestLocale, tz: &RequestTz, csp_nonce: &CspNonce) -> Value {
    context! {
        _locale => locale.0.to_string(),
        _tz => tz.0.name(),
        csp_nonce => csp_nonce.0.clone(),
    }
}

/// Walk `ValidationErrors`, treat each `message` as a Fluent id, and
/// resolve through `Locales`. Empty input falls back to
/// `validation-generic`. Shared by `routes::auth` and `routes::todos`
/// so todos' validation failures get the same localization treatment
/// auth's do.
pub fn localize_validation_errors(
    locales: &Locales,
    locale: &LanguageIdentifier,
    errs: &ValidationErrors,
) -> String {
    let mut parts = Vec::new();
    for (_field, kind) in errs.field_errors() {
        for e in kind {
            let id = e
                .message
                .as_ref()
                .map(std::string::ToString::to_string)
                .unwrap_or_else(|| e.code.to_string());
            parts.push(locales.lookup(locale, &id, None));
        }
    }
    if parts.is_empty() {
        locales.lookup(locale, "validation-generic", None)
    } else {
        parts.join(" ")
    }
}
