//! Fixture file loading. A fixture is a TOML file with an optional `[meta]`
//! section (free-form description used only by the index UI) and an optional
//! `[ctx]` table whose contents become the render context for one template
//! invocation.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("io error reading fixture {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("toml parse error in fixture {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl FixtureError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
    fn parse(path: &Path, source: toml::de::Error) -> Self {
        Self::Parse {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct FixtureMeta {
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug)]
pub struct LoadedFixture {
    pub meta: FixtureMeta,
    /// Render context. Always either a JSON object (when `[ctx]` was present)
    /// or JSON null (when omitted).
    pub ctx: serde_json::Value,
}

/// Internal helper struct that deserializes the whole TOML file. Splits into
/// `meta` and `ctx` so we can return the typed meta + freeform ctx separately.
#[derive(Debug, Default, Deserialize)]
struct FixtureFile {
    #[serde(default)]
    meta: FixtureMeta,
    #[serde(default)]
    ctx: Option<serde_json::Value>,
}

pub fn load_fixture(path: &Path) -> Result<LoadedFixture, FixtureError> {
    let raw = std::fs::read_to_string(path).map_err(|e| FixtureError::io(path, e))?;
    let file: FixtureFile = toml::from_str(&raw).map_err(|e| FixtureError::parse(path, e))?;
    Ok(LoadedFixture {
        meta: file.meta,
        ctx: file.ctx.unwrap_or(serde_json::Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn load_fixture_parses_ctx_into_json_value() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            r#"
[meta]
description = "a fixture"

[ctx]
title = "Hello"
count = 3
flag = true
"#
        )
        .unwrap();

        let loaded = load_fixture(tmp.path()).unwrap();
        assert_eq!(loaded.meta.description.as_deref(), Some("a fixture"));
        assert_eq!(loaded.ctx["title"], serde_json::json!("Hello"));
        assert_eq!(loaded.ctx["count"], serde_json::json!(3));
        assert_eq!(loaded.ctx["flag"], serde_json::json!(true));
    }

    #[test]
    fn load_fixture_missing_file_errors() {
        let err = load_fixture(std::path::Path::new("/no/such/file.toml")).unwrap_err();
        assert!(matches!(err, FixtureError::Io { .. }));
    }

    #[test]
    fn load_fixture_invalid_toml_errors() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "this is not = valid = toml = =").unwrap();
        let err = load_fixture(tmp.path()).unwrap_err();
        assert!(matches!(err, FixtureError::Parse { .. }));
    }

    #[test]
    fn load_fixture_empty_ctx_yields_null() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            r#"[meta]
description = "no ctx""#
        )
        .unwrap();
        let loaded = load_fixture(tmp.path()).unwrap();
        assert!(
            loaded.ctx.is_null() || loaded.ctx.is_object(),
            "ctx should be JSON null or empty object; got {:?}",
            loaded.ctx
        );
    }
}
