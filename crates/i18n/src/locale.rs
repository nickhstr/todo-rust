pub const SUPPORTED: &[&str] = &["en", "es", "fr", "de"];

pub fn negotiate(
    _accept_lang: Option<&str>,
    _cookie: Option<&str>,
    _profile: Option<&str>,
) -> unic_langid::LanguageIdentifier {
    "en".parse().unwrap()
}
