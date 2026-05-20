//! Centralized template context construction. Every handler that
//! renders an HTML response calls `base_context` and merges it with
//! handler-specific values, so `_locale`, `_tz`, and `csp_nonce` are
//! always present.

use minijinja::{context, value::Value};
use time_tz::TimeZone as _;

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
