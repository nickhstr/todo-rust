use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension, Form,
};
use minijinja::context;
use serde::Deserialize;
use todo_domain::{Credentials, NewUser};
use validator::Validate;

use crate::{
    auth::{AuthSession, LoginCredentials},
    middleware::{CspNonce, RequestLocale, RequestTz},
    render::base_context,
    AppError, AppState,
};

// AuthUserRecord is referenced as `crate::auth::AuthUserRecord` below.

/// `GET /login` — render form. Pass `?next=/path` through to the template so
/// a successful login can land back where the user came from.
pub async fn login_form(
    State(state): State<AppState>,
    Query(q): Query<NextQuery>,
    Extension(locale): Extension<RequestLocale>,
    Extension(tz): Extension<RequestTz>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    let html = state.templates.render(
        "login.html",
        context! {
            next => safe_next(&q.next),
            error => "",
            dev_login_enabled => state.config.dev.enabled_email().is_some(),
            ..base_context(&locale, &tz, &nonce),
        },
    )?;
    Ok(html.into_response())
}

#[derive(Debug, Default, Deserialize)]
pub struct NextQuery {
    pub next: Option<String>,
}

/// `POST /login` — verify creds, set session cookie, redirect.
pub async fn login(
    mut auth: AuthSession,
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(locale): Extension<RequestLocale>,
    Extension(tz): Extension<RequestTz>,
    Extension(nonce): Extension<CspNonce>,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    if let Err(errs) = creds.validate() {
        metrics::counter!("auth_logins_total", "result" => "failure").increment(1);
        // Render the form back with the validation message. Plain-text 422 would
        // make the htmx swap dump raw text into the page on bad input.
        return render_login_form(
            &state,
            &locale,
            &tz,
            &nonce,
            &creds.next,
            &format_validation_message(&errs),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }
    let user = match auth.authenticate(LoginCredentials::from(&creds)).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            metrics::counter!("auth_logins_total", "result" => "failure").increment(1);
            return render_login_form(
                &state,
                &locale,
                &tz,
                &nonce,
                &creds.next,
                "incorrect email or password",
                StatusCode::UNAUTHORIZED,
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "authenticate failed");
            return Err(AppError::Internal("authentication backend error".into()));
        }
    };
    auth.login(&user)
        .await
        .map_err(|e| AppError::Internal(format!("session login failed: {e}")))?;
    metrics::counter!("auth_logins_total", "result" => "success").increment(1);

    let target = safe_next(&creds.next).unwrap_or_else(|| "/".to_owned());
    Ok(redirect_after_form(&headers, &target))
}

/// `GET /signup` — render form.
pub async fn signup_form(
    State(state): State<AppState>,
    Extension(locale): Extension<RequestLocale>,
    Extension(tz): Extension<RequestTz>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    let html = state.templates.render(
        "signup.html",
        context! {
            error => "",
            ..base_context(&locale, &tz, &nonce),
        },
    )?;
    Ok(html.into_response())
}

/// `POST /signup` — create user, auto-login, redirect to /.
pub async fn signup(
    mut auth: AuthSession,
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(locale): Extension<RequestLocale>,
    Extension(tz): Extension<RequestTz>,
    Extension(nonce): Extension<CspNonce>,
    Form(new): Form<NewUser>,
) -> Result<Response, AppError> {
    if let Err(errs) = new.validate() {
        metrics::counter!("auth_signups_total", "result" => "failure").increment(1);
        return render_signup_form(
            &state,
            &locale,
            &tz,
            &nonce,
            &format_validation_message(&errs),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    let user = match state.users.create(new).await {
        Ok(u) => u,
        Err(todo_storage::StorageError::Conflict(_)) => {
            metrics::counter!("auth_signups_total", "result" => "failure").increment(1);
            return render_signup_form(
                &state,
                &locale,
                &tz,
                &nonce,
                "an account with that email already exists",
                StatusCode::CONFLICT,
            );
        }
        Err(err) => {
            metrics::counter!("auth_signups_total", "result" => "failure").increment(1);
            return Err(err.into());
        }
    };

    // Skip the post-signup `authenticate()` round-trip: we already have the
    // fresh User from `create`, and a second argon2 verify would cost ~50–100ms
    // for nothing. `axum-login::login` just needs an `AuthUser` impl.
    let record = crate::auth::AuthUserRecord(user);
    auth.login(&record)
        .await
        .map_err(|e| AppError::Internal(format!("post-signup session login failed: {e}")))?;
    metrics::counter!("auth_signups_total", "result" => "success").increment(1);

    Ok(redirect_after_form(&headers, "/"))
}

/// `POST /logout` — clear session, redirect to /login.
pub async fn logout(mut auth: AuthSession, headers: HeaderMap) -> Result<Response, AppError> {
    auth.logout()
        .await
        .map_err(|e| AppError::Internal(format!("logout failed: {e}")))?;
    Ok(redirect_after_form(&headers, "/login"))
}

fn render_login_form(
    state: &AppState,
    locale: &RequestLocale,
    tz: &RequestTz,
    nonce: &CspNonce,
    next: &Option<String>,
    msg: &str,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = state.templates.render(
        "login.html",
        context! {
            next => safe_next(next),
            error => msg,
            dev_login_enabled => state.config.dev.enabled_email().is_some(),
            ..base_context(locale, tz, nonce),
        },
    )?;
    // HX-Retarget=body + HX-Reswap=outerHTML tells htmx to swap the whole page,
    // not just the form's default target, on a non-2xx response.
    Ok((status, hx_full_swap(), html).into_response())
}

fn render_signup_form(
    state: &AppState,
    locale: &RequestLocale,
    tz: &RequestTz,
    nonce: &CspNonce,
    msg: &str,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = state.templates.render(
        "signup.html",
        context! {
            error => msg,
            ..base_context(locale, tz, nonce),
        },
    )?;
    Ok((status, hx_full_swap(), html).into_response())
}

fn hx_full_swap() -> [(header::HeaderName, &'static str); 2] {
    [
        (header::HeaderName::from_static("hx-retarget"), "body"),
        (header::HeaderName::from_static("hx-reswap"), "outerHTML"),
    ]
}

fn format_validation_message(errs: &validator::ValidationErrors) -> String {
    let mut parts = Vec::new();
    for (field, kind) in errs.field_errors() {
        for e in kind {
            let msg = e
                .message
                .as_ref()
                .map(std::string::ToString::to_string)
                .unwrap_or_else(|| e.code.to_string());
            parts.push(format!("{field}: {msg}"));
        }
    }
    if parts.is_empty() {
        "please check your input".into()
    } else {
        parts.join("; ")
    }
}

/// htmx-aware redirect: if `HX-Request` is set, return 200 + `HX-Redirect`
/// (which makes htmx do a full-page navigation). Otherwise classic 303.
fn redirect_after_form(headers: &HeaderMap, target: &str) -> Response {
    if headers.get("HX-Request").is_some() {
        let mut res = StatusCode::OK.into_response();
        if let Ok(val) = header::HeaderValue::from_str(target) {
            res.headers_mut().insert("HX-Redirect", val);
        }
        res
    } else {
        Redirect::to(target).into_response()
    }
}

/// Only honor relative, single-slash, same-origin paths. Rejects `//evil.com`.
pub fn safe_next(raw: &Option<String>) -> Option<String> {
    let v = raw.as_deref()?;
    if v.starts_with('/') && !v.starts_with("//") {
        Some(v.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::safe_next;

    #[test]
    fn safe_next_accepts_relative_paths() {
        assert_eq!(safe_next(&Some("/".into())), Some("/".into()));
        assert_eq!(safe_next(&Some("/todos".into())), Some("/todos".into()));
    }

    #[test]
    fn safe_next_rejects_scheme_relative_and_absolute() {
        assert_eq!(safe_next(&Some("//evil.com".into())), None);
        assert_eq!(safe_next(&Some("https://evil.com".into())), None);
        assert_eq!(safe_next(&Some("evil.com".into())), None);
        assert_eq!(safe_next(&None), None);
    }
}
