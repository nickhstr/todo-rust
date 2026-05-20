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

/// One discovered template, plus the list of stories that have a fixture for
/// it. Templates with no fixtures yield `stories: Vec::new()`.
#[derive(Debug)]
pub struct TemplateEntry {
    /// Logical name passed to minijinja, e.g. `"partials/todo.html"`.
    pub template_path: String,
    pub stories: Vec<Story>,
}

#[derive(Debug)]
pub struct Story {
    /// File stem of the fixture (no `.toml`), e.g. `"default"`, `"completed"`.
    pub name: String,
    /// Absolute path to the fixture file.
    pub file: PathBuf,
}

/// Walk `templates_dir` recursively; for each `.html` file whose basename
/// does NOT start with `_`, look up matching fixtures at
/// `fixtures_dir/<template-path-without-ext>/*.toml`.
///
/// Templates with no fixtures still appear in the returned list (with
/// empty `stories`) so the index UI can show them with a "add a fixture"
/// hint.
///
/// Story sort order: `default.toml` first if present, then the rest sorted
/// lexicographically. Template entries are sorted by `template_path`.
pub fn discover(
    templates_dir: &Path,
    fixtures_dir: &Path,
) -> Result<Vec<TemplateEntry>, FixtureError> {
    let mut out = Vec::new();
    walk_templates(templates_dir, templates_dir, fixtures_dir, &mut out)?;
    out.sort_by(|a, b| a.template_path.cmp(&b.template_path));
    Ok(out)
}

fn walk_templates(
    root: &Path,
    dir: &Path,
    fixtures_dir: &Path,
    out: &mut Vec<TemplateEntry>,
) -> Result<(), FixtureError> {
    let entries = std::fs::read_dir(dir).map_err(|e| FixtureError::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| FixtureError::io(dir, e))?;
        let path = entry.path();
        if path.is_dir() {
            walk_templates(root, &path, fixtures_dir, out)?;
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('_') {
            continue;
        }
        if !name.ends_with(".html") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| FixtureError::io(&path, std::io::Error::other("strip prefix failed")))?;
        let template_path = rel.to_string_lossy().replace('\\', "/");
        let stem = template_path.trim_end_matches(".html");
        let stories = stories_for(&fixtures_dir.join(stem))?;
        out.push(TemplateEntry {
            template_path,
            stories,
        });
    }
    Ok(())
}

fn stories_for(dir: &Path) -> Result<Vec<Story>, FixtureError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut stories = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| FixtureError::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| FixtureError::io(dir, e))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        stories.push(Story {
            name: name.to_string(),
            file: path,
        });
    }
    stories.sort_by(|a, b| match (a.name.as_str(), b.name.as_str()) {
        ("default", "default") => std::cmp::Ordering::Equal,
        ("default", _) => std::cmp::Ordering::Less,
        (_, "default") => std::cmp::Ordering::Greater,
        (a, b) => a.cmp(b),
    });
    Ok(stories)
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

    #[test]
    fn discover_walks_templates_and_pairs_fixtures() {
        let tdir = tempfile::tempdir().unwrap();
        let templates = tdir.path().join("templates");
        let fixtures = tdir.path().join("fixtures");
        std::fs::create_dir_all(templates.join("partials")).unwrap();
        std::fs::create_dir_all(fixtures.join("index")).unwrap();
        std::fs::create_dir_all(fixtures.join("partials/todo")).unwrap();

        // Templates (one underscore-prefixed should be skipped).
        std::fs::write(templates.join("index.html"), "x").unwrap();
        std::fs::write(templates.join("login.html"), "x").unwrap();
        std::fs::write(templates.join("_preview_shell.html"), "x").unwrap();
        std::fs::write(templates.join("partials/todo.html"), "x").unwrap();

        // Fixtures.
        std::fs::write(fixtures.join("index/default.toml"), "[ctx]\n").unwrap();
        std::fs::write(fixtures.join("index/empty.toml"), "[ctx]\n").unwrap();
        std::fs::write(fixtures.join("partials/todo/default.toml"), "[ctx]\n").unwrap();
        std::fs::write(fixtures.join("partials/todo/zebra.toml"), "[ctx]\n").unwrap();

        let idx = discover(&templates, &fixtures).unwrap();

        // Underscore-prefixed templates are skipped.
        let paths: Vec<_> = idx.iter().map(|e| e.template_path.as_str()).collect();
        assert!(paths.contains(&"index.html"));
        assert!(paths.contains(&"login.html"));
        assert!(paths.contains(&"partials/todo.html"));
        assert!(
            !paths.iter().any(|p| p.starts_with("_")),
            "underscore templates must be skipped, got {:?}",
            paths
        );

        // login.html has no fixtures dir → empty stories.
        let login = idx
            .iter()
            .find(|e| e.template_path == "login.html")
            .unwrap();
        assert!(login.stories.is_empty(), "login should have no stories");

        // index has default + empty, default first.
        let index = idx
            .iter()
            .find(|e| e.template_path == "index.html")
            .unwrap();
        let names: Vec<&str> = index.stories.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["default", "empty"]);

        // todo has default + zebra (default pinned first even with z later).
        let todo = idx
            .iter()
            .find(|e| e.template_path == "partials/todo.html")
            .unwrap();
        let names: Vec<&str> = todo.stories.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["default", "zebra"]);
    }
}
