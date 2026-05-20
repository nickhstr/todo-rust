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

// Deliberately NO `From<validator::ValidationErrors> for AppError`.
// `?` would otherwise stringify Fluent ids into the response body and skip
// localization. Handlers must call `render::localize_validation_errors`
// explicitly before turning the result into an `AppError::Validation`.

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
