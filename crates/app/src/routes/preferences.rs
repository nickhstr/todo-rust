//! User preference updates: locale (and later, timezone).

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Form,
};
use serde::Deserialize;
use todo_i18n::SUPPORTED;

use crate::{auth::AuthSession, AppError, AppState};

#[derive(Deserialize)]
pub struct UpdateLocale {
    pub locale: String,
}

/// `POST /preferences/locale` — set the user's preferred locale.
/// Writes a `locale` cookie (and persists to `users.locale` for
/// authenticated users), then 303s back to Referer so the browser
/// reloads the page in the new locale.
///
/// htmx 4 beta3 does NOT honor `HX-Refresh` or `HX-Redirect` response
/// headers (verified against the vendored source), so we use classic
/// HTTP redirect semantics here. The base.html switcher submits via a
/// plain `<form>` (with Alpine wiring the change event) rather than
/// htmx so the browser follows the 303 naturally.
pub async fn update_locale(
    auth: AuthSession,
    State(state): State<AppState>,
    request_headers: HeaderMap,
    Form(body): Form<UpdateLocale>,
) -> Result<Response, AppError> {
    let candidate = body.locale.trim();
    if !SUPPORTED.contains(&candidate) {
        return Err(AppError::Validation(format!(
            "unsupported locale: {candidate}"
        )));
    }

    if let Some(user) = auth.user.as_ref() {
        state
            .users
            .update_preferences(user.0.id, Some(candidate), None)
            .await?;
    }

    let mut headers = HeaderMap::new();
    let cookie = format!(
        "locale={candidate}; Path=/; Max-Age=31536000; SameSite=Lax{secure}",
        secure = if state.config.auth.cookie_secure {
            "; Secure"
        } else {
            ""
        },
    );
    headers.insert(header::SET_COOKIE, cookie.parse().expect("valid cookie"));

    let target = request_headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| "/".to_owned());
    headers.insert(
        header::LOCATION,
        target.parse().unwrap_or_else(|_| "/".parse().unwrap()),
    );
    Ok((StatusCode::SEE_OTHER, headers).into_response())
}
