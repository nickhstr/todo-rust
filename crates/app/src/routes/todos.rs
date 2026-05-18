use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Form,
};
use minijinja::context;
use todo_domain::{NewTodo, TodoId};
use uuid::Uuid;
use validator::Validate;

use crate::{
    auth::{require_user, AuthSession},
    AppError, AppState,
};

/// `GET /todos` — refresh the whole list (htmx target).
pub async fn list(auth: AuthSession, State(state): State<AppState>) -> Result<Response, AppError> {
    let user_id = require_user(&auth)?;
    let todos = state.list_todos_cached(user_id).await?;
    let html = state
        .templates
        .render("partials/todo_list.html", context! { todos => todos })?;
    Ok(html.into_response())
}

/// `POST /todos` — create. Returns 201 + a single-row partial. htmx
/// `hx-swap="afterbegin"` prepends it into `#todo-list`.
pub async fn create(
    auth: AuthSession,
    State(state): State<AppState>,
    Form(new): Form<NewTodo>,
) -> Result<Response, AppError> {
    let user_id = require_user(&auth)?;
    new.validate()?;
    let todo = state.todos.create(user_id, new).await?;
    state.invalidate_todos_cache(user_id).await;
    metrics::counter!("todos_created_total").increment(1);
    let html = state
        .templates
        .render("partials/todo.html", context! { todo => &todo })?;
    Ok((StatusCode::CREATED, html).into_response())
}

/// `POST /todos/:id/toggle` — flip completed; returns the updated row partial.
pub async fn toggle(
    auth: AuthSession,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let user_id = require_user(&auth)?;
    let todo = state.todos.toggle(user_id, TodoId(id)).await?;
    state.invalidate_todos_cache(user_id).await;
    metrics::counter!("todos_toggled_total").increment(1);
    let html = state
        .templates
        .render("partials/todo.html", context! { todo => &todo })?;
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
