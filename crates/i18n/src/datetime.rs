use time::OffsetDateTime;

pub enum DateTimeStyle { Short, Medium, Long }

pub fn format_datetime(
    _value: OffsetDateTime,
    _locale: &unic_langid::LanguageIdentifier,
    _tz: crate::tz::Tz,
    _style: DateTimeStyle,
) -> String {
    String::new()
}
