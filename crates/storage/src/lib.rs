//! Persistence layer: connection pool, migrations, repositories.

pub mod pool;
pub mod todo_repo;
pub mod user_repo;

use thiserror::Error;

pub use pool::{DbPool, DbPoolConfig};
pub use todo_repo::TodoRepository;
pub use user_repo::UserRepository;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Run all pending migrations against the given pool.
pub async fn run_migrations(pool: &DbPool) -> Result<(), StorageError> {
    MIGRATOR.run(pool).await.map_err(StorageError::from)
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("password hashing failed")]
    Hashing,
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}
