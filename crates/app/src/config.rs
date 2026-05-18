use std::{net::SocketAddr, path::PathBuf};

use serde::{Deserialize, Serialize};
use todo_observability::{LogFormat, ObservabilityConfig};
use todo_storage::DbPoolConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DbPoolConfig,
    pub cache: CacheConfig,
    pub auth: AuthConfig,
    pub observability: ObservabilityConfig,
    pub templates_dir: PathBuf,
    pub static_dir: PathBuf,
    #[serde(default = "default_template_autoreload")]
    pub template_autoreload: bool,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub shutdown_timeout_secs: u64,
    /// When true, parse `X-Forwarded-For` and use its rightmost entry as the
    /// client IP for rate limiting. Only enable behind a trusted reverse
    /// proxy that overwrites the header — otherwise clients can spoof it.
    #[serde(default)]
    pub trust_forwarded_for: bool,
}

impl ServerConfig {
    pub fn socket_addr(&self) -> Result<SocketAddr, ConfigError> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|_| ConfigError::BadSocket(format!("{}:{}", self.host, self.port)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub url: String,
    pub pool_size: usize,
    pub default_ttl_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthConfig {
    /// Hex-encoded or raw secret. Must decode to ≥64 bytes.
    pub session_key: String,
    pub session_ttl_secs: i64,
    pub cookie_secure: bool,
    #[serde(default)]
    pub cookie_domain: String,
}

impl AuthConfig {
    /// Decode the session key, preferring hex; fall back to raw bytes.
    /// Errors if the decoded length is < 64.
    pub fn decoded_session_key(&self) -> Result<Vec<u8>, ConfigError> {
        let bytes =
            hex::decode(&self.session_key).unwrap_or_else(|_| self.session_key.as_bytes().to_vec());
        if bytes.len() < 64 {
            return Err(ConfigError::SessionKeyTooShort(bytes.len()));
        }
        Ok(bytes)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_rate_limit_enabled")]
    pub enabled: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn default_rate_limit_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_cfg(s: &str) -> AuthConfig {
        AuthConfig {
            session_key: s.into(),
            session_ttl_secs: 60,
            cookie_secure: false,
            cookie_domain: String::new(),
        }
    }

    #[test]
    fn session_key_decodes_hex() {
        let hex_str: String = (0..128)
            .map(|i| char::from(b"0123456789abcdef"[i % 16]))
            .collect();
        let decoded = key_cfg(&hex_str).decoded_session_key().unwrap();
        assert_eq!(decoded.len(), 64);
    }

    #[test]
    fn session_key_falls_back_to_raw_bytes() {
        let raw = "x".repeat(64);
        let decoded = key_cfg(&raw).decoded_session_key().unwrap();
        assert_eq!(decoded.len(), 64);
    }

    #[test]
    fn session_key_too_short_errors() {
        let err = key_cfg("short").decoded_session_key().unwrap_err();
        assert!(matches!(err, ConfigError::SessionKeyTooShort(_)));
    }

    #[test]
    fn rate_limit_default_is_enabled() {
        let cfg = Config::default();
        assert!(cfg.rate_limit.enabled);
    }

    #[test]
    fn rate_limit_config_serde_roundtrip() {
        // The field comes through env as APP__RATE_LIMIT__ENABLED, which the
        // `config` crate maps to `{rate_limit: {enabled: ...}}`. We exercise the
        // serde shape here so future renames break loudly.
        let disabled: RateLimitConfig = serde_json::from_str(r#"{"enabled": false}"#).unwrap();
        assert!(!disabled.enabled);
        let defaulted: RateLimitConfig = serde_json::from_str("{}").unwrap();
        assert!(defaulted.enabled);
    }
}

fn default_template_autoreload() -> bool {
    false
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".into(),
                trust_forwarded_for: false,
                port: 3000,
                shutdown_timeout_secs: 30,
            },
            database: DbPoolConfig::default(),
            cache: CacheConfig {
                url: "redis://localhost:6379".into(),
                pool_size: 10,
                default_ttl_secs: 300,
            },
            auth: AuthConfig {
                session_key: String::new(),
                session_ttl_secs: 60 * 60 * 24 * 30,
                cookie_secure: false,
                cookie_domain: String::new(),
            },
            observability: ObservabilityConfig::default(),
            templates_dir: PathBuf::from("templates"),
            static_dir: PathBuf::from("static"),
            template_autoreload: false,
            rate_limit: RateLimitConfig::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config build error: {0}")]
    Build(#[from] config::ConfigError),
    #[error("session key must be ≥ 64 bytes, got {0}")]
    SessionKeyTooShort(usize),
    #[error("invalid socket address: {0}")]
    BadSocket(String),
    #[error("missing required env var: {0}")]
    Missing(&'static str),
}

impl Config {
    /// Load from env. Honors:
    ///   - `APP__SECTION__KEY` env vars
    ///   - `DATABASE_URL`, `REDIS_URL`, `RUST_LOG` (12-factor shortcuts)
    ///   - dotenv `.env` already loaded by the caller
    pub fn from_env() -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let mut builder = config::Config::builder()
            .set_default("server.host", defaults.server.host)?
            .set_default("server.port", i64::from(defaults.server.port))?
            .set_default(
                "server.shutdown_timeout_secs",
                defaults.server.shutdown_timeout_secs as i64,
            )?
            .set_default(
                "server.trust_forwarded_for",
                defaults.server.trust_forwarded_for,
            )?
            .set_default("database.url", defaults.database.url)?
            .set_default(
                "database.max_connections",
                i64::from(defaults.database.max_connections),
            )?
            .set_default(
                "database.min_connections",
                i64::from(defaults.database.min_connections),
            )?
            .set_default(
                "database.acquire_timeout_secs",
                defaults.database.acquire_timeout_secs as i64,
            )?
            .set_default("cache.url", defaults.cache.url)?
            .set_default("cache.pool_size", defaults.cache.pool_size as i64)?
            .set_default(
                "cache.default_ttl_secs",
                defaults.cache.default_ttl_secs as i64,
            )?
            .set_default("auth.session_key", defaults.auth.session_key)?
            .set_default("auth.session_ttl_secs", defaults.auth.session_ttl_secs)?
            .set_default("auth.cookie_secure", defaults.auth.cookie_secure)?
            .set_default("auth.cookie_domain", defaults.auth.cookie_domain)?
            .set_default(
                "observability.service_name",
                defaults.observability.service_name,
            )?
            .set_default(
                "observability.otel_endpoint",
                defaults.observability.otel_endpoint,
            )?
            .set_default(
                "observability.otel_enabled",
                defaults.observability.otel_enabled,
            )?
            .set_default("observability.log_format", "pretty")?
            .set_default(
                "templates_dir",
                defaults.templates_dir.to_string_lossy().to_string(),
            )?
            .set_default(
                "static_dir",
                defaults.static_dir.to_string_lossy().to_string(),
            )?
            .set_default("template_autoreload", defaults.template_autoreload)?
            .set_default("rate_limit.enabled", defaults.rate_limit.enabled)?;

        // 12-factor shortcuts
        if let Ok(url) = std::env::var("DATABASE_URL") {
            builder = builder.set_override("database.url", url)?;
        }
        if let Ok(url) = std::env::var("REDIS_URL") {
            builder = builder.set_override("cache.url", url)?;
        }

        // `try_parsing(true)` lets numbers/bools come through as their real
        // types from env strings. We deliberately leave `list_separator` unset
        // so every value stays a scalar — comma-bearing secrets must not be
        // interpreted as a list.
        builder = builder.add_source(
            config::Environment::with_prefix("APP")
                .separator("__")
                .try_parsing(true)
                .convert_case(config::Case::Snake),
        );

        let cfg: Self = builder.build()?.try_deserialize()?;
        // Validate hot bits up front.
        let _ = cfg.auth.decoded_session_key()?;
        let _ = cfg.server.socket_addr()?;
        Ok(cfg)
    }

    pub fn log_format(&self) -> LogFormat {
        self.observability.log_format
    }
}
