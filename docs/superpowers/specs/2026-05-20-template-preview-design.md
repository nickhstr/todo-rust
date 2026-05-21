# Template preview tool

**Status:** Approved 2026-05-20
**Branch:** `feat/template-preview`

## Problem

Today the only way to render a minijinja template is to navigate the live app to a page that uses it. That's tolerable for `index.html`, awkward for `partials/todo.html` (which only appears mid-htmx-swap), and miserable for one-off variants like "what does the signup form look like when the email field has a validation error" or "what does the empty-state look like before the user has created any todos." There is also no convenient way to compare all four locales side-by-side without seeding data, switching the locale cookie, and reloading.

We want a Storybook-style preview surface — pick a template, pick a fixture, see it render — without buying into Storybook's tooling.

## Goals

- Render any template in `templates/` in a browser, using the exact production minijinja environment (`Templates` enum, real `t()` and `asset()` globals).
- Supply each render with a hand-edited fixture data set.
- Support multiple named fixtures ("stories") per template.
- Switch locale and timezone via URL query params at preview time.
- Live entirely behind a dev gate — both compiled out of `--release` and runtime-disabled by default.

## Non-goals

- Interactive htmx in previewed partials. Clicking the toggle/delete buttons does nothing in v1. (`hx-*` attributes are dead because the host shell deliberately does not load htmx.)
- Snapshot/regression testing against committed HTML artifacts.
- An in-browser fixture authoring UI — fixtures are hand-edited TOML files.
- Mocking server-side state beyond what a fixture can express (no fake `AuthSession`, no fake CSRF).
- Auto-generating fixtures from domain types.

## Architecture

New module inside the existing app crate:

```
crates/app/src/preview/
  mod.rs        # pub fn router() -> Router<AppState>; re-exports
  routes.rs     # GET /__preview, GET /__preview/*template/:story
  fixtures.rs   # discovery walker + TOML loader
  shell.rs      # host-page wrapper for partials
```

Mounted from `crates/app/src/router.rs` inside the existing `#[cfg(debug_assertions)]` block next to `/dev/login`. The whole `preview` module is `#[cfg(debug_assertions)]`, so the routes are absent from `--release` builds. Handlers also re-check `state.config.dev.preview_enabled` at runtime, so a debug build with the flag off returns 404.

The path prefix `/__preview` (double underscore) is chosen to avoid collision with any plausible real route now or later.

### Configuration

Extend `DevConfig` in `crates/app/src/config.rs`:

```rust
pub struct DevConfig {
    pub auto_login_email: Option<String>,
    pub preview_enabled: bool,         // APP__DEV__PREVIEW (default false)
    pub preview_fixtures_dir: PathBuf, // APP__DEV__PREVIEW_FIXTURES_DIR
                                       // (default "fixtures/templates")
}
```

`docker/compose.dev.yaml` sets `APP__DEV__PREVIEW=true` so it's on automatically in dev. `docker/compose.yaml` (prod) never sets it, and the routes don't compile into the prod image anyway.

### Routes

| Route | Behavior |
|---|---|
| `GET /__preview` | Index — flat tree of templates × stories, with locale picker (`en/es/fr/de`) and tz picker (UTC + a handful of common zones) at top. |
| `GET /__preview/render/*path` | Render one fixture. `*path` is the axum catchall (e.g. `partials/todo.html/default`). The handler splits on the last `/` — everything before is the template path, everything after is the story. `?locale=` / `?tz=` override defaults. |

(Axum's `*foo` catchall must be the final path segment, so we can't write `/*template/:story` directly; the split happens in the handler.)

### Per-render pipeline

1. Load `fixtures/templates/<template-path-without-ext>/<story>.toml`.
2. Deserialize directly into `serde_json::Value` via `toml::from_str::<serde_json::Value>(&s)?` — the toml deserializer accepts any serde target, no intermediate `toml::Value` needed.
3. Build ambient context: `_locale` (query param or `"en"`), `_tz` (query param or `"UTC"`), `csp_nonce` (already populated on request extensions by the existing `csp_nonce_middleware`, same as every other HTML response).
4. Merge ambient into fixture `[ctx]`. **Fixture wins on key conflict** — an explicit override in a fixture always takes effect.
5. Call `state.templates.render(template, merged)`. This is the same `Templates::render` every production handler uses, so `t()` and `asset()` resolve identically.
6. If the template path starts with `partials/`, wrap the rendered fragment in `shell::host(...)`. Otherwise pass through (the template extends `base.html` and is already a full document).
7. Response flows through the normal middleware stack. CSP, version header, request ID, etc. all behave normally.

## Fixture format

```toml
# fixtures/templates/partials/todo/default.toml

[meta]
description = "A pending todo with a recent timestamp"

[ctx]
todo = { id = "00000000-0000-0000-0000-000000000001",
         title = "Buy milk",
         completed = false,
         created_at = "2026-05-19T10:00:00Z",
         updated_at = "2026-05-19T10:00:00Z" }
```

- `[ctx]` — required. Contents become the template render context. Field names mirror what the handler would pass (here: a `todo` matching `todo_domain::Todo`'s serde shape).
- `[meta]` — optional. `description` shows in the index UI; other keys are ignored.

A fixture that omits a field the template uses behaves the same as production: minijinja renders `""` or errors depending on how the template references the value. We do not validate fixture shape against the template — that's the job of the human who writes both.

## Discovery

Walk `templates/` recursively. The walker:

- Skips files whose basename starts with `_` (jinja convention for include-only fragments).
- For each `templates/<rel>.html`, looks up `fixtures/templates/<rel>/*.toml`.
- Sorts stories alphabetically, with `default.toml` always pinned first if present.
- Templates with no fixture dir show on the index marked `[no fixtures]`, with a hint link to "add a fixture at `fixtures/templates/<rel>/default.toml`".

Caching:
- When `Templates` is `Static` (production-shaped, used in dev when `APP__TEMPLATE_AUTORELOAD=false`), the walk happens once at first request and is cached for the process lifetime.
- When `Templates` is `Reloading` (the default dev mode), the walk happens on every index request so new templates and fixtures appear without a restart.

## Host shell for partials

Ships as a minijinja template at `templates/_preview_shell.html`. The leading underscore means the discovery walker skips it (so it doesn't appear on the index), but `path_loader` can still load it by name. This means it gets the existing `t()` and `asset()` helpers for free — no separate env, no `include_str!` plumbing.

```html
<!doctype html>
<html lang="{{ _locale }}">
<head>
  <meta charset="utf-8">
  <title>preview · {{ template }} · {{ story }}</title>
  <link rel="stylesheet" href="{{ asset('css/app.css') }}">
  <script src="{{ asset('vendor/alpine-3.15.12.min.js') }}" defer></script>
  {# Deliberately NO htmx. hx-* attributes are dead markup. #}
</head>
<body>
  <div class="preview-bar">
    PREVIEW · {{ template }} · {{ story }} · locale={{ _locale }} tz={{ _tz }}
    [en] [es] [fr] [de]
  </div>
  <main>
    {{ rendered_partial | safe }}
  </main>
</body>
</html>
```

The preview handler renders the partial first, then renders `_preview_shell.html` with `{rendered_partial, template, story, _locale, _tz}` plus the ambient base context.

Top-level templates (anything that already extends `base.html`) skip the shell and serve as-is.

The 50-odd lines of HTML do ship inside the production binary's compiled-in templates dir, which is harmless. If that ever becomes objectionable, the shell can move to `include_str!` + `env.add_template_owned(...)` registered only when `preview_enabled` is true.

## Error handling

| Case | Response |
|---|---|
| Unknown template path | 404, body lists available templates |
| Story missing for known template | 404, body lists available stories for that template |
| TOML parse error | 500, body shows the underlying `toml::de::Error` plus the file path |
| Render error (minijinja) | Reuses existing `AppError::Template` path. Minijinja errors carry source position info, which surfaces in the response. |
| Preview disabled (debug build, flag off) | 404 |
| Release build (routes don't exist) | 404 from the global fallback |

## Index UI

Server-rendered HTML, no JS framework. Flat list grouped by directory:

```
templates/
  base.html  [no fixtures]
  index.html
    default
    empty
    many-items
  login.html
    default
    with-validation-error
  signup.html
    default
    with-validation-error
  partials/
    todo.html
      default
      completed
      long-title
    todo_list.html
      default
      empty
```

Locale switcher at top: `[en] [es] [fr] [de]` — each is a link to the current URL with `?locale=X`. Tz switcher next to it with a short pre-baked list (`UTC`, `America/New_York`, `Europe/Paris`, `Asia/Tokyo`).

## Seed fixtures shipped in v1

Bundled in the initial commit so the tool is useful immediately:

- `index.html`: `default`, `empty`, `many-items`
- `login.html`: `default`, `with-validation-error`
- `signup.html`: `default`, `with-validation-error`
- `partials/todo.html`: `default`, `completed`, `long-title`
- `partials/todo_list.html`: `default`, `empty`
- `base.html`: no fixtures (layout-only)

## Testing

Unit tests in `crates/app/src/preview/fixtures.rs`:
- TOML parse roundtrip for a sample fixture.
- Discovery walker against a tempdir produces the expected (template, stories) pairs.
- Story sort order: `default.toml` first, rest alphabetical.

One integration test file `crates/app/tests/preview_flow.rs`, gated `#[cfg(debug_assertions)]`:
- `GET /__preview` → 200, body contains template names.
- `GET /__preview/render/partials/todo.html/default` → 200, body contains the fixture's todo title.
- `GET /__preview/render/partials/todo.html/default?locale=es` → 200, body contains a known Spanish string from the catalog.
- `GET` on missing fixture → 404.
- With `preview_enabled=false` → 404 (handler short-circuits even though the route exists).

## Migration and rollout

- Single PR. Branch `feat/template-preview`.
- No data migration; no production code path changes.
- A new direct dep on `toml` in `crates/app/Cargo.toml`. Cargo does not support gating deps on `cfg(debug_assertions)` (that's a rustc cfg, not a target spec), so the crate is unconditionally compiled in. The dev gate keeps the dependency footprint harmless — the parser is only called from preview routes that don't exist in release.
- `CLAUDE.md` gains a short paragraph in the "Where to add things" section pointing future contributors at `fixtures/templates/` when they create a new template.
- `README.md` gets a one-line developer-tools section.

## Out of scope (acknowledged, can layer later)

- Interactive htmx (clicking buttons does nothing). v2 if someone wants live state in previews.
- Snapshot regression tests against committed `.html` artifacts.
- In-browser fixture authoring.
- Preview of authenticated-only behavior that depends on session state beyond what a fixture can mock.
- Validating fixture shape against the template at load time.
- Generating fixtures from typed domain structs.
