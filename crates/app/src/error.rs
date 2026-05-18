use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use todo_storage::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation: {0}")]
    Validation(String),
    #[error("rate limited")]
    RateLimited,
    #[error("template render error")]
    Template,
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Template | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn body(&self) -> String {
        match self {
            Self::Unauthorized => "unauthorized".into(),
            Self::Forbidden => "forbidden".into(),
            Self::NotFound => "not found".into(),
            Self::Conflict(m) | Self::Validation(m) | Self::Internal(m) => m.clone(),
            Self::RateLimited => "too many requests".into(),
            Self::Template => "template error".into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = ?self, "server error");
        } else {
            tracing::debug!(error = ?self, status = %status, "client error");
        }
        let body = self.body();
        (
            status,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            body,
        )
            .into_response()
    }
}

impl From<StorageError> for AppError {
    fn from(value: StorageError) -> Self {
        match value {
            StorageError::NotFound => Self::NotFound,
            StorageError::Conflict(msg) => Self::Conflict(msg),
            StorageError::InvalidCredentials => Self::Unauthorized,
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<validator::ValidationErrors> for AppError {
    fn from(value: validator::ValidationErrors) -> Self {
        Self::Validation(format_validation(&value))
    }
}

impl From<minijinja::Error> for AppError {
    fn from(err: minijinja::Error) -> Self {
        tracing::error!(error = %err, "template render failed");
        Self::Template
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

fn format_validation(errs: &validator::ValidationErrors) -> String {
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
        "validation failed".into()
    } else {
        parts.join("; ")
    }
}
