use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Form,
};
use minijinja::context;
use todo_domain::{NewTodo, TodoId};
use uuid::Uuid;
use validator::Validate;

use crate::{
    auth::{require_user, AuthSession},
    middleware::{CspNonce, RequestLocale, RequestTz},
    render::{base_context, localize_validation_errors, override_from_profile},
    AppError, AppState,
};

/// `GET /todos` — refresh the whole list (htmx target).
pub async fn list(
    auth: AuthSession,
    State(state): State<AppState>,
    Extension(locale): Extension<RequestLocale>,
    Extension(tz): Extension<RequestTz>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    let user_id = require_user(&auth)?;
    let user = auth.user.as_ref().expect("require_user succeeded");
    let (locale, tz) = override_from_profile(&user.0, locale, tz);
    let todos = state.list_todos_cached(user_id).await?;
    let html = state.templates.render(
        "partials/todo_list.html",
        context! {
            todos => todos,
            ..base_context(&locale, &tz, &nonce),
        },
    )?;
    Ok(html.into_response())
}

/// `POST /todos` — create. Returns 201 + a single-row partial. htmx
/// `hx-swap="afterbegin"` prepends it into `#todo-list`.
pub async fn create(
    auth: AuthSession,
    State(state): State<AppState>,
    Extension(locale): Extension<RequestLocale>,
    Extension(tz): Extension<RequestTz>,
    Extension(nonce): Extension<CspNonce>,
    Form(new): Form<NewTodo>,
) -> Result<Response, AppError> {
    let user_id = require_user(&auth)?;
    let user = auth.user.as_ref().expect("require_user succeeded");
    let (locale, tz) = override_from_profile(&user.0, locale, tz);
    // Resolve Fluent ids through the request's locale before the `?`
    // operator on storage errors swallows context. (Validation has no
    // `From` into AppError, so the explicit handling is mandatory.)
    if let Err(errs) = new.validate() {
        return Err(AppError::Validation(localize_validation_errors(
            &state.locales,
            &locale.0,
            &errs,
        )));
    }
    let todo = state.todos.create(user_id, new).await?;
    state.invalidate_todos_cache(user_id).await;
    metrics::counter!("todos_created_total").increment(1);
    let html = state.templates.render(
        "partials/todo.html",
        context! {
            todo => &todo,
            ..base_context(&locale, &tz, &nonce),
        },
    )?;
    Ok((StatusCode::CREATED, html).into_response())
}

/// `POST /todos/:id/toggle` — flip completed; returns the updated row partial.
pub async fn toggle(
    auth: AuthSession,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(locale): Extension<RequestLocale>,
    Extension(tz): Extension<RequestTz>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    let user_id = require_user(&auth)?;
    let user = auth.user.as_ref().expect("require_user succeeded");
    let (locale, tz) = override_from_profile(&user.0, locale, tz);
    let todo = state.todos.toggle(user_id, TodoId(id)).await?;
    state.invalidate_todos_cache(user_id).await;
    metrics::counter!("todos_toggled_total").increment(1);
    let html = state.templates.render(
        "partials/todo.html",
        context! {
            todo => &todo,
            ..base_context(&locale, &tz, &nonce),
        },
    )?;
    Ok(html.into_response())
}

/// `DELETE /todos/:id` — empty 200; htmx fades the row out client-side.
pub async fn delete(
    auth: AuthSession,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let user_id = require_user(&auth)?;
    state.todos.delete(user_id, TodoId(id)).await?;
    state.invalidate_todos_cache(user_id).await;
    metrics::counter!("todos_deleted_total").increment(1);
    Ok(StatusCode::OK.into_response())
}
