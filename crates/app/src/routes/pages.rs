use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use minijinja::context;

use crate::{
    auth::AuthSession,
    middleware::{CspNonce, RequestLocale, RequestTz},
    render::{base_context, override_from_profile},
    AppState,
};

/// `GET /` — index. If unauthenticated, redirect to /login rather than 401-ing
/// the browser (better UX in a tab).
pub async fn index(
    auth: AuthSession,
    State(state): State<AppState>,
    Extension(locale): Extension<RequestLocale>,
    Extension(tz): Extension<RequestTz>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, crate::AppError> {
    let Some(user) = auth.user.as_ref() else {
        return Ok(Redirect::to("/login").into_response());
    };
    let (locale, tz) = override_from_profile(&user.0, locale, tz);

    let todos = state.list_todos_cached(user.0.id).await?;
    let html = state.templates.render(
        "index.html",
        context! {
            user => &user.0,
            todos => todos,
            ..base_context(&locale, &tz, &nonce),
        },
    )?;
    Ok(html.into_response())
}
