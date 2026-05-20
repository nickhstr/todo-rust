//! Timezone parsing. Wraps `time_tz`'s static `Tz` so callers don't need
//! to depend on `time_tz` directly.

use time_tz::Tz as TimeTz;

pub type Tz = &'static TimeTz;

pub const UTC: Tz = time_tz::timezones::db::UTC;

/// Parse an IANA timezone name (e.g. "America/Los_Angeles"). Returns
/// `None` for unknown names. Whitespace and a leading `+` are tolerated
/// since browsers occasionally encode the value oddly.
pub fn parse_tz(name: &str) -> Option<Tz> {
    let trimmed = name.trim().trim_start_matches('+');
    if trimmed.is_empty() {
        return None;
    }
    time_tz::timezones::find_by_name(trimmed).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_zones() {
        assert!(parse_tz("America/Los_Angeles").is_some());
        assert!(parse_tz("Europe/Berlin").is_some());
        assert!(parse_tz("UTC").is_some());
        assert!(parse_tz("Asia/Tokyo").is_some());
    }

    #[test]
    fn rejects_unknown() {
        assert!(parse_tz("Atlantis/Sunken_City").is_none());
        assert!(parse_tz("not-a-tz").is_none());
        assert!(parse_tz("").is_none());
    }

    #[test]
    fn tolerates_whitespace_and_leading_plus() {
        assert!(parse_tz("  UTC  ").is_some());
        assert!(parse_tz("+America/New_York").is_some());
    }
}
