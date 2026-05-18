//! Thin Valkey/Redis helper. Get-or-compute with TTL; explicit invalidation.

use std::{future::Future, time::Duration};

use fred::{
    interfaces::{ClientLike, KeysInterface},
    prelude::{Builder, RedisConfig, RedisError, RedisPool},
    types::Expiration,
};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone)]
pub struct Cache {
    pool: Option<RedisPool>,
    default_ttl: Duration,
}

impl Cache {
    pub fn new(pool: Option<RedisPool>, default_ttl: Duration) -> Self {
        Self { pool, default_ttl }
    }

    /// Disabled cache: never reads, never writes. Used when Redis is unreachable.
    pub fn disabled() -> Self {
        Self {
            pool: None,
            default_ttl: Duration::from_secs(60),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.pool.is_some()
    }

    /// Get a value, deserializing JSON.
    #[tracing::instrument(skip(self), fields(key = %key))]
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let pool = self.pool.as_ref()?;
        let result: Result<Option<String>, RedisError> = pool.get(key.to_owned()).await;
        match result {
            Ok(Some(raw)) => match serde_json::from_str::<T>(&raw) {
                Ok(v) => {
                    metrics::counter!("cache_operations_total", "op" => "get", "result" => "hit")
                        .increment(1);
                    Some(v)
                }
                Err(err) => {
                    tracing::warn!(%err, "cache deserialize failed; treating as miss");
                    metrics::counter!(
                        "cache_operations_total",
                        "op" => "get",
                        "result" => "decode_error",
                    )
                    .increment(1);
                    None
                }
            },
            Ok(None) => {
                metrics::counter!("cache_operations_total", "op" => "get", "result" => "miss")
                    .increment(1);
                None
            }
            Err(err) => {
                tracing::warn!(%err, "cache GET failed");
                metrics::counter!("cache_operations_total", "op" => "get", "result" => "error")
                    .increment(1);
                None
            }
        }
    }

    #[tracing::instrument(skip(self, value), fields(key = %key))]
    pub async fn put<T: Serialize>(&self, key: &str, value: &T, ttl: Option<Duration>) {
        let Some(pool) = self.pool.as_ref() else {
            return;
        };
        let ttl = ttl.unwrap_or(self.default_ttl);
        let raw = match serde_json::to_string(value) {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(%err, "cache serialize failed; skipping put");
                return;
            }
        };
        let result: Result<(), RedisError> = pool
            .set(
                key.to_owned(),
                raw,
                Some(Expiration::EX(ttl.as_secs() as i64)),
                None,
                false,
            )
            .await;
        if let Err(err) = result {
            tracing::warn!(%err, "cache SET failed");
            metrics::counter!("cache_operations_total", "op" => "set", "result" => "error")
                .increment(1);
        } else {
            metrics::counter!("cache_operations_total", "op" => "set", "result" => "ok")
                .increment(1);
        }
    }

    #[tracing::instrument(skip(self), fields(key = %key))]
    pub async fn invalidate(&self, key: &str) {
        let Some(pool) = self.pool.as_ref() else {
            return;
        };
        let result: Result<u64, RedisError> = pool.del(key.to_owned()).await;
        if let Err(err) = result {
            tracing::warn!(%err, "cache DEL failed");
            metrics::counter!("cache_operations_total", "op" => "del", "result" => "error")
                .increment(1);
        } else {
            metrics::counter!("cache_operations_total", "op" => "del", "result" => "ok")
                .increment(1);
        }
    }

    /// Read-through: returns cached value if present, otherwise calls `compute`,
    /// stores the result, and returns it.
    pub async fn get_or_compute<T, F, Fut, E>(
        &self,
        key: &str,
        ttl: Option<Duration>,
        compute: F,
    ) -> Result<T, E>
    where
        T: Serialize + DeserializeOwned + Send + Sync,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>> + Send,
    {
        if let Some(v) = self.get::<T>(key).await {
            return Ok(v);
        }
        let value = compute().await?;
        self.put(key, &value, ttl).await;
        Ok(value)
    }
}

/// Build a `RedisPool` from a URL with the given pool size, and `init()` it.
pub async fn build_redis_pool(url: &str, size: usize) -> Result<RedisPool, RedisError> {
    let config = RedisConfig::from_url(url)?;
    let pool = Builder::from_config(config).build_pool(size.max(1))?;
    pool.init().await?;
    Ok(pool)
}
