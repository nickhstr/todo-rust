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
/// Writes the `locale` cookie and (for authenticated users) persists
/// it to `users.locale`.
///
/// htmx callers (identified by `HX-Request`) get a 204 No Content;
/// they fire `location.reload()` themselves via `hx-on::after:request`
/// — htmx 4 beta3 doesn't parse `HX-Refresh`/`HX-Redirect` response
/// headers, so the reload is wired in the template.
///
/// Plain form posts (the `<noscript>` fallback, or anyone hitting the
/// endpoint without htmx) get a classic 303 to Referer so the browser
/// follows naturally.
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

    if request_headers.get("HX-Request").is_some() {
        Ok((StatusCode::NO_CONTENT, headers).into_response())
    } else {
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
}
