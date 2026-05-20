# Extract assets crate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the content-hashed static asset manifest and the `asset()` minijinja helper out of `crates/i18n/` into a new `crates/assets/` (package `todo-assets`).

**Architecture:** New crate exposes `todo_assets::Assets` and `todo_assets::minijinja::register`. `crates/i18n/` keeps only locale negotiation, Fluent messages, ICU datetime formatting, and the `t()` + `datetime()` minijinja helpers. `crates/app/` depends on both and calls each crate's `minijinja::register` function at startup. No runtime behavior changes — the rendered HTML still references `/static/css/app.<8-hex-hash>.css` with `Cache-Control: public, max-age=31536000, immutable`.

**Tech Stack:** Cargo workspace, Rust 2021 edition. New crate uses `sha2`, `minijinja`, `thiserror`; dev-dep `tempfile`. i18n drops `sha2` + `base64` (the latter is already unused).

**Spec:** `docs/superpowers/specs/2026-05-20-extract-assets-crate-design.md`

**Branch:** `refactor/extract-assets-crate`

**Safe ordering:** every commit must build. Tasks 1–6 progress incrementally: scaffold → add Assets → migrate Assets consumers → migrate the helper → docs → final verification.

---

## File Structure

**New:**
- `crates/assets/Cargo.toml`
- `crates/assets/src/lib.rs`
- `crates/assets/src/manifest.rs`  (Task 2)
- `crates/assets/src/minijinja.rs` (Task 4)

**Modified:**
- `Cargo.toml` (workspace root) — Task 1
- `crates/i18n/Cargo.toml` — Tasks 3 + 4
- `crates/i18n/src/lib.rs` — Tasks 3 + 4
- `crates/i18n/src/minijinja_helpers.rs` → renamed to `crates/i18n/src/minijinja.rs` (Task 4)
- `crates/app/Cargo.toml` — Task 3
- `crates/app/src/main.rs` — Tasks 3 + 4
- `crates/app/src/state.rs` — Task 3
- `crates/app/src/templates.rs` — Task 4
- `CLAUDE.md` — Task 5

**Deleted:**
- `crates/i18n/src/assets.rs` (Task 3)

---

## Task 1: Scaffold the new crate

**Files:**
- Create: `crates/assets/Cargo.toml`
- Create: `crates/assets/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

After this task, `cargo build --workspace` succeeds with an empty `todo-assets` crate that nothing imports yet.

- [ ] **Step 1: Create `crates/assets/Cargo.toml`**

```toml
[package]
name         = "todo-assets"
version      = "0.1.0"
edition      = { workspace = true }
rust-version = { workspace = true }
license      = { workspace = true }
publish      = { workspace = true }

[lints]

[dependencies]
sha2      = { workspace = true }
minijinja = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

The deps mirror what `crates/i18n/src/assets.rs` currently uses (`sha2`, `thiserror`) plus `minijinja` (for the helper, which lands in Task 4). `tempfile` is for the existing tests.

- [ ] **Step 2: Create `crates/assets/src/lib.rs`**

```rust
//! Content-hashed static asset manifest and the `asset()` minijinja helper.
//! Used by the HTTP layer (`todo-app`) to render long-cacheable URLs for
//! static files and to serve those hashed URLs.
```

(File contains only the crate-level doc comment for now. Module declarations get added in Tasks 2 and 4.)

- [ ] **Step 3: Add `crates/assets` to the workspace**

Edit `/Users/nick/projects/todo-rust/Cargo.toml`. Find:

```toml
[workspace]
resolver = "2"
members = [
    "crates/app",
    "crates/domain",
    "crates/i18n",
    "crates/observability",
    "crates/storage",
]
```

Change to:

```toml
[workspace]
resolver = "2"
members = [
    "crates/app",
    "crates/assets",
    "crates/domain",
    "crates/i18n",
    "crates/observability",
    "crates/storage",
]
```

Then find this block (near the bottom):

```toml
# Internal crates
todo-domain        = { path = "crates/domain" }
todo-storage       = { path = "crates/storage" }
todo-observability = { path = "crates/observability" }
todo-i18n          = { path = "crates/i18n" }
```

Change to:

```toml
# Internal crates
todo-domain        = { path = "crates/domain" }
todo-storage       = { path = "crates/storage" }
todo-observability = { path = "crates/observability" }
todo-i18n          = { path = "crates/i18n" }
todo-assets        = { path = "crates/assets" }
```

- [ ] **Step 4: Verify the workspace builds**

Run:
```bash
cargo build --workspace
```

Expected: all crates compile, including the new empty `todo-assets`. `Compiling todo-assets v0.1.0` appears in the output.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/assets
git commit -m "$(cat <<'EOF'
chore(workspace): scaffold empty todo-assets crate

Adds crates/assets/ as a new workspace member. The crate is empty and
unused at this point — content lands in subsequent commits.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add the `Assets` manifest to the new crate

**Files:**
- Create: `crates/assets/src/manifest.rs`
- Modify: `crates/assets/src/lib.rs`

After this task, `crates/assets/` has a fully functional `Assets` type with the 5 unit tests it had in i18n. Nothing imports it yet — `crates/i18n/src/assets.rs` still exists. The compile time existence of two `Assets` types (one in `todo_i18n::Assets`, one in `todo_assets::Assets`) is fine because nothing uses `todo_assets::Assets` yet.

- [ ] **Step 1: Create `crates/assets/src/manifest.rs`**

Copy the contents of `crates/i18n/src/assets.rs` verbatim, with one edit: change the module-level doc comment (the first two lines) to be slightly more focused on the new crate context. Final contents:

```rust
//! Content-hashed static asset manifest. In production, walks a directory
//! at startup and computes sha256(file)[..8] for each file. The
//! `resolve` function returns the hashed URL for a logical path; the
//! `resolve_hashed_request` function recognizes the hashed pattern when serving.
//!
//! In dev, `Assets::dev()` returns a no-op manifest so `resolve` returns
//! the raw logical path. Tailwind --watch can edit `app.css` freely.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

#[derive(Clone)]
pub enum Assets {
    /// Production: walk the static dir, compute hashes, return hashed URLs.
    Hashed {
        root: PathBuf,
        manifest: HashMap<String, String>, // "css/app.css" -> "css/app.<hash>.css"
    },
    /// Dev: identity mapping, raw paths.
    Passthrough { root: PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub enum AssetsError {
    #[error("read static dir: {0}")]
    Read(#[from] std::io::Error),
}

impl Assets {
    pub fn production(root: PathBuf) -> Result<Self, AssetsError> {
        let mut manifest = HashMap::new();
        walk_files(&root, &root, &mut manifest)?;
        Ok(Self::Hashed { root, manifest })
    }

    pub fn dev(root: PathBuf) -> Self {
        Self::Passthrough { root }
    }

    pub fn root(&self) -> &Path {
        match self {
            Self::Hashed { root, .. } => root,
            Self::Passthrough { root } => root,
        }
    }

    /// Resolve a logical path (e.g. "css/app.css") to a URL path (without
    /// the leading "/static/" prefix — callers prepend that).
    pub fn resolve(&self, logical: &str) -> String {
        match self {
            Self::Hashed { manifest, .. } => manifest
                .get(logical)
                .cloned()
                .unwrap_or_else(|| logical.to_owned()),
            Self::Passthrough { .. } => logical.to_owned(),
        }
    }

    /// If `url_path` matches `<dir>/<name>.<8hex>.<ext>` AND the manifest
    /// has the same hash for the unhashed path, return the on-disk path
    /// of the unhashed file. Otherwise return None.
    pub fn resolve_hashed_request(&self, url_path: &str) -> Option<PathBuf> {
        let Self::Hashed { root, manifest } = self else {
            return None;
        };
        // Find the unhashed entry whose hashed value equals url_path.
        let unhashed = manifest
            .iter()
            .find_map(|(unhashed, hashed)| (hashed == url_path).then(|| unhashed.clone()))?;
        Some(root.join(&unhashed))
    }
}

fn walk_files(
    root: &Path,
    dir: &Path,
    manifest: &mut HashMap<String, String>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_files(root, &path, manifest)?;
            continue;
        }
        // Skip precompressed siblings; they share the unhashed path's hash.
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.ends_with(".gz") || name.ends_with(".br") {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap();
        let logical = rel.to_string_lossy().replace('\\', "/");
        let hash = hash_file(&path)?;
        let hashed = inject_hash(&logical, &hash);
        manifest.insert(logical, hashed);
    }
    Ok(())
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    // First 8 hex chars is plenty for cache busting; collision risk is
    // negligible at the file counts we care about.
    Ok(hex_short(&digest[..]))
}

fn hex_short(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(8);
    for b in &bytes[..4] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// "css/app.css" + "abcd1234" -> "css/app.abcd1234.css"
fn inject_hash(logical: &str, hash: &str) -> String {
    if let Some(dot) = logical.rfind('.') {
        let (stem, ext) = logical.split_at(dot);
        format!("{stem}.{hash}{ext}")
    } else {
        format!("{logical}.{hash}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, body: &[u8]) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }

    #[test]
    fn dev_passthrough_returns_logical_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = Assets::dev(dir.path().to_path_buf());
        assert_eq!(a.resolve("css/app.css"), "css/app.css");
    }

    #[test]
    fn production_injects_hash_before_extension() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("css/app.css"), b"body { color: red; }");
        let a = Assets::production(dir.path().to_path_buf()).unwrap();
        let resolved = a.resolve("css/app.css");
        assert!(resolved.starts_with("css/app."));
        assert!(resolved.ends_with(".css"));
        // The injected hash has 8 hex chars + 1 dot before the extension.
        let middle = &resolved["css/app.".len()..resolved.len() - ".css".len()];
        assert_eq!(middle.len(), 8);
        assert!(middle.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn resolve_hashed_request_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("css/app.css"), b"hello");
        let a = Assets::production(dir.path().to_path_buf()).unwrap();
        let resolved = a.resolve("css/app.css");
        let on_disk = a.resolve_hashed_request(&resolved).unwrap();
        assert!(on_disk.ends_with("css/app.css"));
    }

    #[test]
    fn unknown_logical_path_returns_itself() {
        let dir = tempfile::tempdir().unwrap();
        let a = Assets::production(dir.path().to_path_buf()).unwrap();
        assert_eq!(a.resolve("not/in/manifest.png"), "not/in/manifest.png");
    }

    #[test]
    fn precompressed_siblings_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("js/app.js"), b"console.log('hi');");
        write(&dir.path().join("js/app.js.gz"), b"compressed");
        write(&dir.path().join("js/app.js.br"), b"compressed");
        let a = Assets::production(dir.path().to_path_buf()).unwrap();
        let Assets::Hashed { manifest, .. } = &a else {
            panic!()
        };
        assert!(manifest.contains_key("js/app.js"));
        assert!(!manifest.contains_key("js/app.js.gz"));
        assert!(!manifest.contains_key("js/app.js.br"));
    }
}
```

- [ ] **Step 2: Update `crates/assets/src/lib.rs`**

Change the file from the doc-only stub to:

```rust
//! Content-hashed static asset manifest and the `asset()` minijinja helper.
//! Used by the HTTP layer (`todo-app`) to render long-cacheable URLs for
//! static files and to serve those hashed URLs.

pub mod manifest;

pub use manifest::{Assets, AssetsError};
```

The `minijinja` module is added in Task 4.

- [ ] **Step 3: Verify the new crate builds and its tests pass**

Run:
```bash
cargo test -p todo-assets --lib
```

Expected: 5 tests pass.

```
test manifest::tests::dev_passthrough_returns_logical_path ... ok
test manifest::tests::production_injects_hash_before_extension ... ok
test manifest::tests::resolve_hashed_request_round_trips ... ok
test manifest::tests::unknown_logical_path_returns_itself ... ok
test manifest::tests::precompressed_siblings_are_skipped ... ok

test result: ok. 5 passed; 0 failed
```

- [ ] **Step 4: Confirm the workspace still builds end to end**

```bash
cargo build --workspace
```

Expected: succeeds. `todo-i18n` still has its own `Assets` type (unchanged); `todo-assets` is the new home; nothing imports the new one yet.

- [ ] **Step 5: Commit**

```bash
git add crates/assets/Cargo.toml crates/assets/src
git commit -m "$(cat <<'EOF'
feat(assets): add Assets manifest to the todo-assets crate

Verbatim port of crates/i18n/src/assets.rs into crates/assets/src/manifest.rs,
including its five unit tests. The i18n copy remains in place for one more
commit while consumers migrate.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Migrate `Assets` consumers off `todo-i18n`; delete `crates/i18n/src/assets.rs`

**Files:**
- Modify: `crates/i18n/Cargo.toml`
- Modify: `crates/i18n/src/lib.rs`
- Modify: `crates/i18n/src/minijinja_helpers.rs` (still keeps the `asset()` helper; just switches import source)
- Modify: `crates/app/Cargo.toml`
- Modify: `crates/app/src/main.rs`
- Modify: `crates/app/src/state.rs`
- Delete: `crates/i18n/src/assets.rs`

After this task, only `todo-assets` defines `Assets`. The `asset()` minijinja helper still lives in `crates/i18n/` but imports `Assets` from `todo_assets`. This is a one-task transient: Task 4 finishes the migration by moving the helper out of i18n too.

- [ ] **Step 1: Add `todo-assets` as a dependency of `todo-app`**

Edit `crates/app/Cargo.toml`. Find the `[dependencies]` block and add (alphabetically among internal crates):

```toml
todo-assets        = { workspace = true }
```

A reasonable spot is right next to the other `todo-*` dependencies — keep them grouped.

- [ ] **Step 2: Add `todo-assets` as a dependency of `todo-i18n` (transient)**

Edit `crates/i18n/Cargo.toml`. Add to `[dependencies]`:

```toml
todo-assets = { workspace = true }
```

This dep is removed at the end of Task 4 once the helper itself moves.

- [ ] **Step 3: Drop `sha2` and `base64` from `crates/i18n/Cargo.toml`**

Find these two lines in `[dependencies]`:
```toml
sha2             = { workspace = true }
base64           = { workspace = true }
```

Delete both. `sha2` was only used by the now-departed `assets.rs`; `base64` is unused (verified by grep over `crates/i18n/src/*.rs`).

- [ ] **Step 4: Switch the `Assets` import in `crates/i18n/src/minijinja_helpers.rs`**

Find:
```rust
use crate::{
    assets::Assets,
    datetime::{format_datetime, DateTimeStyle},
    messages::{FluentArgs, Locales},
    tz::{Tz, UTC},
};
```

Change to:
```rust
use todo_assets::Assets;

use crate::{
    datetime::{format_datetime, DateTimeStyle},
    messages::{FluentArgs, Locales},
    tz::{Tz, UTC},
};
```

(External crate import first, then internal `use crate::{...}`.)

The test module's `use super::*;` still works. The test `build_env()` uses `Assets::dev(PathBuf::from("."))` — that now resolves to `todo_assets::Assets::dev` through the re-export. No further edit needed in the test code.

- [ ] **Step 5: Remove `Assets` from `crates/i18n/src/lib.rs`**

Find:
```rust
pub mod assets;
pub mod datetime;
pub mod locale;
pub mod messages;
pub mod minijinja_helpers;
pub mod tz;

pub use assets::Assets;
pub use datetime::{format_datetime, DateTimeStyle};
pub use locale::{negotiate, SUPPORTED};
pub use messages::{FluentArgs, Locales};
pub use minijinja_helpers::{register, Helpers};
pub use tz::{parse_tz, Tz, UTC};
```

Change to:
```rust
pub mod datetime;
pub mod locale;
pub mod messages;
pub mod minijinja_helpers;
pub mod tz;

pub use datetime::{format_datetime, DateTimeStyle};
pub use locale::{negotiate, SUPPORTED};
pub use messages::{FluentArgs, Locales};
pub use minijinja_helpers::{register, Helpers};
pub use tz::{parse_tz, Tz, UTC};
```

(Removed `pub mod assets;` and `pub use assets::Assets;`.)

Also update the doc comment at the top — the second sentence currently says "crate is depended on by `app` and has no edges into `domain` or `storage`." After this change it also has an edge into `todo-assets`; mention that:

Find:
```rust
//! Internationalization, timezone-aware datetime formatting, and a
//! content-hashed asset manifest. This crate is depended on by `app` and
//! has no edges into `domain` or `storage`.
```

Change to:
```rust
//! Internationalization and timezone-aware datetime formatting. The
//! minijinja helpers in this crate need `todo_assets::Assets` to
//! register the `asset()` helper; that dependency goes away once
//! the helper itself moves into `todo-assets`.
```

(This temporary doc note gets simplified again in Task 4.)

- [ ] **Step 6: Switch the `Assets` import in `crates/app/src/state.rs`**

Find line 5:
```rust
use todo_i18n::{Assets, Locales};
```

Change to:
```rust
use todo_assets::Assets;
use todo_i18n::Locales;
```

- [ ] **Step 7: Switch the `Assets` references in `crates/app/src/main.rs`**

Find these two lines (around line 70–75):
```rust
        Arc::new(todo_i18n::Assets::dev(config.static_dir.clone()))
    } else {
        Arc::new(
            todo_i18n::Assets::production(config.static_dir.clone())
```

Change to:
```rust
        Arc::new(todo_assets::Assets::dev(config.static_dir.clone()))
    } else {
        Arc::new(
            todo_assets::Assets::production(config.static_dir.clone())
```

(Two occurrences swapped from `todo_i18n::Assets` to `todo_assets::Assets`. The `let helpers = todo_i18n::minijinja_helpers::Helpers { ... };` block below is left alone — that's Task 4's surgery.)

- [ ] **Step 8: Delete `crates/i18n/src/assets.rs`**

```bash
git rm crates/i18n/src/assets.rs
```

(Use `git rm` to stage the deletion in one step; this also avoids any `rm` sandbox issues.)

- [ ] **Step 9: Build the workspace**

```bash
cargo build --workspace
```

Expected: clean build. If anything fails to compile, the most likely cause is a missed `todo_i18n::Assets` reference somewhere not anticipated by this plan — grep for it: `grep -rn 'todo_i18n::Assets\|i18n::assets' crates/`.

- [ ] **Step 10: Run tests**

```bash
cargo test --workspace --lib --bins
```

Expected: same total test count as before. The 5 manifest tests now report under `todo-assets`; i18n still has its 3 minijinja helper tests + 2 escape tests = 5; messages tests + datetime tests still in i18n.

- [ ] **Step 11: Verify `cargo tree` shows expected edges**

```bash
cargo tree -p todo-i18n --depth 1 | grep -E 'sha2|base64|todo-assets'
```

Expected:
- No `sha2` line.
- No `base64` line.
- One `todo-assets` line (the transient dep added in Step 2).

```bash
cargo tree -p todo-assets --depth 1 | grep -E 'fluent|icu|time-tz'
```

Expected: no output. `todo-assets` does NOT depend on any of those.

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(i18n): migrate Assets consumers to todo-assets; delete the i18n copy

crates/app and crates/i18n now import Assets from todo_assets instead of
todo_i18n. The old crates/i18n/src/assets.rs is gone. The asset()
minijinja helper still lives in i18n for one more commit; it imports
Assets from the new crate. sha2 + base64 are dropped from i18n's deps
(base64 was already unused).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Move the `asset()` minijinja helper into `todo-assets`

**Files:**
- Create: `crates/assets/src/minijinja.rs`
- Modify: `crates/assets/src/lib.rs`
- Rename: `crates/i18n/src/minijinja_helpers.rs` → `crates/i18n/src/minijinja.rs` (via `git mv`)
- Modify: the renamed file (drop `asset()`, drop `Helpers`, drop the asset test)
- Modify: `crates/i18n/src/lib.rs`
- Modify: `crates/i18n/Cargo.toml` (drop the transient `todo-assets` dep)
- Modify: `crates/app/src/main.rs`
- Modify: `crates/app/src/templates.rs`

After this task, i18n owns only the t/datetime helpers and has no dep on `todo-assets`. Each crate's minijinja surface lives in a `minijinja` module exposing `register`.

- [ ] **Step 1: Create `crates/assets/src/minijinja.rs`**

```rust
//! Minijinja global: `asset(logical)`. Resolves a logical path
//! (e.g. `css/app.css`) through the `Assets` manifest and returns the
//! served URL (e.g. `/static/css/app.<hash>.css` in prod,
//! `/static/css/app.css` in dev).

use std::sync::Arc;

use minijinja::{value::Value, Environment, Error as JinjaError};

use crate::manifest::Assets;

pub fn register(env: &mut Environment<'static>, assets: Arc<Assets>) {
    env.add_function("asset", move |logical: String| {
        let resolved = assets.resolve(&logical);
        Ok::<_, JinjaError>(Value::from(format!("/static/{}", resolved)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use minijinja::context;
    use std::path::PathBuf;

    #[test]
    fn asset_helper_prepends_static_prefix() {
        let assets = Arc::new(Assets::dev(PathBuf::from(".")));
        let mut env = Environment::new();
        register(&mut env, assets);
        env.add_template("test.txt", "{{ asset('css/app.css') }}")
            .unwrap();
        let out = env
            .get_template("test.txt")
            .unwrap()
            .render(context! {})
            .unwrap();
        assert_eq!(out, "/static/css/app.css");
    }
}
```

(The test is verbatim from the asset-helper-related part of the old i18n test module, simplified to not need `Locales`.)

- [ ] **Step 2: Update `crates/assets/src/lib.rs` to expose the new module**

```rust
//! Content-hashed static asset manifest and the `asset()` minijinja helper.
//! Used by the HTTP layer (`todo-app`) to render long-cacheable URLs for
//! static files and to serve those hashed URLs.

pub mod manifest;
pub mod minijinja;

pub use manifest::{Assets, AssetsError};
```

- [ ] **Step 3: Rename `crates/i18n/src/minijinja_helpers.rs` → `crates/i18n/src/minijinja.rs`**

```bash
git mv crates/i18n/src/minijinja_helpers.rs crates/i18n/src/minijinja.rs
```

- [ ] **Step 4: Edit the renamed file to drop `asset()`, drop `Helpers`, drop the asset test**

Open `crates/i18n/src/minijinja.rs` (the renamed file) and replace its full contents with:

```rust
//! Minijinja globals: `t(id, **kwargs)` and `datetime(value, style)`.
//! The globals read shared state via closures so the Environment only has
//! to be set up once.

use std::borrow::Cow;

use fluent_templates::fluent_bundle::FluentValue;
use minijinja::{value::Value, Environment, Error as JinjaError, ErrorKind};
use time::OffsetDateTime;
use unic_langid::{langid, LanguageIdentifier};

use crate::{
    datetime::{format_datetime, DateTimeStyle},
    messages::{FluentArgs, Locales},
    tz::{Tz, UTC},
};

pub fn register(env: &mut Environment<'static>, locales: Locales) {
    let locales_for_t = locales.clone();
    env.add_function(
        "t",
        move |state: &minijinja::State<'_, '_>, id: String, kwargs: minijinja::value::Kwargs| {
            let locale = current_locale(state);
            let args = kwargs_to_args(&kwargs);
            Ok::<_, JinjaError>(Value::from(locales_for_t.lookup(&locale, &id, args.as_ref())))
        },
    );

    env.add_function(
        "datetime",
        move |state: &minijinja::State<'_, '_>,
              value: Value,
              kwargs: minijinja::value::Kwargs|
              -> Result<Value, JinjaError> {
            let locale = current_locale(state);
            let tz = current_tz(state);
            let style_str: String = kwargs.get("style").unwrap_or_else(|_| "medium".into());
            let style = DateTimeStyle::parse(&style_str);
            let dt = value_to_offset_datetime(&value)?;
            let iso = dt
                .format(&time::format_description::well_known::Rfc3339)
                .map_err(|e| JinjaError::new(ErrorKind::InvalidOperation, e.to_string()))?;
            let inner = format_datetime(dt, &locale, tz, style);
            let html = format!(
                "<time datetime=\"{}\" data-style=\"{}\">{}</time>",
                escape_attr(&iso),
                escape_attr(&style_str),
                escape_text(&inner),
            );
            Ok(Value::from_safe_string(html))
        },
    );
}

fn current_locale(state: &minijinja::State<'_, '_>) -> LanguageIdentifier {
    state
        .lookup("_locale")
        .and_then(|v| v.as_str().map(str::to_owned))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| langid!("en"))
}

fn current_tz(state: &minijinja::State<'_, '_>) -> Tz {
    state
        .lookup("_tz")
        .and_then(|v| v.as_str().map(str::to_owned))
        .and_then(|s| crate::tz::parse_tz(&s))
        .unwrap_or(UTC)
}

fn kwargs_to_args(kwargs: &minijinja::value::Kwargs) -> Option<FluentArgs<'static>> {
    let names: Vec<String> = kwargs.args().map(str::to_owned).collect();
    if names.is_empty() {
        return None;
    }
    let mut args: FluentArgs<'static> = std::collections::HashMap::new();
    for name in names {
        if let Ok(v) = kwargs.get::<Value>(&name) {
            // Convert minijinja Value into FluentValue. We accept strings and
            // integers; everything else gets stringified.
            let fv: FluentValue<'static> = if let Some(s) = v.as_str() {
                FluentValue::from(s.to_owned())
            } else if let Some(n) = v.as_i64() {
                FluentValue::from(n)
            } else {
                FluentValue::from(v.to_string())
            };
            args.insert(Cow::Owned(name), fv);
        }
    }
    Some(args)
}

fn value_to_offset_datetime(value: &Value) -> Result<OffsetDateTime, JinjaError> {
    // Templates render the OffsetDateTime via Value::from_serialize, which
    // currently produces an RFC 3339 string. Accept that string here.
    let s = value
        .as_str()
        .ok_or_else(|| JinjaError::new(ErrorKind::InvalidOperation, "datetime expects a string"))?;
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .map_err(|e| JinjaError::new(ErrorKind::InvalidOperation, e.to_string()))
}

/// Full HTML attribute-value escape: handles `&`, `<`, `>`, `"`, `'`.
/// Used for the `datetime` attribute and `data-style` attribute of the
/// generated `<time>` element. Today's inputs (RFC3339 timestamp,
/// `DateTimeStyle::parse` output) are known-safe — this is defense in
/// depth so future callers can't accidentally smuggle markup through.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// HTML text-node escape: handles `&`, `<`, `>`. ICU's formatted output
/// for the four shipped locales is always safe ASCII/printable Unicode
/// without markup-significant characters, but this is defense in depth
/// for future locale additions.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use minijinja::context;
    use std::path::PathBuf;

    fn locales_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("locales")
    }

    fn build_env() -> Environment<'static> {
        let locales = Locales::from_dir(locales_dir()).unwrap();
        let mut env = Environment::new();
        register(&mut env, locales);
        env
    }

    #[test]
    fn t_helper_renders_message() {
        let mut env = build_env();
        env.add_template("test.txt", "{{ t('app-name') }}").unwrap();
        let out = env
            .get_template("test.txt")
            .unwrap()
            .render(context! { _locale => "en" })
            .unwrap();
        assert_eq!(out, "Quiet Ledger");
    }

    #[test]
    fn datetime_helper_emits_time_element() {
        let mut env = build_env();
        env.add_template("test.txt", "{{ datetime(value, style='medium') }}")
            .unwrap();
        let out = env
            .get_template("test.txt")
            .unwrap()
            .render(context! {
                _locale => "en",
                _tz => "America/Los_Angeles",
                value => "2026-05-19T20:00:00Z"
            })
            .unwrap();
        assert!(out.starts_with("<time datetime=\"2026-05-19T20:00:00Z\""));
        assert!(out.contains("data-style=\"medium\""));
        assert!(out.contains("2026"));
    }

    #[test]
    fn escape_attr_handles_all_html_significant_chars() {
        let out = escape_attr(r#"&<>"'"#);
        assert_eq!(out, "&amp;&lt;&gt;&quot;&#39;");
    }

    #[test]
    fn escape_text_handles_markup_chars() {
        let out = escape_text("a<script>&amp;");
        assert_eq!(out, "a&lt;script&gt;&amp;amp;");
    }
}
```

Changes from the original file:
- Module-level doc comment trimmed to mention only `t` and `datetime` (no `asset`).
- `Helpers` struct removed.
- `register` signature is now `register(env: &mut Environment<'static>, locales: Locales)`.
- Inside `register`, the closures no longer reference `helpers.locales` / `helpers.assets`; the `t` closure captures `locales_for_t` (a clone of the `locales` argument).
- The `asset()` `env.add_function(...)` block is gone.
- The `use std::sync::Arc;` and `use todo_assets::Assets;` lines are gone.
- The `asset_helper_prepends_static_prefix` test is gone (moved to `crates/assets/src/minijinja.rs` in Step 1).
- The test `build_env()` no longer constructs Assets and now calls `register(env, locales)`.

- [ ] **Step 5: Update `crates/i18n/src/lib.rs`**

Find:
```rust
//! Internationalization and timezone-aware datetime formatting. The
//! minijinja helpers in this crate need `todo_assets::Assets` to
//! register the `asset()` helper; that dependency goes away once
//! the helper itself moves into `todo-assets`.

pub mod datetime;
pub mod locale;
pub mod messages;
pub mod minijinja_helpers;
pub mod tz;

pub use datetime::{format_datetime, DateTimeStyle};
pub use locale::{negotiate, SUPPORTED};
pub use messages::{FluentArgs, Locales};
pub use minijinja_helpers::{register, Helpers};
pub use tz::{parse_tz, Tz, UTC};
```

Change to:
```rust
//! Internationalization and timezone-aware datetime formatting. This
//! crate is depended on by `app` and has no edges into `domain`,
//! `storage`, or `assets`.

pub mod datetime;
pub mod locale;
pub mod messages;
pub mod minijinja;
pub mod tz;

pub use datetime::{format_datetime, DateTimeStyle};
pub use locale::{negotiate, SUPPORTED};
pub use messages::{FluentArgs, Locales};
pub use minijinja::register;
pub use tz::{parse_tz, Tz, UTC};
```

Changes:
- Doc comment cleaned up.
- `pub mod minijinja_helpers;` → `pub mod minijinja;`.
- `pub use minijinja_helpers::{register, Helpers};` → `pub use minijinja::register;` (drop `Helpers`).

- [ ] **Step 6: Drop the transient `todo-assets` dep from `crates/i18n/Cargo.toml`**

Find:
```toml
todo-assets = { workspace = true }
```

Delete that line.

- [ ] **Step 7: Update `crates/app/src/main.rs`**

Find this block (around line 79–88):
```rust
    let helpers = todo_i18n::minijinja_helpers::Helpers {
        locales: locales.clone(),
        assets: assets.clone(),
    };

    let templates = if config.template_autoreload {
        Templates::dev(config.templates_dir.clone(), helpers)
    } else {
        Templates::production(&config.templates_dir, helpers)
    };
```

Change to:
```rust
    let templates = if config.template_autoreload {
        Templates::dev(
            config.templates_dir.clone(),
            locales.clone(),
            assets.clone(),
        )
    } else {
        Templates::production(&config.templates_dir, locales.clone(), assets.clone())
    };
```

(The `helpers` local is removed; both constructors now take `(dir, locales, assets)`.)

- [ ] **Step 8: Update `crates/app/src/templates.rs` constructor signatures**

Read the file first to confirm current state. Expected current shape (per grep done during planning):

```rust
impl Templates {
    pub fn production(dir: &PathBuf, helpers: Helpers) -> Self {
        let mut env = Environment::new();
        env.set_loader(path_loader(dir));
        register(&mut env, helpers);
        Self::Static(Arc::new(env))
    }

    pub fn dev(dir: PathBuf, helpers: Helpers) -> Self {
        let helpers_for_reload = helpers.clone();
        let reloader = AutoReloader::new(move |notifier| {
            let dir = dir.clone();
            let helpers = helpers_for_reload.clone();
            let mut env = Environment::new();
            env.set_loader(path_loader(&dir));
            register(&mut env, helpers);
            notifier.watch_path(&dir, true);
            ...
```

Change both functions to take `(dir, locales, assets)` and register both crates' helpers:

```rust
impl Templates {
    pub fn production(
        dir: &PathBuf,
        locales: todo_i18n::Locales,
        assets: std::sync::Arc<todo_assets::Assets>,
    ) -> Self {
        let mut env = Environment::new();
        env.set_loader(path_loader(dir));
        todo_i18n::minijinja::register(&mut env, locales);
        todo_assets::minijinja::register(&mut env, assets);
        Self::Static(Arc::new(env))
    }

    pub fn dev(
        dir: PathBuf,
        locales: todo_i18n::Locales,
        assets: std::sync::Arc<todo_assets::Assets>,
    ) -> Self {
        let locales_for_reload = locales.clone();
        let assets_for_reload = assets.clone();
        let reloader = AutoReloader::new(move |notifier| {
            let dir = dir.clone();
            let locales = locales_for_reload.clone();
            let assets = assets_for_reload.clone();
            let mut env = Environment::new();
            env.set_loader(path_loader(&dir));
            todo_i18n::minijinja::register(&mut env, locales);
            todo_assets::minijinja::register(&mut env, assets);
            notifier.watch_path(&dir, true);
            // (Existing code that builds the Environment continues here —
            // do not change the rest of the closure body.)
            Ok(env)
        });
        Self::Reloader(Arc::new(reloader))
    }
}
```

Notes:
- Remove the existing `use todo_i18n::minijinja_helpers::{register, Helpers};` import at the top of `templates.rs` (or whatever the current import is); it is no longer needed.
- The `dev()` constructor's auto-reload closure has more lines than shown above. Preserve everything below the `notifier.watch_path(&dir, true);` line as-is. Only change the helper-registration portion at the top of the closure.

Read the full current file before editing; if the closure body differs from this plan's sketch, preserve it verbatim and only swap the helper-registration calls.

- [ ] **Step 9: Build the workspace**

```bash
cargo build --workspace
```

Expected: clean build. The most likely failure mode is a missed `Helpers` reference or a `minijinja_helpers` import — grep for them: `grep -rn 'minijinja_helpers\|Helpers {' crates/`.

- [ ] **Step 10: Run all tests**

```bash
cargo test --workspace --lib --bins
```

Expected: same total test count as before (26 unit tests across the workspace). Distribution:
- `todo-assets`: 5 manifest tests + 1 minijinja test = 6.
- `todo-i18n`: 1 t-helper test + 1 datetime test + 2 escape tests + (existing messages/locale/datetime tests) — total unchanged minus the moved `asset_helper_prepends_static_prefix`.

- [ ] **Step 11: Verify dependency graph**

```bash
cargo tree -p todo-i18n --depth 1 | grep -E 'sha2|base64|todo-assets'
```

Expected: no output. i18n now has zero of these deps.

```bash
cargo tree -p todo-assets --depth 1 | grep -E 'sha2|minijinja|thiserror'
```

Expected: lines for `sha2`, `minijinja`, `thiserror` (the deps the new crate uses).

- [ ] **Step 12: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(assets): move the asset() minijinja helper out of i18n

Adds crates/assets/src/minijinja.rs with the asset() register function
and its unit test. Renames crates/i18n/src/minijinja_helpers.rs to
crates/i18n/src/minijinja.rs and strips out asset()-related code —
i18n now owns only t() and datetime(). The Helpers struct is gone;
each crate's register fn takes what it needs directly.

Templates::production and Templates::dev now take (dir, locales, assets)
and invoke both register functions. todo-i18n's transient dep on
todo-assets is gone.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update the Workspace layout section**

Find the `i18n/` block:

```
  i18n/           Locale negotiation, Fluent message catalogs, ICU datetime formatting,
                  content-hashed asset manifest, minijinja helpers (t, datetime, asset).
                  May depend on fluent/icu/time-tz/minijinja — NOT axum, NOT sqlx.
  app/            HTTP layer: config, error, state, templates, auth, middleware, router, routes, cache.
```

Change to:

```
  i18n/           Locale negotiation, Fluent message catalogs, ICU datetime formatting,
                  minijinja helpers (t, datetime).
                  May depend on fluent/icu/time-tz/minijinja — NOT axum, NOT sqlx.
  assets/         Content-hashed static asset manifest, minijinja helper (asset).
                  May depend on sha2/minijinja — NOT axum, NOT sqlx, NOT fluent/icu.
  app/            HTTP layer: config, error, state, templates, auth, middleware, router, routes, cache.
```

Also update the section header from `## Workspace layout (4 crates + binary)` to `## Workspace layout (5 crates + binary)` to reflect the new count.

- [ ] **Step 2: Update the "Asset hashing manifest" sharp-edge entry**

Find in CLAUDE.md (currently around line ~140):

```
- **Asset hashing manifest is built at startup in production.** When `template_autoreload=false`, `crates/i18n/src/assets.rs` walks `static/` and computes `sha256(file)[..8]` for each non-precompressed file. ...
```

Change `crates/i18n/src/assets.rs` to `crates/assets/src/manifest.rs`. Leave the rest of the bullet untouched.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "$(cat <<'EOF'
docs(claude): reflect the new todo-assets crate in the workspace layout

Updates the Workspace layout section (now 5 crates) and corrects the
file path in the asset-hashing sharp-edge entry.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Final verification

This task makes no commits. It's a verification gate before merge.

- [ ] **Step 1: clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: zero warnings, zero errors.

- [ ] **Step 2: Full test suite (no Docker)**

```bash
cargo test --workspace --lib --bins
```

Expected: all tests pass.

- [ ] **Step 3: Release build**

```bash
cargo build --release --bin todo-app
```

Expected: succeeds.

- [ ] **Step 4: Smoke test the rendered HTML**

```bash
GIT_SHA=$(git rev-parse --short HEAD) docker compose -f docker/compose.yaml --env-file .env up --build -d
sleep 15
curl -sf http://localhost:3000/ -o /tmp/index.html -w 'GET /: %{http_code}\n'
grep -o 'href="/static/css/app[^"]*"' /tmp/index.html | head -1
HASHED_URL=$(grep -o '/static/css/app\.[a-f0-9]\{8\}\.css' /tmp/index.html | head -1)
echo "Hashed URL in HTML: $HASHED_URL"
curl -sI "http://localhost:3000$HASHED_URL" | grep -iE '^(HTTP|cache-control|content-type)'
docker compose -f docker/compose.yaml --env-file .env down
```

Expected:
- `GET /: 200`.
- The `<link rel="stylesheet">` tag in the HTML references `/static/css/app.<8-hex-chars>.css`.
- `curl -I` on that URL returns:
  - `HTTP/1.1 200 OK`
  - `cache-control: public, max-age=31536000, immutable`
  - `content-type: text/css` (possibly with charset)

If any of those don't match, the refactor introduced a regression — investigate before merging.

- [ ] **Step 5: Confirm the commit graph**

```bash
git log --oneline main..HEAD
```

Expected: 6 commits — the spec doc, then one per Task 1–5. Reviewable in order.

```bash
git diff --stat main..HEAD | tail -5
```

Sanity-check the net diff: it should show a new `crates/assets/` tree, deletions in `crates/i18n/src/assets.rs`, and small edits across the app crate and CLAUDE.md.

---

## Out of scope

- Splitting `t()` from `datetime()` further (e.g., into separate sub-modules).
- Moving the static-serving handler in `crates/app/src/routes/assets.rs` (it stays where it is — only the `Assets` import changes).
- Changing the asset-hashing algorithm or its 8-char output.
- Adding new asset types (images, fonts) with special handling.
