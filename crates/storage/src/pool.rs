use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::StorageError;

pub type DbPool = PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbPoolConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_secs: u64,
}

impl Default for DbPoolConfig {
    fn default() -> Self {
        Self {
            url: "postgres://todo:todo@localhost:5432/todo".into(),
            max_connections: 20,
            min_connections: 2,
            acquire_timeout_secs: 5,
        }
    }
}

#[tracing::instrument(skip(cfg), fields(max = cfg.max_connections, min = cfg.min_connections))]
pub async fn build_pool(cfg: &DbPoolConfig) -> Result<DbPool, StorageError> {
    let pool = PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .min_connections(cfg.min_connections)
        .acquire_timeout(Duration::from_secs(cfg.acquire_timeout_secs))
        .connect(&cfg.url)
        .await?;
    tracing::info!("postgres pool ready");
    Ok(pool)
}

/// Quick liveness ping. Returns Ok(()) on success.
pub async fn ping(pool: &DbPool) -> Result<(), StorageError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await?;
    Ok(())
}
