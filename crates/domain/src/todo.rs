use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;
use validator::Validate;

use crate::user::UserId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TodoId(pub Uuid);

impl TodoId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for TodoId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TodoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for TodoId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: TodoId,
    pub owner_id: UserId,
    pub title: String,
    pub completed: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct NewTodo {
    #[validate(length(min = 1, max = 280, message = "validation-todo-title-length"))]
    pub title: String,
}

#[derive(Debug, Clone, Default, Deserialize, Validate)]
pub struct TodoUpdate {
    #[validate(length(min = 1, max = 280, message = "validation-todo-title-length"))]
    pub title: Option<String>,
    pub completed: Option<bool>,
}

#[derive(Debug, Error)]
pub enum TodoError {
    #[error("todo not found")]
    NotFound,
    #[error("validation failed: {0}")]
    Validation(String),
}
