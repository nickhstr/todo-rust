//! Locale negotiation. Precedence per request:
//!   1. authenticated user's profile.locale (if in the supported set)
//!   2. `locale` cookie (if in the supported set)
//!   3. Accept-Language, negotiated against the supported set
//!   4. `en` fallback

use fluent_langneg::{negotiate_languages, parse_accepted_languages, NegotiationStrategy};
use unic_langid::{langid, LanguageIdentifier};

/// The locales the app ships translations for.
pub const SUPPORTED: &[&str] = &["en", "es", "fr", "de"];

/// The default. Always returned by `negotiate` if every other source fails.
pub fn default_locale() -> LanguageIdentifier {
    langid!("en")
}

/// Apply the precedence rules and return the chosen locale.
pub fn negotiate(
    accept_lang: Option<&str>,
    cookie: Option<&str>,
    profile: Option<&str>,
) -> LanguageIdentifier {
    if let Some(p) = profile.and_then(parse_supported) {
        return p;
    }
    if let Some(c) = cookie.and_then(parse_supported) {
        return c;
    }
    if let Some(al) = accept_lang {
        if let Some(matched) = negotiate_from_accept_language(al) {
            return matched;
        }
    }
    default_locale()
}

fn parse_supported(value: &str) -> Option<LanguageIdentifier> {
    let langid: LanguageIdentifier = value.parse().ok()?;
    if SUPPORTED.iter().any(|s| s == &langid.language.as_str()) {
        Some(langid)
    } else {
        None
    }
}

fn negotiate_from_accept_language(header: &str) -> Option<LanguageIdentifier> {
    // `parse_accepted_languages` discards the ";q=..." weights; we rely on
    // header order for priority, which matches what almost every browser
    // emits anyway.
    let requested = parse_accepted_languages(header);
    let available: Vec<LanguageIdentifier> = SUPPORTED
        .iter()
        .map(|s| s.parse().expect("valid SUPPORTED locale"))
        .collect();
    let default = default_locale();
    let chosen = negotiate_languages(
        &requested,
        &available,
        Some(&default),
        NegotiationStrategy::Filtering,
    );
    chosen.into_iter().next().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(s: &str) -> LanguageIdentifier { s.parse().unwrap() }

    #[test]
    fn profile_wins_over_cookie_and_header() {
        let r = negotiate(Some("de"), Some("es"), Some("fr"));
        assert_eq!(r, lang("fr"));
    }

    #[test]
    fn cookie_wins_over_header() {
        let r = negotiate(Some("de"), Some("es"), None);
        assert_eq!(r, lang("es"));
    }

    #[test]
    fn header_wins_when_no_profile_or_cookie() {
        let r = negotiate(Some("de,en;q=0.5"), None, None);
        assert_eq!(r, lang("de"));
    }

    #[test]
    fn unsupported_profile_falls_through() {
        let r = negotiate(Some("es"), None, Some("ja"));
        assert_eq!(r, lang("es"));
    }

    #[test]
    fn unsupported_everything_returns_default() {
        let r = negotiate(Some("ja,zh"), Some("ko"), Some("ar"));
        assert_eq!(r, lang("en"));
    }

    #[test]
    fn empty_inputs_return_default() {
        let r = negotiate(None, None, None);
        assert_eq!(r, lang("en"));
    }

    #[test]
    fn regional_variant_matches_base_language() {
        // fr-CA should pick up the fr bundle
        let r = negotiate(Some("fr-CA"), None, None);
        assert_eq!(r.language.as_str(), "fr");
    }
}
