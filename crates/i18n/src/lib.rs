//! Internationalization and timezone-aware datetime formatting. This
//! crate is depended on by `app` and has no edges into `domain`,
//! `storage`, or `assets`.

pub mod datetime;
pub mod locale;
pub mod messages;
pub mod minijinja;
pub mod tz;

pub use datetime::{format_datetime, DateTimeStyle};
pub use locale::{negotiate, SUPPORTED};
pub use messages::{FluentArgs, Locales};
pub use minijinja::register;
pub use tz::{parse_tz, Tz, UTC};
