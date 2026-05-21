# Template Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Storybook-style preview surface for minijinja templates: pick a template, pick a hand-edited TOML fixture, render it in the browser. Lives behind a dev-only gate (debug build + `APP__DEV__PREVIEW=true`).

**Architecture:** New `crates/app/src/preview/` module gated entirely on `cfg(debug_assertions)`, mounted at `/__preview/*` from `router.rs` next to `/dev/login`. Reuses the production `Templates` env (so `t()` / `asset()` resolve identically) and the existing csp_nonce middleware. Fixtures are TOML files at `fixtures/templates/<template-path-without-ext>/<story>.toml` parsed straight into `serde_json::Value`. Partials get wrapped in a minimal host shell (`templates/_preview_shell.html`) that loads CSS + Alpine but deliberately not htmx.

**Tech Stack:** Rust, axum 0.7, minijinja, the `toml` crate (new dep), serde_json. Tests via testcontainers (Postgres in Docker) + reqwest.

**Spec:** [`docs/superpowers/specs/2026-05-20-template-preview-design.md`](../specs/2026-05-20-template-preview-design.md)

---

## File Structure

**New files:**

```
crates/app/src/preview/
  mod.rs            # module root, re-exports router()
  fixtures.rs       # discovery walker + TOML loader
  shell.rs          # host shell helper (renders _preview_shell.html)
  routes.rs         # GET /__preview, GET /__preview/render/*path

crates/app/tests/
  preview_flow.rs   # integration tests

templates/
  _preview_index.html   # index page (underscore = skipped by discovery)
  _preview_shell.html   # host shell for partials

fixtures/templates/
  index/{default,empty,many-items}.toml
  login/{default,with-validation-error}.toml
  signup/{default,with-validation-error}.toml
  partials/todo/{default,completed,long-title}.toml
  partials/todo_list/{default,empty}.toml
```

**Modified files:**

- `crates/app/src/lib.rs` — add `#[cfg(debug_assertions)] pub mod preview;`
- `crates/app/src/config.rs` — extend `DevConfig` with `preview_enabled` + `preview_fixtures_dir`; add defaults to `from_env()`
- `crates/app/src/router.rs` — mount `preview::router()` inside the existing `#[cfg(debug_assertions)]` block, behind `state.config.dev.preview_enabled`
- `crates/app/Cargo.toml` — add `toml` dep
- `Cargo.toml` (workspace) — add `toml = "0.8"` to `[workspace.dependencies]`
- `docker/compose.dev.yaml` — set `APP__DEV__PREVIEW=true`
- `CLAUDE.md` — short paragraph under "Where to add things"
- `README.md` — one-line developer-tools mention

---

## Task 1: Add `toml` workspace dependency and to app crate

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/app/Cargo.toml`

- [ ] **Step 1: Add `toml` to workspace dependencies**

Open the workspace `Cargo.toml` at the repo root. Find the `[workspace.dependencies]` table (it's the existing table that lists shared deps like `axum`, `sqlx`, `serde`, etc.). Add this line, kept in alphabetical order with the other entries:

```toml
toml = "0.8"
```

- [ ] **Step 2: Add `toml` to the app crate**

Open `crates/app/Cargo.toml`. Add this line to `[dependencies]` (alphabetical order — goes after `time-tz` and before `tokio`):

```toml
toml = { workspace = true }
```

- [ ] **Step 3: Verify the workspace still builds**

Run: `cargo build --workspace`
Expected: PASS with no new warnings.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/app/Cargo.toml Cargo.lock
git commit -m "chore(deps): add toml parser for preview tool"
```

---

## Task 2: Extend `DevConfig` with `preview_enabled` and `preview_fixtures_dir`

**Files:**
- Modify: `crates/app/src/config.rs`

- [ ] **Step 1: Write the failing test**

Add this test to the `tests` module inside `crates/app/src/config.rs` (right after `rate_limit_config_serde_roundtrip`):

```rust
#[test]
fn dev_preview_defaults_are_off_and_fixtures_dir() {
    let cfg = Config::default();
    assert!(!cfg.dev.preview_enabled, "preview must default to off");
    assert_eq!(
        cfg.dev.preview_fixtures_dir,
        std::path::PathBuf::from("fixtures/templates"),
        "preview_fixtures_dir default"
    );
}

#[test]
fn dev_preview_serde_roundtrip() {
    let json = r#"{"auto_login_email": "", "preview_enabled": true, "preview_fixtures_dir": "x/y"}"#;
    let cfg: DevConfig = serde_json::from_str(json).unwrap();
    assert!(cfg.preview_enabled);
    assert_eq!(cfg.preview_fixtures_dir, std::path::PathBuf::from("x/y"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p todo-app --lib config::tests::dev_preview -- --nocapture`
Expected: FAIL — `preview_enabled` and `preview_fixtures_dir` not found on `DevConfig`.

- [ ] **Step 3: Add the fields to `DevConfig`**

Replace the `DevConfig` struct in `crates/app/src/config.rs` (the `#[derive(... )] pub struct DevConfig` block) with:

```rust
/// Local-development conveniences. The features here are compiled out of
/// `--release` builds (see `cfg(debug_assertions)` gates in `router.rs` and
/// `main.rs`), so a config leak alone can't expose them in production.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevConfig {
    /// When non-empty (and the binary is a debug build), `POST /dev/login`
    /// drops the bearer into a session as this user. The account is created
    /// on startup if missing.
    #[serde(default)]
    pub auto_login_email: String,

    /// When true (and the binary is a debug build), mounts `/__preview/*`.
    #[serde(default)]
    pub preview_enabled: bool,

    /// Directory containing `<template>/<story>.toml` fixtures.
    #[serde(default = "default_preview_fixtures_dir")]
    pub preview_fixtures_dir: PathBuf,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            auto_login_email: String::new(),
            preview_enabled: false,
            preview_fixtures_dir: default_preview_fixtures_dir(),
        }
    }
}

fn default_preview_fixtures_dir() -> PathBuf {
    PathBuf::from("fixtures/templates")
}
```

(The previous `#[derive(... Default)]` is replaced by an explicit `Default` impl because the field default for `preview_fixtures_dir` is a non-empty PathBuf.)

Also extend the `from_env()` builder. Find the line that ends with `.set_default("dev.auto_login_email", defaults.dev.auto_login_email)?;` and append two more chained calls:

```rust
            .set_default("dev.auto_login_email", defaults.dev.auto_login_email)?
            .set_default("dev.preview_enabled", defaults.dev.preview_enabled)?
            .set_default(
                "dev.preview_fixtures_dir",
                defaults.dev.preview_fixtures_dir.to_string_lossy().to_string(),
            )?;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p todo-app --lib config::tests -- --nocapture`
Expected: PASS — all tests in the config module, including the two new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/config.rs
git commit -m "feat(config): add dev.preview_enabled and dev.preview_fixtures_dir"
```

---

## Task 3: Fixture loader (TOML → JSON)

**Files:**
- Create: `crates/app/src/preview/mod.rs`
- Create: `crates/app/src/preview/fixtures.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: Wire the module skeleton**

Create `crates/app/src/preview/mod.rs` with this exact content:

```rust
//! Dev-only template preview surface, mounted at `/__preview/*` when
//! `cfg(debug_assertions)` AND `state.config.dev.preview_enabled`. Entire
//! module is compiled out of `--release` builds via the `cfg` gate in
//! `lib.rs`.

pub mod fixtures;
```

Create `crates/app/src/preview/fixtures.rs` as an empty file (the test in Step 2 will fail to compile, which is the expected first-failure state).

Add this line to `crates/app/src/lib.rs` (top-level, near the other `pub mod` lines):

```rust
#[cfg(debug_assertions)]
pub mod preview;
```

- [ ] **Step 2: Write the failing tests**

Add this test block to the bottom of `crates/app/src/preview/fixtures.rs`:

```rust
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
        assert!(matches!(err, FixtureError::Io(_)));
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
        writeln!(tmp, r#"[meta]
description = "no ctx""#).unwrap();
        let loaded = load_fixture(tmp.path()).unwrap();
        assert!(loaded.ctx.is_null() || loaded.ctx.is_object(),
            "ctx should be JSON null or empty object; got {:?}", loaded.ctx);
    }
}
```

Also add `tempfile` to `crates/app/Cargo.toml` `[dev-dependencies]` if not already present (it is — confirm with `grep tempfile crates/app/Cargo.toml`).

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p todo-app --lib preview::fixtures -- --nocapture`
Expected: FAIL — `load_fixture`, `FixtureError`, etc. are not defined.

- [ ] **Step 4: Implement `load_fixture`**

Replace the contents of `crates/app/src/preview/fixtures.rs` with:

```rust
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
```

Note the helper struct: toml's deserializer can target any serde type, including `serde_json::Value` for the `ctx` field — that's how we go straight from TOML to a JSON value without an intermediate `toml::Value`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p todo-app --lib preview::fixtures -- --nocapture`
Expected: PASS — all 4 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/preview/mod.rs crates/app/src/preview/fixtures.rs crates/app/src/lib.rs
git commit -m "feat(preview): fixture loader (TOML -> serde_json)"
```

---

## Task 4: Fixture discovery walker

**Files:**
- Modify: `crates/app/src/preview/fixtures.rs`

- [ ] **Step 1: Write the failing test**

Append this test to the `tests` module in `crates/app/src/preview/fixtures.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p todo-app --lib preview::fixtures::tests::discover -- --nocapture`
Expected: FAIL — `discover`, `Story`, etc. not defined.

- [ ] **Step 3: Implement the walker**

Append the following to `crates/app/src/preview/fixtures.rs` (after the `load_fixture` function, before the test module):

```rust
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
pub fn discover(templates_dir: &Path, fixtures_dir: &Path) -> Result<Vec<TemplateEntry>, FixtureError> {
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p todo-app --lib preview::fixtures -- --nocapture`
Expected: PASS — all 5 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/preview/fixtures.rs
git commit -m "feat(preview): discovery walker for templates x fixtures"
```

---

## Task 5: Host shell template and helper

**Files:**
- Create: `templates/_preview_shell.html`
- Create: `crates/app/src/preview/shell.rs`
- Modify: `crates/app/src/preview/mod.rs`

This task has no unit test of its own — the shell helper is exercised end-to-end by the integration tests in Task 9.

- [ ] **Step 1: Create the host shell template**

Create `templates/_preview_shell.html` with exactly this content:

```html
<!doctype html>
<html lang="{{ _locale or 'en' }}">
<head>
  <meta charset="utf-8">
  <title>preview · {{ template_path }} · {{ story }}</title>
  <link rel="stylesheet" href="{{ asset('css/app.css') }}">
  <script src="{{ asset('vendor/alpine-3.15.12.min.js') }}" defer></script>
  {# Deliberately NO htmx. hx-* attributes are dead markup here. #}
  <style>
    .__preview_bar {
      position: fixed; top: 0; left: 0; right: 0;
      background: #222; color: #eee;
      padding: 6px 12px; font: 12px/1.4 ui-monospace, monospace;
      z-index: 9999;
      display: flex; gap: 12px; flex-wrap: wrap;
    }
    .__preview_bar a { color: #6cf; text-decoration: none; }
    .__preview_bar a:hover { text-decoration: underline; }
    body { padding-top: 32px; }
  </style>
</head>
<body class="antialiased">
  <div class="__preview_bar">
    <span>PREVIEW</span>
    <span>{{ template_path }} · {{ story }}</span>
    <span>locale={{ _locale }} tz={{ _tz }}</span>
    <span>
      {% for l in ["en", "es", "fr", "de"] -%}
        <a href="?locale={{ l }}&tz={{ _tz }}">[{{ l }}]</a>
      {%- endfor %}
    </span>
    <span><a href="/__preview">↩ index</a></span>
  </div>
  <main>
    {{ rendered_partial | safe }}
  </main>
</body>
</html>
```

- [ ] **Step 2: Create the shell helper**

Create `crates/app/src/preview/shell.rs`:

```rust
//! Renders the host shell for partial-template previews.

use minijinja::context;

use crate::{templates::Templates, AppError};

/// Wrap an already-rendered partial in `_preview_shell.html`.
/// Top-level templates that extend `base.html` skip this entirely.
pub fn wrap(
    templates: &Templates,
    rendered_partial: String,
    template_path: &str,
    story: &str,
    locale: &str,
    tz: &str,
) -> Result<String, AppError> {
    let html = templates.render(
        "_preview_shell.html",
        context! {
            rendered_partial,
            template_path,
            story,
            _locale => locale,
            _tz => tz,
        },
    )?;
    Ok(html.0)
}
```

- [ ] **Step 3: Wire the module**

Update `crates/app/src/preview/mod.rs` to expose `shell`:

```rust
//! Dev-only template preview surface, mounted at `/__preview/*` when
//! `cfg(debug_assertions)` AND `state.config.dev.preview_enabled`. Entire
//! module is compiled out of `--release` builds via the `cfg` gate in
//! `lib.rs`.

pub mod fixtures;
pub mod shell;
```

- [ ] **Step 4: Verify the workspace still builds**

Run: `cargo build --workspace`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add templates/_preview_shell.html crates/app/src/preview/shell.rs crates/app/src/preview/mod.rs
git commit -m "feat(preview): host shell template + wrap() helper"
```

---

## Task 6: Render route — `GET /__preview/render/*path`

**Files:**
- Create: `crates/app/src/preview/routes.rs`
- Modify: `crates/app/src/preview/mod.rs`

This task's behavior is verified end-to-end in Task 9 (integration tests). No unit test here.

- [ ] **Step 1: Create the routes file**

Create `crates/app/src/preview/routes.rs` with this content:

```rust
//! Preview HTTP handlers. Mounted by `preview::router()` and gated by
//! `state.config.dev.preview_enabled` inside each handler — a defense-in-depth
//! check beyond the `cfg(debug_assertions)` mount gate in `router.rs`.

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use minijinja::context;
use serde::Deserialize;

use crate::{
    middleware::CspNonce,
    preview::{
        fixtures::{discover, load_fixture, TemplateEntry},
        shell,
    },
    AppError, AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/render/*path", get(render))
}

#[derive(Debug, Deserialize, Default)]
pub struct PreviewQuery {
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub tz: Option<String>,
}

impl PreviewQuery {
    fn locale_or_default(&self) -> &str {
        self.locale.as_deref().unwrap_or("en")
    }
    fn tz_or_default(&self) -> &str {
        self.tz.as_deref().unwrap_or("UTC")
    }
}

fn check_enabled(state: &AppState) -> Result<(), AppError> {
    if state.config.dev.preview_enabled {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

/// `GET /__preview` — list every discovered template and its stories.
pub async fn index(
    State(state): State<AppState>,
    Query(q): Query<PreviewQuery>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    check_enabled(&state)?;
    let entries: Vec<TemplateEntry> = discover(
        &state.config.templates_dir,
        &state.config.dev.preview_fixtures_dir,
    )
    .map_err(|e| AppError::Internal(format!("preview discover failed: {e}")))?;

    let html = state.templates.render(
        "_preview_index.html",
        context! {
            entries => entries.iter().map(|e| context! {
                template_path => &e.template_path,
                stories => e.stories.iter().map(|s| &s.name).collect::<Vec<_>>(),
            }).collect::<Vec<_>>(),
            _locale => q.locale_or_default(),
            _tz => q.tz_or_default(),
            csp_nonce => nonce.0.clone(),
        },
    )?;
    Ok(html.into_response())
}

/// `GET /__preview/render/*path` — render a single (template, story) pair.
/// `path` is an axum catchall; the last segment is the story name, the rest
/// is the template path (e.g. `partials/todo.html/default`).
pub async fn render(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(q): Query<PreviewQuery>,
    Extension(nonce): Extension<CspNonce>,
) -> Result<Response, AppError> {
    check_enabled(&state)?;

    let (template_path, story) = split_last_segment(&path).ok_or(AppError::NotFound)?;

    // Spec: unknown template path → 404 (not 500 from minijinja). Check
    // existence on disk before calling render(); a typo'd template name
    // would otherwise bubble up as AppError::Template -> 500.
    if !state.config.templates_dir.join(template_path).is_file() {
        return Err(AppError::NotFound);
    }

    let fixture_file = state
        .config
        .dev
        .preview_fixtures_dir
        .join(template_path.trim_end_matches(".html"))
        .join(format!("{story}.toml"));
    if !fixture_file.exists() {
        return Err(AppError::NotFound);
    }
    let loaded = load_fixture(&fixture_file)
        .map_err(|e| AppError::Internal(format!("fixture load failed: {e}")))?;

    let locale = q.locale_or_default();
    let tz = q.tz_or_default();

    // Build the render context: ambient defaults, then merge fixture ctx on top.
    // Fixture wins on key conflict (explicit override always works).
    let mut merged = serde_json::Map::new();
    merged.insert("_locale".into(), serde_json::Value::String(locale.to_string()));
    merged.insert("_tz".into(), serde_json::Value::String(tz.to_string()));
    merged.insert("csp_nonce".into(), serde_json::Value::String(nonce.0.clone()));
    if let serde_json::Value::Object(ctx) = loaded.ctx {
        for (k, v) in ctx {
            merged.insert(k, v);
        }
    }
    let merged = serde_json::Value::Object(merged);

    let rendered = state.templates.render(template_path, &merged)?;

    // Partials get wrapped in the host shell; everything else (templates that
    // extend base.html) is already a full document.
    let body = if template_path.starts_with("partials/") {
        Html(shell::wrap(
            &state.templates,
            rendered.0,
            template_path,
            story,
            locale,
            tz,
        )?)
    } else {
        rendered
    };

    Ok(body.into_response())
}

/// Split `"a/b/c.html/story"` into `("a/b/c.html", "story")`.
/// Returns `None` if there is no `/`.
fn split_last_segment(path: &str) -> Option<(&str, &str)> {
    let idx = path.rfind('/')?;
    Some((&path[..idx], &path[idx + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_last_segment_basic() {
        assert_eq!(
            split_last_segment("partials/todo.html/default"),
            Some(("partials/todo.html", "default"))
        );
        assert_eq!(split_last_segment("index.html/default"), Some(("index.html", "default")));
        assert_eq!(split_last_segment("no-slash"), None);
    }
}
```

- [ ] **Step 2: Update the module to export routes**

Update `crates/app/src/preview/mod.rs`:

```rust
//! Dev-only template preview surface, mounted at `/__preview/*` when
//! `cfg(debug_assertions)` AND `state.config.dev.preview_enabled`. Entire
//! module is compiled out of `--release` builds via the `cfg` gate in
//! `lib.rs`.

pub mod fixtures;
pub mod routes;
pub mod shell;

pub use routes::router;
```

- [ ] **Step 3: Verify it compiles and the small unit test passes**

Run: `cargo build --workspace && cargo test -p todo-app --lib preview::routes -- --nocapture`
Expected: PASS — `split_last_segment_basic` passes; the routes module compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/preview/routes.rs crates/app/src/preview/mod.rs
git commit -m "feat(preview): index + render handlers"
```

---

## Task 7: Index template

**Files:**
- Create: `templates/_preview_index.html`

- [ ] **Step 1: Create the index template**

Create `templates/_preview_index.html`:

```html
<!doctype html>
<html lang="{{ _locale or 'en' }}">
<head>
  <meta charset="utf-8">
  <title>preview · index</title>
  <link rel="stylesheet" href="{{ asset('css/app.css') }}">
  <style>
    body { font: 14px/1.5 ui-sans-serif, system-ui; padding: 24px; max-width: 720px; margin: 0 auto; }
    h1 { font-size: 18px; margin-bottom: 16px; }
    h2 { font-size: 14px; margin-top: 24px; color: #555; font-weight: 500; }
    ul { list-style: none; padding-left: 16px; }
    li { padding: 2px 0; }
    .empty { color: #999; font-style: italic; }
    .switcher { background: #f5f5f0; padding: 8px 12px; border-radius: 4px; margin-bottom: 16px; font-family: ui-monospace, monospace; font-size: 12px; }
    .switcher a { margin-right: 8px; }
  </style>
</head>
<body>
  <h1>Template preview</h1>

  <div class="switcher">
    locale:
    {% for l in ["en", "es", "fr", "de"] %}
      <a href="?locale={{ l }}&tz={{ _tz }}">[{{ l }}]</a>
    {% endfor %}
    &nbsp; tz:
    {% for z in ["UTC", "America/New_York", "Europe/Paris", "Asia/Tokyo"] %}
      <a href="?locale={{ _locale }}&tz={{ z }}">[{{ z }}]</a>
    {% endfor %}
  </div>

  <ul>
    {% for entry in entries %}
      <li>
        <strong>{{ entry.template_path }}</strong>
        {% if entry.stories %}
          <ul>
            {% for story in entry.stories %}
              <li>
                <a href="/__preview/render/{{ entry.template_path }}/{{ story }}?locale={{ _locale }}&tz={{ _tz }}">
                  {{ story }}
                </a>
              </li>
            {% endfor %}
          </ul>
        {% else %}
          <ul><li class="empty">[no fixtures — add at fixtures/templates/{{ entry.template_path | replace(".html", "") }}/default.toml]</li></ul>
        {% endif %}
      </li>
    {% endfor %}
  </ul>
</body>
</html>
```

- [ ] **Step 2: Verify the workspace still builds (template is loaded at runtime, but a syntax error wouldn't be caught here — Task 9 will exercise it).**

Run: `cargo build --workspace`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add templates/_preview_index.html
git commit -m "feat(preview): index page template"
```

---

## Task 8: Mount preview routes in the main router

**Files:**
- Modify: `crates/app/src/router.rs`

- [ ] **Step 1: Add the mount**

Open `crates/app/src/router.rs`. Find the existing `#[cfg(debug_assertions)]` block that adds the `/dev/login` route:

```rust
    #[cfg(debug_assertions)]
    let api = api.route("/dev/login", post(crate::routes::dev::auto_login));
```

Immediately AFTER that line (still inside the file's existing structure — not inside any nested block), add:

```rust
    // Preview tool. Runtime-gated by `dev.preview_enabled` inside the handlers
    // so an accidental config flip alone doesn't expose it; the whole module
    // is also `#[cfg(debug_assertions)]`-gated so it's absent in --release.
    #[cfg(debug_assertions)]
    let api = api.nest("/__preview", crate::preview::router());
```

(Two separate `let api = ...` rebindings under `#[cfg(debug_assertions)]` so each is independently gated. This works because shadowing one inside a cfg block keeps the outer binding intact when the cfg is off.)

- [ ] **Step 2: Verify the workspace still builds**

Run: `cargo build --workspace`
Expected: PASS.

- [ ] **Step 3: Verify clippy is clean**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS — no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/router.rs
git commit -m "feat(preview): mount /__preview behind cfg(debug_assertions) + dev.preview_enabled"
```

---

## Task 9: Seed v1 fixtures

**Files:**
- Create: all 12 TOML files listed below

- [ ] **Step 1: Create the index fixtures**

Create `fixtures/templates/index/default.toml`:

```toml
[meta]
description = "Authenticated user with three todos in mixed states"

[ctx]
user = { id = "00000000-0000-0000-0000-000000000001", email = "alice@example.com", password_hash = "x", created_at = "2026-05-01T09:00:00Z", locale = "en", timezone = "UTC" }
todos = [
  { id = "11111111-1111-1111-1111-111111111111", title = "Buy milk", completed = false, created_at = "2026-05-19T10:00:00Z", updated_at = "2026-05-19T10:00:00Z" },
  { id = "22222222-2222-2222-2222-222222222222", title = "Email Maria", completed = true, created_at = "2026-05-18T14:00:00Z", updated_at = "2026-05-20T11:30:00Z" },
  { id = "33333333-3333-3333-3333-333333333333", title = "Read the Tempo 3 release notes", completed = false, created_at = "2026-05-20T08:15:00Z", updated_at = "2026-05-20T08:15:00Z" },
]
```

Create `fixtures/templates/index/empty.toml`:

```toml
[meta]
description = "Authenticated user with no todos — empty state"

[ctx]
user = { id = "00000000-0000-0000-0000-000000000001", email = "alice@example.com", password_hash = "x", created_at = "2026-05-01T09:00:00Z", locale = "en", timezone = "UTC" }
todos = []
```

Create `fixtures/templates/index/many-items.toml`:

```toml
[meta]
description = "Long list — stress-test list spacing and the empty state hide"

[ctx]
user = { id = "00000000-0000-0000-0000-000000000001", email = "alice@example.com", password_hash = "x", created_at = "2026-05-01T09:00:00Z", locale = "en", timezone = "UTC" }
todos = [
  { id = "aaaaaaaa-0000-0000-0000-000000000001", title = "Item one",   completed = false, created_at = "2026-05-20T10:00:00Z", updated_at = "2026-05-20T10:00:00Z" },
  { id = "aaaaaaaa-0000-0000-0000-000000000002", title = "Item two",   completed = false, created_at = "2026-05-20T10:01:00Z", updated_at = "2026-05-20T10:01:00Z" },
  { id = "aaaaaaaa-0000-0000-0000-000000000003", title = "Item three", completed = true,  created_at = "2026-05-20T10:02:00Z", updated_at = "2026-05-20T10:02:00Z" },
  { id = "aaaaaaaa-0000-0000-0000-000000000004", title = "Item four",  completed = false, created_at = "2026-05-20T10:03:00Z", updated_at = "2026-05-20T10:03:00Z" },
  { id = "aaaaaaaa-0000-0000-0000-000000000005", title = "Item five",  completed = true,  created_at = "2026-05-20T10:04:00Z", updated_at = "2026-05-20T10:04:00Z" },
  { id = "aaaaaaaa-0000-0000-0000-000000000006", title = "Item six",   completed = false, created_at = "2026-05-20T10:05:00Z", updated_at = "2026-05-20T10:05:00Z" },
  { id = "aaaaaaaa-0000-0000-0000-000000000007", title = "Item seven", completed = false, created_at = "2026-05-20T10:06:00Z", updated_at = "2026-05-20T10:06:00Z" },
  { id = "aaaaaaaa-0000-0000-0000-000000000008", title = "Item eight", completed = true,  created_at = "2026-05-20T10:07:00Z", updated_at = "2026-05-20T10:07:00Z" },
]
```

- [ ] **Step 2: Create the login fixtures**

Create `fixtures/templates/login/default.toml`:

```toml
[meta]
description = "Fresh login form, no errors"

[ctx]
next = ""
error = ""
dev_login_enabled = false
```

Create `fixtures/templates/login/with-validation-error.toml`:

```toml
[meta]
description = "Login form with an error message displayed"

[ctx]
next = ""
error = "Invalid email or password."
dev_login_enabled = false
```

- [ ] **Step 3: Create the signup fixtures**

Create `fixtures/templates/signup/default.toml`:

```toml
[meta]
description = "Fresh signup form, no errors"

[ctx]
error = ""
```

Create `fixtures/templates/signup/with-validation-error.toml`:

```toml
[meta]
description = "Signup form with a validation error displayed"

[ctx]
error = "Email is already taken."
```

- [ ] **Step 4: Create the todo partial fixtures**

Create `fixtures/templates/partials/todo/default.toml`:

```toml
[meta]
description = "A pending todo with a recent timestamp"

[ctx]
todo = { id = "11111111-1111-1111-1111-111111111111", title = "Buy milk", completed = false, created_at = "2026-05-19T10:00:00Z", updated_at = "2026-05-19T10:00:00Z" }
```

Create `fixtures/templates/partials/todo/completed.toml`:

```toml
[meta]
description = "A completed todo (rendered in done style with updated_at)"

[ctx]
todo = { id = "22222222-2222-2222-2222-222222222222", title = "Email Maria", completed = true, created_at = "2026-05-18T14:00:00Z", updated_at = "2026-05-20T11:30:00Z" }
```

Create `fixtures/templates/partials/todo/long-title.toml`:

```toml
[meta]
description = "Stress the title wrap with a 200-char title"

[ctx]
todo = { id = "33333333-3333-3333-3333-333333333333", title = "A long-running line of intentions that goes well past what any reasonable to-do entry has any business carrying, just to see how the layout copes with the overflow situation", completed = false, created_at = "2026-05-20T08:15:00Z", updated_at = "2026-05-20T08:15:00Z" }
```

- [ ] **Step 5: Create the todo_list partial fixtures**

Create `fixtures/templates/partials/todo_list/default.toml`:

```toml
[meta]
description = "List with three todos"

[ctx]
todos = [
  { id = "11111111-1111-1111-1111-111111111111", title = "Buy milk", completed = false, created_at = "2026-05-19T10:00:00Z", updated_at = "2026-05-19T10:00:00Z" },
  { id = "22222222-2222-2222-2222-222222222222", title = "Email Maria", completed = true, created_at = "2026-05-18T14:00:00Z", updated_at = "2026-05-20T11:30:00Z" },
  { id = "33333333-3333-3333-3333-333333333333", title = "Read the Tempo 3 release notes", completed = false, created_at = "2026-05-20T08:15:00Z", updated_at = "2026-05-20T08:15:00Z" },
]
```

Create `fixtures/templates/partials/todo_list/empty.toml`:

```toml
[meta]
description = "Empty list — renders <ul> with no <li> children"

[ctx]
todos = []
```

- [ ] **Step 6: Commit**

```bash
git add fixtures/
git commit -m "feat(preview): seed fixtures for v1 templates"
```

---

## Task 10: Integration tests

**Files:**
- Create: `crates/app/tests/preview_flow.rs`

- [ ] **Step 1: Add `fixtures` path injection to the test harness**

Before adding the test file, extend `crates/app/tests/common/mod.rs` so each test's `spawn_with` closure can point the preview at the repo's `fixtures/templates` (the same way it already does for `templates_dir`).

Open `crates/app/tests/common/mod.rs`. Find the block that sets `cfg.templates_dir` and `cfg.static_dir`. Right after `cfg.static_dir = ...;` (and before `cfg.auth.session_key = ...;`), add:

```rust
    cfg.dev.preview_fixtures_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("fixtures/templates");
```

- [ ] **Step 2: Write the failing tests**

Create `crates/app/tests/preview_flow.rs`:

```rust
//! Integration tests for the dev preview tool. Routes are gated on
//! `cfg(debug_assertions)`; `cargo test` defaults to the debug profile so
//! the gate is always satisfied here.

#![cfg(debug_assertions)]

mod common;

use common::{spawn, spawn_with};

#[tokio::test]
async fn preview_index_lists_templates() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!("{}/__preview", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "expected 200, got {}", res.status());
    let body = res.text().await.unwrap();
    assert!(body.contains("partials/todo.html"), "body missing template path. body:\n{body}");
    assert!(body.contains("index.html"), "body missing index.html");
    assert!(
        !body.contains("_preview_shell.html"),
        "underscore-prefixed templates must be hidden from the index"
    );
}

#[tokio::test]
async fn preview_render_partial_shows_fixture_data() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!(
            "{}/__preview/render/partials/todo.html/default",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "expected 200, got {}", res.status());
    let body = res.text().await.unwrap();
    // The default fixture has title "Buy milk".
    assert!(body.contains("Buy milk"), "body missing fixture title. body:\n{body}");
    // Host shell should be present for partials.
    assert!(body.contains("PREVIEW"), "host shell PREVIEW bar missing. body:\n{body}");
}

#[tokio::test]
async fn preview_render_respects_locale_query_param() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!(
            "{}/__preview/render/partials/todo.html/default?locale=es",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    // partials/todo.html uses `t("todo-mark-done")` or `t("todo-mark-open")`.
    // The Spanish catalog at locales/es/main.ftl has whichever of those keys.
    // We assert SOME Spanish-only marker shows up — `lang="es"` is set by the
    // host shell from _locale, which is a stable, low-flake assertion.
    assert!(
        body.contains(r#"lang="es""#),
        "expected lang=\"es\" in shell. body:\n{body}"
    );
}

#[tokio::test]
async fn preview_render_full_page_skips_shell() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!(
            "{}/__preview/render/login.html/default",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    // login.html extends base.html, so the host shell's "PREVIEW" banner must
    // NOT be present. We expect the real base.html `<title>` containing the
    // localized page title instead.
    assert!(!body.contains("__preview_bar"),
        "host shell must be skipped for full-page templates");
    // And the page should be a complete document (base.html applied).
    assert!(body.contains("<title>"), "missing base.html <title>");
}

#[tokio::test]
async fn preview_render_missing_fixture_is_404() {
    let app = spawn_with(|cfg| cfg.dev.preview_enabled = true).await;
    let res = app
        .client
        .get(format!(
            "{}/__preview/render/partials/todo.html/does-not-exist",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn preview_disabled_returns_404() {
    let app = spawn().await; // default config — preview_enabled = false
    let res = app
        .client
        .get(format!("{}/__preview", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);

    let res = app
        .client
        .get(format!(
            "{}/__preview/render/partials/todo.html/default",
            app.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}
```

- [ ] **Step 3: Run the tests (Docker must be running)**

Run: `cargo test -p todo-app --test preview_flow -- --nocapture`
Expected: PASS — all 6 integration tests.

If any test fails, read the body printed by the assertion. The most common breakages:
- A fixture file is missing or misnamed → fix in Task 8.
- The render handler builds the path wrong → re-read Task 6 step 1.
- The shell template references an undefined variable → re-read Task 5.

- [ ] **Step 4: Run the full test suite to make sure nothing regressed**

Run: `cargo test --workspace`
Expected: PASS — all tests, including the existing 21 + 6 new integration tests.

- [ ] **Step 5: Run clippy as the workspace gate**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app/tests/preview_flow.rs crates/app/tests/common/mod.rs
git commit -m "test(preview): end-to-end integration tests"
```

---

## Task 11: Enable preview in dev compose

**Files:**
- Modify: `docker/compose.dev.yaml`

- [ ] **Step 1: Find the `app:` service's environment block**

Open `docker/compose.dev.yaml`. Locate the `services:` → `app:` → `environment:` block (it already contains entries like `APP__DEV__AUTO_LOGIN_EMAIL`). Add:

```yaml
      APP__DEV__PREVIEW: "true"
```

Keep alphabetical / grouped ordering consistent with the surrounding entries.

- [ ] **Step 2: Bring the dev stack up and smoke-check (manual)**

Run: `just up`
Then in a browser, open `http://localhost:3000/__preview` — expect to see the index listing all templates with their stories. Click `partials/todo.html → default` and confirm the preview bar appears with locale links.

If you don't want to spin up the full stack, this step can be skipped — Task 10 covered the behavior end-to-end. Bringing the stack up just gives a visual sanity check.

- [ ] **Step 3: Commit**

```bash
git add docker/compose.dev.yaml
git commit -m "chore(compose): enable APP__DEV__PREVIEW in dev"
```

---

## Task 12: Docs

**Files:**
- Modify: `CLAUDE.md`
- Modify: `README.md`

- [ ] **Step 1: Update CLAUDE.md**

Open `CLAUDE.md`. Find the section `## Where to add things`. After the existing list, add a new bullet:

```markdown
- **A new fixture for the template preview tool** → drop a TOML file at `fixtures/templates/<template-path-without-ext>/<story>.toml` with a `[ctx]` table holding the render context. Visit `/__preview` in a dev build (with `APP__DEV__PREVIEW=true`) to see it. Spec: `docs/superpowers/specs/2026-05-20-template-preview-design.md`.
```

- [ ] **Step 2: Update README.md**

Open `README.md`. Find a "Developer tools" / "Local dev" type section (or add one near the top of the dev instructions if none exists). Add:

```markdown
### Template preview

Dev-only: with the dev compose stack up, browse to `http://localhost:3000/__preview` to render any template in `templates/` against hand-edited TOML fixtures in `fixtures/templates/`. Off in production builds. Spec: [docs/superpowers/specs/2026-05-20-template-preview-design.md](docs/superpowers/specs/2026-05-20-template-preview-design.md).
```

If there's no obvious place for it, add it as a new top-level `## Developer tools` section just before the "License" footer (or the equivalent closing section).

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md README.md
git commit -m "docs: mention template preview tool"
```

---

## Final verification

After Task 12:

- [ ] **Step 1: Workspace build + lint + tests, one last time**

Run: `cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS on all three.

- [ ] **Step 2: Confirm the binary's release build still has zero preview code**

Run: `cargo build --release --bin todo-app && strings target/release/todo-app | grep -c "__preview"`
Expected: `0` — release binary contains no references to the preview path or module symbols. (If non-zero, the cfg gating is wrong; revisit Task 8.)

- [ ] **Step 3: Branch is ready for PR**

Run: `git log --oneline main..HEAD`
Expected: 12 commits, one per task, in order.

---

## Deliberate deviations from the spec

The spec calls for two enhancements that this plan omits as YAGNI for v1:

1. **Discovery caching.** Spec §"Discovery" says the walk should be cached for the process lifetime when `Templates::Static` and re-walked per request when `Templates::Reloading`. Implementation always re-walks, because the templates dir holds ~7 files and the walk costs <100µs. If preview latency ever becomes a complaint, add a `OnceCell<Vec<TemplateEntry>>` to `AppState` (or a lazy static) keyed on whichever mode is active.

2. **404 bodies that enumerate available templates / stories.** Spec §"Error handling" says "404, body lists available templates / stories." Implementation returns `AppError::NotFound` with the standard plain `"not found"` body for consistency with the rest of the app. The index page at `/__preview` lists everything anyway. If the omission stings, swap `AppError::NotFound` for a custom response that renders a small not-found shell with the list inlined.

Both are reversible additions later; neither blocks v1.

## Notes for the implementer

- **Pre-commit hooks**: this repo uses standard cargo formatters/linters at commit time. If a commit fails because `cargo fmt` rewrote a file you just added, re-stage the file and retry the commit — don't `--amend`.
- **Docker**: integration tests in Task 10 need Docker running. If `cargo test --test preview_flow` fails with `start postgres` errors, check `docker ps` works first.
- **Template autoreload**: the test harness uses `Templates::production` (autoreload off). If you ever spot-check changes by running `just up`, the dev stack uses `Templates::Reloading` and the new templates (`_preview_*.html`) reload on the next request without a restart.
- **Path encoding**: axum's `*path` decodes percent-encoding before handing it to the handler. A fixture name with a `/` in it would split into a sub-segment by `split_last_segment` — don't name fixtures with `/`.
- **Edge case to know about**: minijinja's `path_loader` rejects template names containing `..`. Don't try to write a fixture name like `../escape`; the `state.templates.render(template_path, ...)` call will return an error that bubbles up as `AppError::Template` (500). That's the right behavior — no fix needed, just don't be surprised.
