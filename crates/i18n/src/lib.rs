//! Internationalization and timezone-aware datetime formatting. The
//! minijinja helpers in this crate need `todo_assets::Assets` to
//! register the `asset()` helper; that dependency goes away once
//! the helper itself moves into `todo-assets`.

pub mod datetime;
pub mod locale;
pub mod messages;
pub mod minijinja_helpers;
pub mod tz;

pub use datetime::{format_datetime, DateTimeStyle};
pub use locale::{negotiate, SUPPORTED};
pub use messages::{FluentArgs, Locales};
pub use minijinja_helpers::{register, Helpers};
pub use tz::{parse_tz, Tz, UTC};
