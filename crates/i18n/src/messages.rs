//! Fluent bundle loader. One `Locales` value owns all loaded bundles;
//! `lookup` resolves a message id for a given locale with English
//! fallback.

use std::{borrow::Cow, collections::HashMap, path::PathBuf, sync::Arc};

use fluent_templates::{ArcLoader, Loader};
// Use the fluent_bundle re-exported by fluent-templates (0.16) to avoid a
// version conflict with the workspace's fluent-bundle 0.15 dependency.
use fluent_templates::fluent_bundle::FluentValue;
use unic_langid::{langid, LanguageIdentifier};

/// Args map type used by fluent-templates 0.13's `Loader` trait.
///
/// Note: fluent-templates 0.13 uses `fluent-bundle` 0.16 internally. The
/// workspace also has a separate `fluent-bundle` 0.15 dependency (via the
/// `fluent` 0.16 crate). We import `FluentValue` through `fluent-templates`
/// to ensure the types match the `Loader` trait's expectations.
pub type FluentArgs<'a> = HashMap<Cow<'static, str>, FluentValue<'a>>;

#[derive(Clone)]
pub struct Locales(Arc<ArcLoader>);

#[derive(Debug, thiserror::Error)]
pub enum LocalesError {
    #[error("failed to build fluent loader: {0}")]
    Build(String),
}

impl Locales {
    /// Build a loader from a directory tree (`<dir>/<lang>/*.ftl`).
    pub fn from_dir(dir: PathBuf) -> Result<Self, LocalesError> {
        let loader = ArcLoader::builder(&dir, langid!("en"))
            .build()
            .map_err(|e| LocalesError::Build(e.to_string()))?;
        Ok(Self(Arc::new(loader)))
    }

    /// Look up a message id. Returns the resolved string, or the id
    /// itself if the lookup fails in every fallback locale.
    ///
    /// `args` is a `HashMap<Cow<'static, str>, FluentValue>` as required
    /// by fluent-templates 0.13's `Loader` trait. This differs from the
    /// `fluent::FluentArgs` type used in some older docs — that type is from
    /// `fluent-bundle` 0.15, while fluent-templates 0.13 uses 0.16.
    pub fn lookup<'a>(
        &self,
        locale: &LanguageIdentifier,
        id: &str,
        args: Option<&FluentArgs<'a>>,
    ) -> String {
        if let Some(s) = self.0.try_lookup_complete(locale, id, args) {
            return s;
        }
        tracing::warn!(
            locale = %locale,
            key = id,
            "i18n: missing message id; rendering literal"
        );
        metrics::counter!(
            "i18n_missing_key_total",
            "locale" => locale.to_string(),
            "key" => id.to_owned(),
        )
        .increment(1);
        id.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locales_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("locales")
    }

    #[test]
    fn loads_and_looks_up_a_key() {
        let l = Locales::from_dir(locales_dir()).unwrap();
        let s = l.lookup(&langid!("en"), "app-name", None);
        assert_eq!(s, "Quiet Ledger");
    }

    #[test]
    fn substitutes_arguments() {
        let l = Locales::from_dir(locales_dir()).unwrap();
        let mut args: FluentArgs<'_> = HashMap::new();
        args.insert(Cow::Borrowed("name"), FluentValue::from("world"));
        let s = l.lookup(&langid!("en"), "greeting", Some(&args));
        // Fluent inserts Unicode directional isolate marks around variables
        // by default; assert the visible text instead of exact equality.
        assert!(s.contains("world"), "got: {s:?}");
    }

    #[test]
    fn missing_key_returns_literal_id() {
        let l = Locales::from_dir(locales_dir()).unwrap();
        let s = l.lookup(&langid!("en"), "totally-missing-id-xyz", None);
        assert_eq!(s, "totally-missing-id-xyz");
    }
}
