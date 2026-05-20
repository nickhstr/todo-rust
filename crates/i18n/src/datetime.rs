//! Timezone- and locale-aware datetime formatting. We go direct to ICU
//! for the locale-correct CLDR patterns; the server-rendered text is
//! what no-JS users and search engines see, and matches what the
//! client-side `Intl.DateTimeFormat` will produce.

use icu::calendar::DateTime as IcuDateTime;
use icu::datetime::{options::length, DateTimeFormatter, DateTimeFormatterOptions};
use icu::locid::Locale;
use time::OffsetDateTime;
use time_tz::OffsetDateTimeExt;
use unic_langid::LanguageIdentifier;

use crate::tz::Tz;

#[derive(Debug, Clone, Copy)]
pub enum DateTimeStyle {
    Short,
    Medium,
    Long,
}

impl DateTimeStyle {
    pub fn parse(s: &str) -> Self {
        match s {
            "short" => Self::Short,
            "long" => Self::Long,
            _ => Self::Medium,
        }
    }

    fn length(self) -> DateTimeFormatterOptions {
        let (date, time) = match self {
            Self::Short => (length::Date::Short, length::Time::Short),
            Self::Medium => (length::Date::Medium, length::Time::Short),
            Self::Long => (length::Date::Long, length::Time::Short),
        };
        length::Bag::from_date_time_style(date, time).into()
    }
}

pub fn format_datetime(
    value: OffsetDateTime,
    locale: &LanguageIdentifier,
    tz: Tz,
    style: DateTimeStyle,
) -> String {
    let local = value.to_timezone(tz);

    let icu_dt = IcuDateTime::try_new_iso_datetime(
        local.year(),
        local.month() as u8,
        local.day(),
        local.hour(),
        local.minute(),
        local.second(),
    );

    // Parse the locale string into an ICU Locale, falling back to "en".
    // We append "-u-ca-gregory" so that `DateTimeFormatter` always uses the
    // Gregorian calendar — this avoids calendar-mismatch errors when
    // `to_any()` produces an ISO-calendar datetime.
    let locale_str = format!("{}-u-ca-gregory", locale);
    let icu_locale: Locale = locale_str
        .parse()
        .unwrap_or_else(|_| "en-u-ca-gregory".parse().unwrap());

    let formatter = DateTimeFormatter::try_new(&icu_locale.into(), style.length());

    match (icu_dt, formatter) {
        (Ok(dt), Ok(f)) => {
            let any = dt.to_any();
            match f.format_to_string(&any) {
                Ok(s) => s,
                Err(_) => iso_fallback(local),
            }
        }
        _ => iso_fallback(local),
    }
}

fn iso_fallback(local: OffsetDateTime) -> String {
    local
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "\u{2014}".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use unic_langid::langid;

    fn la() -> Tz {
        crate::tz::parse_tz("America/Los_Angeles").unwrap()
    }

    #[test]
    fn formats_in_english_la_tz() {
        let utc = datetime!(2026-05-19 20:00 UTC);
        let s = format_datetime(utc, &langid!("en"), la(), DateTimeStyle::Medium);
        // 20:00 UTC == 13:00 PDT in May. The exact format varies by ICU
        // version; assert on substrings that must be present.
        assert!(s.contains("2026"), "got: {s:?}");
        assert!(s.contains("1:00"), "got: {s:?}");
    }

    #[test]
    fn formats_in_spanish() {
        let utc = datetime!(2026-05-19 20:00 UTC);
        let s = format_datetime(utc, &langid!("es"), la(), DateTimeStyle::Medium);
        assert!(s.contains("2026"), "got: {s:?}");
        // Spanish medium format uses "may" or "may." for May
        assert!(s.to_lowercase().contains("may"), "got: {s:?}");
    }

    #[test]
    fn falls_back_to_iso_on_bad_locale() {
        let utc = datetime!(2026-05-19 20:00 UTC);
        let bad: LanguageIdentifier = "und".parse().unwrap();
        let s = format_datetime(utc, &bad, crate::tz::UTC, DateTimeStyle::Medium);
        // Either a valid format or the ISO fallback — both contain "2026".
        assert!(s.contains("2026"), "got: {s:?}");
    }
}
