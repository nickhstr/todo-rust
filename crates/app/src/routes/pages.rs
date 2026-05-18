use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
};
use minijinja::context;

use crate::{auth::AuthSession, AppState};

/// `GET /` — index. If unauthenticated, redirect to /login rather than 401-ing
/// the browser (better UX in a tab).
pub async fn index(
    auth: AuthSession,
    State(state): State<AppState>,
) -> Result<Response, crate::AppError> {
    let Some(user) = auth.user.as_ref() else {
        return Ok(Redirect::to("/login").into_response());
    };
    let todos = state.list_todos_cached(user.0.id).await?;
    let html = state.templates.render(
        "index.html",
        context! {
            user  => &user.0,
            todos => todos,
        },
    )?;
    Ok(html.into_response())
}
