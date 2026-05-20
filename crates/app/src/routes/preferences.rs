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
/// Writes a `locale` cookie (anonymous users) and persists to
/// `users.locale` for authenticated users. Returns `HX-Refresh: true`
/// so htmx triggers a full reload in the new locale.
pub async fn update_locale(
    auth: AuthSession,
    State(state): State<AppState>,
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
        secure = if state.config.auth.cookie_secure { "; Secure" } else { "" },
    );
    headers.insert(header::SET_COOKIE, cookie.parse().expect("valid cookie"));
    headers.insert("HX-Refresh", "true".parse().expect("valid header"));

    Ok((StatusCode::NO_CONTENT, headers).into_response())
}
