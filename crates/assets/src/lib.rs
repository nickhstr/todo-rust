//! Content-hashed static asset manifest and the `asset()` minijinja helper.
//! Used by the HTTP layer (`todo-app`) to render long-cacheable URLs for
//! static files and to serve those hashed URLs.

pub mod manifest;

pub use manifest::{Assets, AssetsError};
