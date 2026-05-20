//! Dev-only template preview surface, mounted at `/__preview/*` when
//! `cfg(debug_assertions)` AND `state.config.dev.preview_enabled`. Entire
//! module is compiled out of `--release` builds via the `cfg` gate in
//! `lib.rs`.

pub mod fixtures;
pub mod shell;
