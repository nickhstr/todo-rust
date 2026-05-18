//! Public surface used by integration tests and the binary entrypoint.

pub mod auth;
pub mod cache;
pub mod config;
pub mod error;
pub mod middleware;
pub mod router;
pub mod routes;
pub mod state;
pub mod templates;

pub use config::Config;
pub use error::AppError;
pub use router::build_router;
pub use state::AppState;
