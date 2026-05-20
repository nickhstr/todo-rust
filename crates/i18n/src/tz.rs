use time_tz::Tz as TimeTz;

pub type Tz = &'static TimeTz;
pub const UTC: Tz = time_tz::timezones::db::UTC;

pub fn parse_tz(_s: &str) -> Option<Tz> {
    None
}
