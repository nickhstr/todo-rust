use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;
use validator::Validate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub Uuid);

impl UserId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for UserId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub created_at: OffsetDateTime,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct NewUser {
    #[validate(email(message = "validation-email-invalid"))]
    pub email: String,
    #[validate(length(min = 12, max = 256, message = "validation-password-too-short"))]
    pub password: String,
}

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct Credentials {
    #[validate(email(message = "validation-email-invalid"))]
    pub email: String,
    #[validate(length(min = 1, max = 256, message = "validation-password-empty"))]
    pub password: String,
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Debug, Error)]
pub enum UserError {
    #[error("user not found")]
    NotFound,
    #[error("email already in use")]
    Conflict,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("validation failed: {0}")]
    Validation(String),
}
