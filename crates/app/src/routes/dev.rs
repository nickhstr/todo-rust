//! Local-development convenience routes. The whole module is compiled out of
//! `--release` builds via `cfg(debug_assertions)` in `router.rs`, so a leaked
//! config flag in production cannot expose these endpoints.

use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
};

use crate::{
    auth::{AuthSession, AuthUserRecord},
    AppError, AppState,
};

/// `POST /dev/login` — log in as the configured dev user without a password.
/// Returns 404 if dev login isn't active so an accidentally enabled config in
/// production looks identical to a missing route.
pub async fn auto_login(
    mut auth: AuthSession,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let Some(email) = state.config.dev.enabled_email() else {
        return Err(AppError::NotFound);
    };

    let user = state
        .users
        .find_by_email(email)
        .await?
        .ok_or_else(|| AppError::Internal(format!("dev user {email} missing from db")))?;

    let record = AuthUserRecord(user);
    auth.login(&record)
        .await
        .map_err(|e| AppError::Internal(format!("dev session login failed: {e}")))?;

    tracing::warn!(email, "dev auto-login used");
    Ok(Redirect::to("/").into_response())
}
