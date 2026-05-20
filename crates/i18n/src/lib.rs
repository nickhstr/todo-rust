//! Internationalization, timezone-aware datetime formatting, and a
//! content-hashed asset manifest. This crate is depended on by `app` and
//! has no edges into `domain` or `storage`.

pub mod assets;
pub mod datetime;
pub mod locale;
pub mod messages;
pub mod minijinja_helpers;
pub mod tz;

pub use assets::Assets;
pub use datetime::{format_datetime, DateTimeStyle};
pub use locale::{negotiate, SUPPORTED};
pub use messages::{FluentArgs, Locales};
pub use minijinja_helpers::{register, Helpers};
pub use tz::{parse_tz, Tz, UTC};
