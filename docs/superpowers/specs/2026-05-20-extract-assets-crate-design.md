# Extract content-hashed asset module into `crates/assets/`

**Status:** Approved 2026-05-20
**Branch:** `refactor/extract-assets-crate`

## Problem

`crates/i18n/` contains a module that has nothing to do with internationalization: `assets.rs` (content-hashed static asset manifest) and the `asset()` minijinja helper inside `minijinja_helpers.rs`. The crate's `Cargo.toml` also pulls `sha2` and `base64` solely on behalf of `assets.rs` — `base64` is actually unused already; `sha2` is only used by `assets.rs`.

The workspace pattern is one crate per concern (`domain`, `storage`, `observability`, `i18n`, `app`). Asset hashing is its own concern and belongs in its own crate.

## Goals

- Move asset hashing into a new `crates/assets/` crate published as `todo-assets`.
- Move the `asset()` minijinja helper into the same crate.
- Leave `t()` and `datetime()` in `crates/i18n/`.
- Make each crate own its minijinja surface via a namespaced `minijinja::register(...)` function.
- No runtime behavior changes: HTML still references `/static/css/app.<hash>.css`; the asset endpoint still returns `Cache-Control: public, max-age=31536000, immutable`.

## Non-goals

- Splitting `t()` from `datetime()` further.
- Moving the static-serving router code out of `crates/app/`.
- Changing the asset-hashing algorithm or output shape.
- Adding new asset types.

## Architecture after the move

```
crates/
  domain/         (unchanged)
  storage/        (unchanged)
  observability/  (unchanged)
  i18n/           loses assets.rs and the asset() helper registration
  assets/         NEW: Assets manifest + asset() minijinja helper
  app/            depends on i18n + assets; calls both register fns at startup
```

Dependency edges:
- `todo-assets` depends on: `sha2`, `minijinja`, `thiserror`, dev-dep `tempfile`. No `axum`, no `sqlx`.
- `todo-i18n` after the move: drops `sha2` and `base64` from its deps.
- `todo-app` adds `todo-assets` to its deps.

## File structure

### New: `crates/assets/`

```
crates/assets/
  Cargo.toml
  src/
    lib.rs        Re-exports: pub use manifest::Assets; pub mod minijinja;
    manifest.rs   pub enum Assets { Hashed { … }, Passthrough { … } }
                  pub fn production(root) -> Result<Self, AssetsError>
                  pub fn dev(root) -> Self
                  pub fn resolve(&self, logical) -> String
                  pub fn resolve_hashed_request(&self, url_path) -> Option<PathBuf>
                  pub fn root(&self) -> &Path
                  pub enum AssetsError { Read(io::Error) }
                  (Verbatim move from crates/i18n/src/assets.rs.)
    minijinja.rs  pub fn register(env: &mut Environment<'static>, assets: Arc<Assets>)
                  Registers ONLY the asset() helper. Same body as the existing
                  block in crates/i18n/src/minijinja_helpers.rs that builds
                  `/static/<resolved>` from a logical path.
```

### Modified: `crates/i18n/`

- **Delete** `src/assets.rs`.
- **Rename** `src/minijinja_helpers.rs` → `src/minijinja.rs` (parity with the new crate's module name).
  - Drop the `asset()` registration body.
  - Remove the `Helpers` struct entirely. Signature becomes `pub fn register(env: &mut Environment<'static>, locales: Locales)`. The closure body changes accordingly: `let locales = locales.clone();` at the top instead of `let locales = helpers.locales.clone();`.
  - Drop `use crate::assets::Assets;`.
- **`src/lib.rs`**:
  - Remove `pub mod assets;` and `pub use assets::Assets;`.
  - Rename `pub mod minijinja_helpers;` → `pub mod minijinja;`.
  - Update the re-export line: `pub use minijinja::register;` (drop the `Helpers` export — struct removed).
- **`Cargo.toml`**: drop `sha2` and `base64` from `[dependencies]`. (Verified: `base64` has no users in any `.rs` file under `crates/i18n/src/`; `sha2` is only used in `assets.rs`.)

### Modified: `crates/app/`

- **`Cargo.toml`**: add `todo-assets = { workspace = true }`.
- **`src/main.rs`**:
  - Replace `use todo_i18n::Assets;` with `use todo_assets::Assets;`.
  - The current single `todo_i18n::register(&mut env, Helpers { locales, assets })` call becomes two calls:
    ```rust
    todo_i18n::minijinja::register(&mut env, locales.clone());
    todo_assets::minijinja::register(&mut env, assets.clone());
    ```
- **`src/state.rs`**: change `use todo_i18n::{Assets, Locales};` to `use todo_assets::Assets; use todo_i18n::Locales;`.
- **`src/router.rs`**: no import change. The router accesses `Assets` through `state.assets` (typed via `AppState`); it never imports the type by name. (Verified by grep.)
- **`src/routes/assets.rs`** (the HTTP route handler) and **`src/main.rs`** call sites that referred to `todo_i18n::Assets::dev` / `Assets::production`: switch to `todo_assets::Assets`.

### Workspace root `Cargo.toml`

- Add `"crates/assets"` to `workspace.members`.
- Add `todo-assets = { path = "crates/assets" }` to `workspace.dependencies`.

## Tests

The existing tests inside `crates/i18n/src/assets.rs` (`#[cfg(test)] mod tests`) move with `assets.rs` to `crates/assets/src/manifest.rs`. The five test functions:
- `dev_passthrough_returns_logical_path`
- `production_injects_hash_before_extension`
- `resolve_hashed_request_round_trips`
- `unknown_logical_path_returns_itself`
- `precompressed_siblings_are_skipped`

The minijinja helper tests in `crates/i18n/src/minijinja_helpers.rs` split:
- `t_helper_renders_message`, `datetime_helper_emits_time_element`, `escape_attr_handles_all_html_significant_chars`, `escape_text_handles_markup_chars` — stay in `crates/i18n/src/minijinja.rs`.
- `asset_helper_prepends_static_prefix` — moves to `crates/assets/src/minijinja.rs`.

The two i18n tests' `build_env` helper currently constructs a `Helpers { locales, assets: Arc::new(Assets::dev(...)) }`. After the split, those tests no longer need `Assets` — `build_env` becomes `register(env, locales)`.

The moved `asset_helper_prepends_static_prefix` test rebuilds its own minimal env: register the asset helper with an `Arc<Assets::dev(...)>` and render `{{ asset('css/app.css') }}`.

## CLAUDE.md update

The "Workspace layout" section currently reads:

```
  i18n/           Locale negotiation, Fluent message catalogs, ICU datetime formatting,
                  content-hashed asset manifest, minijinja helpers (t, datetime, asset).
                  May depend on fluent/icu/time-tz/minijinja — NOT axum, NOT sqlx.
```

After this refactor it reads:

```
  i18n/           Locale negotiation, Fluent message catalogs, ICU datetime formatting,
                  minijinja helpers (t, datetime).
                  May depend on fluent/icu/time-tz/minijinja — NOT axum, NOT sqlx.
  assets/         Content-hashed static asset manifest, minijinja helper (asset).
                  May depend on sha2/minijinja — NOT axum, NOT sqlx, NOT fluent/icu.
```

The "Asset hashing manifest is built at startup in production" sharp-edge entry (around line ~140 in CLAUDE.md) still references `crates/i18n/src/assets.rs`. Update the path to `crates/assets/src/manifest.rs`.

## Error handling

No runtime error paths change. `AssetsError::Read` is the only error type, and it moves verbatim.

## Verification

1. `cargo build --workspace` succeeds.
2. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
3. `cargo test --workspace --lib --bins` passes. Test count: same as before, just distributed differently (assets gets 5 manifest tests + 1 helper test; i18n keeps the t/datetime/escape tests).
4. `cargo build --release --bin todo-app` succeeds.
5. Manual smoke: boot the stack (`just up-prod`), view HTML source, confirm the `<link rel="stylesheet">` references `/static/css/app.<8-hex-hash>.css`, and `curl -I` that URL returns `Cache-Control: public, max-age=31536000, immutable`.
6. `cargo tree -p todo-i18n` shows no `sha2` / `base64` deps.
7. `cargo tree -p todo-assets` shows `sha2` + `minijinja` (no `fluent`, no `icu`, no `time-tz`).

## Migration / rollback

Pure refactor on its own branch. Rollback = `git revert` of the merge commit.

The currently-open Tailwind PR (#4) touches a different set of files (no overlap with `crates/i18n/`, `crates/app/main.rs`, or workspace root). The only file both branches edit is `CLAUDE.md`. Whichever PR merges second resolves a trivial textual conflict.

## Open questions

None. All design choices resolved during brainstorming (destination, helper-split strategy, crate name).
