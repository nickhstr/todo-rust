# CLAUDE.md — context for future sessions

This file is for AI assistants picking up work on `todo-rust`. It's a running tour of what's in the tree, plus a list of sharp edges that bit previous sessions. `README.md` is the human-facing entry point.

## What this is

A small Rust + axum todo app, production-shaped: Postgres + Valkey, cookie sessions via `axum-login`, htmx + Alpine + Tailwind v4 on the front, OTel + Prometheus + Tempo + Grafana on the side. Multi-stage Dockerfile lands the release binary in `gcr.io/distroless/cc-debian12`.

The point isn't the todos — it's the scaffold. Optimize for that.

## Workspace layout (4 crates + binary)

```
crates/
  domain/         Pure types: UserId, User, Credentials, NewUser, TodoId, Todo, NewTodo, TodoUpdate.
                  No I/O, no axum, no sqlx. May depend on serde/uuid/time/thiserror/validator only.
  storage/        sqlx pool, UserRepository, TodoRepository, MIGRATOR.
                  Embeds migrations from ../../migrations via sqlx::migrate!.
                  May depend on tokio/sqlx/password-auth/tracing — NOT axum.
  observability/  init_tracing (OTLP + fmt subscriber), install_metrics_recorder (Prometheus).
                  Independent of domain/storage/app. Imported only by app.
  i18n/           Locale negotiation, Fluent message catalogs, ICU datetime formatting,
                  content-hashed asset manifest, minijinja helpers (t, datetime, asset).
                  May depend on fluent/icu/time-tz/minijinja — NOT axum, NOT sqlx.
  app/            HTTP layer: config, error, state, templates, auth, middleware, router, routes, cache.
                  src/lib.rs re-exports build_router so integration tests can spin up the same router as main.rs.
                  src/main.rs is the binary entrypoint.
```

Boundaries are enforced by Cargo dependency edges — if you find yourself wanting to import axum from `storage`, refactor instead.

## Build / test / run

| Task | Command |
|---|---|
| Workspace build | `cargo build --workspace` |
| Lint (gate) | `cargo clippy --workspace --all-targets -- -D warnings` |
| Unit tests (no Docker) | `cargo test --workspace --lib --bins` |
| Integration tests (need Docker) | `cargo test --workspace` |
| Release build | `cargo build --release --bin todo-app` |
| Full stack | `just up-prod` (production-like) or `just up` (dev w/ hot reload) |
| Generate session key | `just gen-session-key` (or `just init-env` to write it into `.env`) |

## Frontend dependencies (vendored)

- `static/vendor/htmx-4.0.0-beta3.min.js` (+ `.gz`, `.br`)
- `static/vendor/alpine-3.15.12.min.js` (+ `.gz`, `.br`)

Both are referenced from `templates/base.html` by their versioned filenames. The CSP in `crates/app/src/middleware/security.rs` is locked to `script-src 'self'` — no CDN allowance — so any future inline or third-party script needs a CSP update.

To bump a version: download the new file into `static/vendor/`, regenerate the `.gz` and `.br` siblings (`gzip -9 -k -f $f && brotli -9 -k -f $f`), and update the `<script src=...>` in `base.html`. The `ServeDir` is built with `precompressed_gzip().precompressed_br()` so the compressed siblings serve automatically with the right `Content-Encoding`.

### htmx 4 — validate all usage against the v4 API

**This project uses htmx 4 (beta), which has breaking API changes from htmx 1/2.** Always verify htmx usage against the v4 docs (`four.htmx.org`) or the vendored source. Do not copy examples from htmx 1/2 docs or Stack Overflow without checking compatibility.

Key differences from htmx 1/2:

| htmx 1/2 | htmx 4 | Notes |
|---|---|---|
| `htmx:afterRequest` (camelCase) | `htmx:after:request` (colon-separated) | All lifecycle events renamed |
| `hx-on::after-request` | `hx-on::after:request` | Hyphen in event → colons in v4 |
| `event.detail.successful` | **removed** | Use `hx-on::after:swap` instead (only fires on success) |
| `event.detail.xhr` | `event.detail.ctx.response` | Response object moved under `ctx` |

**Response headers** are NOT all handled — verified against the vendored source, htmx 4 beta3 only recognizes the request-context headers (`HX-Boosted`, `HX-Current-URL`, `HX-History-Restore-Request`, `HX-Request`, `HX-Request-Type`, `HX-Source`, `HX-Target`). **`HX-Refresh`, `HX-Redirect`, `HX-Retarget`, and `HX-Reswap` are silently dropped.** If you need to make htmx reload the page after a POST, return a 303 + `Location` and have the form submit be a plain `<form>` (the browser follows the redirect). The language switcher in `templates/base.html` is the worked example.

**Swap spec modifiers** (`swap:200ms`, `settle:100ms`) still work — `parseInterval` handles `ms`/`s`/`m` suffixes.

**Rule:** any time you write `event.detail.*` in an `hx-on` handler, verify the property exists in the vendored htmx 4 source (`grep -o "successful\|xhr\|etc" static/vendor/htmx-4.0.0-beta3.min.js`).

## Things the plan says one way but the code does differently

| Plan | Reality | Why |
|---|---|---|
| Rust 1.82 pinned (plan + Dockerfile) | `rust-toolchain.toml` and `docker/Dockerfile*` pin to **1.94** | The dev box ships 1.94. Also: a transitive dep (`base64ct 1.8.3`) requires `edition2024`, which needs Rust ≥ 1.85, so 1.82 won't build the Docker image at all anymore. |
| `Templates::dev` watches `templates/` for autoreload | Yes, via `minijinja-autoreload::AutoReloader` | Toggle with `APP__TEMPLATE_AUTORELOAD=true`. |
| Build router in `main.rs` | Lives at `crates/app/src/router.rs`, re-exported as `todo_app::build_router` | Integration tests need to construct the same router; binary-private functions aren't visible to them. |
| Toggle endpoint `hx-target="this"` | `hx-target="closest li"` | The endpoint returns the full `<li>` partial; `this` would target just the button. |
| Tailwind v3 fallback | **Tailwind v4 CSS-first**. Design tokens in `static/css/app.src.css` `@theme { … }`. `tailwind.config.js` exists only as a content-scan fallback. | v4 is the current Tailwind; v3-style `tailwind.config.js` would have been a regression. |
| `axum-login` `verify_password` override | Removed | `axum-login` 0.16's `AuthnBackend` trait no longer exposes this hook; argon2 verification happens in `UserRepository::verify` via `spawn_blocking`. |
| `tower::limit::RateLimitLayer` for /login | Custom `RateLimiter` middleware (per-IP token bucket) in `middleware/rate_limit.rs` | `tower::limit::RateLimitLayer` is global, not per-IP. 5 logins/min/IP, burst of 5. |

## Things the plan acknowledged as `[VERIFY]` and how they landed

- **htmx 4 beta**: loaded from `cdn.jsdelivr.net/npm/htmx.org@next/dist/htmx.min.js`. If 4.0.0 ships, pin to a semver.
- **opentelemetry-rust crates**: pinned to 0.26 family. `opentelemetry-otlp` 0.26 uses the `new_pipeline().tracing().with_exporter(...).install_batch(Tokio)` shape — NOT the `SpanExporter::builder()` form mentioned in some examples. See `crates/observability/src/lib.rs`.
- **fred 9.x**: `KeysInterface` is feature-gated under `i-keys`; `RedisPool` lives at `fred::prelude::RedisPool`. `Builder::from_config(RedisConfig::from_url(url))?.build_pool(n)?` is the current shape. We enable `i-keys` + `i-server` + `enable-rustls-ring` + `metrics`.

## How auth + sessions actually work

1. `tower-sessions-sqlx-store::PostgresStore::migrate()` at startup creates the session table.
2. `SessionManagerLayer` is `.with_signed(Key::from(&session_key))` — see `Config::decoded_session_key`. Key must decode (hex preferred, raw bytes accepted) to ≥64 bytes or startup bails.
3. `axum-login`'s `AuthSession` extractor is wired via `AuthManagerLayer`. The user record is `AuthUserRecord(pub User)` — a newtype wrapper because the orphan rule prevents implementing `AuthUser` directly on `todo_domain::User`.
4. Signup flow: `users.create(NewUser)` → wrap the returned `User` in `AuthUserRecord` → `auth.login(&record)`. We deliberately skip the post-create `auth.authenticate(...)` round-trip because we already have a verified-fresh `User` — a second argon2 verify would cost ~50–100 ms for nothing.
5. Session cleanup runs as a tokio task (`continuously_delete_expired`); aborted on shutdown.

## Observability surface

- `/metrics` — Prometheus text. Mounted on the same router; in prod you should firewall this externally.
- `/healthz` — liveness; never touches DB.
- `/readyz` — pings the pool; 503 if not ready.
- Spans: one per HTTP request via `TraceLayer`; repository methods are `#[tracing::instrument(skip(self, …))]`.
- Metrics: `http_requests_total`, `http_request_duration_seconds`, `todos_{created,toggled,deleted}_total`, `auth_{logins,signups}_total{result}`, `cache_operations_total{op,result}`. `http_metrics_layer` keys on `MatchedPath` so cardinality stays bounded.

## Where to add things

- **A new HTTP route** → `crates/app/src/routes/<file>.rs`; register in `crates/app/src/router.rs`.
- **A new domain type** → `crates/domain/src/<file>.rs` first, then a repository method in `crates/storage`, then a handler in `crates/app`.
- **A new metric** → emit in the relevant handler with `metrics::counter!()` or `metrics::histogram!()`. If it should land in the Grafana dashboard, also add a panel to `docker/grafana/dashboards/app.json`.
- **A new template** → put in `templates/` (partials in `templates/partials/`); `Templates::render("name.html", context)` from a handler.

## Known gaps / "this would be nice next"

- **Non-JS fallback**: the toggle uses a `<form>` POST but returns a single `<li>` fragment; non-JS users see a broken page. Delete is htmx-only. Acceptable since the app requires JS, but call it out if JS-less support becomes a goal.
- **No load test was run.** Plan's target is ~1000 RPS on `GET /` with cache hot, p95 < 50ms on a developer laptop.
- **`Templates::dev` autoreload** is wired but never observed via test.
- **Tempo storage** uses local volume; the config is the minimum that boots Tempo 3.0-rc — newer versions removed `ingester` and `compactor` config keys, so anything more elaborate from older docs will fail.

## Sharp edges to know about

- **OTel `tracing` → log correlation needs an explicit context activator.** `opentelemetry-appender-tracing` builds a `LogRecord` and calls `Logger::emit`. The SDK auto-fills `LogRecord::trace_context` from `Context::map_current(...)` — but `tracing-opentelemetry` stores the OTel span in tracing's per-span extensions, not on OTel's thread-local context stack. Without a bridge, every log record ships with an empty trace context. `crates/observability/src/otel_context_layer.rs` is the bridge: on each `on_enter` it reads `OtelData` from the span's extensions, builds an `opentelemetry::Context` via `with_remote_span_context(...)`, calls `.attach()`, and parks the guard on a thread-local LIFO stack. `on_exit` pops. Order matters in `init_tracing`: the activator must be added **after** `tracing-opentelemetry::layer()` (which populates `OtelData` in `on_new_span`) and **before** `OpenTelemetryTracingBridge`.
- **Tempo `/api/traces/<id>` returns 404 without a time hint.** Tempo 3.0's live-store needs `?start=&end=` query params to scope the lookup. `/api/search` falls back to a stale default window too — pass explicit `start` / `end` (epoch seconds) when querying programmatically. Grafana's Tempo datasource passes these automatically.
- **Tailwind v4 `--watch` exits when stdin closes** — and docker compose doesn't allocate a TTY by default. Symptom: container exits 0 after printing the version banner; `static/css/app.css` stays at 0 bytes; pages load unstyled. The `package.json` `watch` script uses `--watch=always` for the same reason.
- **Native FS events don't cross podman's macOS bind-mount.** Inotify never fires inside the container for changes the host makes. This breaks every FS-event-driven watcher (Tailwind v4 uses `@parcel/watcher`; `cargo-watch` uses `notify-rs`; `CHOKIDAR_USEPOLLING` doesn't help because neither uses chokidar). The dev compose works around it with explicit polling: `cargo watch --poll` for the app, and a shell `find -printf '%T@' | sort | tail -1` mtime-snapshot loop calling `tailwindcss` one-shot for the CSS. If you ever move to a runtime that does propagate events (Docker Desktop with VirtioFS, or native Linux), the polling is wasted but cheap.
- **`systemfd -s http::3000` defaults to 127.0.0.1**, not 0.0.0.0. In a container that's invisible from outside the bridge network. The dev compose command and `just run` both pass `-s http::0.0.0.0:3000` explicitly.
- **`cargo install ... --features rustls,postgres` applies the feature flags to ALL listed packages.** `systemfd` and `cargo-watch` don't have those features and the install will fail. Install sqlx-cli on its own line.
- **`http_metrics_layer` collapses `/static/*` to a single label**. `nest_service` doesn't set `MatchedPath`, so without the collapse, every probed filename becomes a distinct Prometheus series. If you ever move static serving off of `nest_service`, drop the collapse.
- **Don't change `users.verify` to skip the dummy argon2 hash** when the user doesn't exist. It's there on purpose to equalize timing and prevent user-enumeration via response time. `crates/storage/src/user_repo.rs` `TIMING_DUMMY_HASH`.
- **Don't trust `?next=` query-string redirects.** `routes::auth::safe_next` rejects anything that doesn't start with `/` and `//foo` (scheme-relative) — keep it that way.
- **The `MIGRATOR` macro path is relative to the crate** (`../../migrations`). If you move the migrations folder, update `crates/storage/src/lib.rs`.
- **`SQLX_OFFLINE=true` is set in the Dockerfile** as a safety net. We use `sqlx::query_as` not `sqlx::query!`, so it isn't strictly required, but if anyone adds a query macro later they'll need `cargo sqlx prepare --workspace` to generate `.sqlx/`.
- **rate_limit_middleware needs `ConnectInfo`** — `main.rs` and `tests/common/mod.rs` both use `app.into_make_service_with_connect_info::<SocketAddr>()`. If you ever swap `axum::serve(listener, app)` back to the plain form, the rate limit becomes effectively global.
- **`X-App-Version` SHA is baked at compile time** by `crates/app/build.rs`. It reads `$GIT_SHA` first, then falls back to `git rev-parse --short HEAD`, then to `"unknown"`. `.git/` is in `.dockerignore`, so the production Dockerfile shells the SHA in via `ARG GIT_SHA` — the prod `compose.yaml` threads it as a build-arg; `compose.dev.yaml` passes it as a container env (dev compiles inside the container via cargo-watch, after image build). `just up*` populates it from `git rev-parse --short HEAD`. A bare `docker build .` (no build-arg) will stamp `"unknown"`.
- **Dev passwordless login is gated TWO ways.** `POST /dev/login` exists when (a) the binary was built with `debug_assertions` AND (b) `APP__DEV__AUTO_LOGIN_EMAIL` is non-empty. The route module (`crates/app/src/routes/dev.rs`) is `#[cfg(debug_assertions)]`, so `--release` literally doesn't link it; the handler also checks `DevConfig::enabled_email()` at runtime. `main::ensure_dev_user` seeds the account with a throwaway random password on startup; the `/dev/login` handler bypasses verify. Set in `docker/compose.dev.yaml` to `dev@local`. Don't add this to `compose.yaml` — that's the prod path.
- **CSP nonces are per-request.** The static `Content-Security-Policy` header layer was replaced by `csp_nonce_middleware` in `crates/app/src/middleware/security.rs`, which generates a 128-bit base64 nonce, stashes `CspNonce` on request extensions, and writes `script-src 'self' 'unsafe-eval' 'nonce-<value>'` into the response. Templates pull the nonce via the standard context key `csp_nonce` (provided by `crates/app/src/render.rs::base_context`) and emit it on any inline `<script>`. Don't add `'unsafe-inline'` back — the whole point is to avoid it. `'unsafe-eval'` stays because Alpine.js and htmx 4's `hx-on::*` runtime-compile expressions.
- **Default `Cache-Control: private, no-cache` on every response.** Set by `SetResponseHeaderLayer::if_not_present` in the router. Handlers and the static-asset path override; the hashed-asset handler sets `public, max-age=31536000, immutable`, and the wrapped `ServeDir` for unhashed assets gets `public, max-age=300` via `SetResponseHeaderLayer::overriding`. If you add a route that wants a longer HTML cache, set the header yourself — `if_not_present` semantics let you override.
- **Asset hashing manifest is built at startup in production.** When `template_autoreload=false`, `crates/i18n/src/assets.rs` walks `static/` and computes `sha256(file)[..8]` for each non-precompressed file. The `asset()` minijinja global resolves logical paths (`css/app.css`) to hashed URLs (`/static/css/app.<hash>.css`). In dev (`template_autoreload=true`), the manifest is a passthrough; raw paths are used so Tailwind `--watch` can edit files freely. The custom service in `router.rs` checks `Assets::resolve_hashed_request(url_path)` first; on miss it falls through to `ServeDir` with the short cache.
- **`<time datetime="…">` is the canonical source of truth for dates.** The server renders the inner text in the user's locale + tz via `crates/i18n/src/datetime.rs` (ICU CLDR patterns), and an inline upgrader script in `<head>` reformats client-side using `Intl.DateTimeFormat` (also setting the `tz` cookie on first visit). If you add a date-bearing template, use `{{ datetime(value, style="medium") }}` rather than printing the value directly.
- **i18n cookie names are `locale` and `tz`** (not signed). The language switcher writes `locale` via `POST /preferences/locale` (also persists to `users.locale` if authenticated); the JS upgrader writes `tz`. Server-side precedence per `crates/app/src/middleware/i18n.rs`: profile (authenticated, in handlers) → cookie → `Accept-Language` → en for locale; profile → cookie → UTC for tz.
- **Fluent message IDs are validated at use time, not at compile time.** Templates and `#[validate(message = "...")]` attributes hold raw IDs; the `t()` minijinja helper and the HTTP-edge error mapper resolve them via `state.locales.lookup(...)`. A typo in an ID renders the literal ID and logs `i18n: missing message id` with an `i18n_missing_key_total{locale,key}` metric — visible at runtime, not at build time.
- **`time` serde format defaults to a tuple, not a string.** `OffsetDateTime` fields that flow through `Value::from_serialize` (template render context, session cookie) must have `#[serde(with = "time::serde::rfc3339")]` or the `datetime()` minijinja helper fails. Both `Todo.{created_at, updated_at}` and `User.created_at` are tagged.
- **`icu` ships ~4 MB of CLDR data baked into the binary.** `icu = { features = ["compiled_data"] }` is the default and pulls all locales. To trim to just the four shipped locales (en/es/fr/de) would require an `icu_datagen` build.rs step — recorded as future work; the simpler default is good enough for v1.

## What was verified end-to-end with Docker

`docker compose -f docker/compose.yaml --env-file .env up --build -d` brings up app + Postgres + Valkey + OTel collector + Tempo + Prometheus + Grafana. Smoke test confirmed:

- `GET /healthz` → 200, `GET /readyz` → 200, `GET /metrics` exposes the Prometheus text format with proper templated path labels (`/todos/:id`, `/static/*` — no cardinality blow-up).
- Signup → 303 + signed session cookie; index page loads templated HTML; create / toggle / delete todo work via htmx-style POST/DELETE/POST with the expected status codes (201/200/200).
- Rate limiter trips after 5 failed logins from one IP → 429 + `auth_rate_limited_total` counter increments.
- Tailwind v4 CSS compiled to `/static/css/app.css` and served at 200 with `text/css`.
- All 21 tests pass (5 unit + 4 auth_flow + 2 todos_flow + 5 storage repo + 2 rate_limit + 3 xff). Integration tests use testcontainers and need Docker.
- OTel trace pipeline: app → otel-collector → tempo → Tempo's `/api/search?tags=service.name=todo-app` returns spans.
- Prometheus scrapes `app:3000/metrics`; both targets `up`; dashboard `todo-app` provisioned and the panel queries return data.
- Graceful shutdown: SIGTERM → "SIGTERM received; shutting down" → "todo-app shut down" → process exits clean.

What **isn't** end-to-end-tested: load test (no time-on-load was measured), template autoreload (Templates::dev path), and 3-replica scale-out (podman's port forwarding doesn't multiplex a single host port across replicas on macOS — the app is stateless so scale-out would work behind a real LB).

## Tone for changes in this repo

- Boring + correct beats clever. Hot reload, observability, and graceful shutdown should keep working.
- Domain types don't leak into HTTP types; HTTP types don't leak into storage. Newtype wrappers (`AuthUserRecord`, `TodoId`, `UserId`) exist on purpose.
- If a fix requires touching three crates, that's a sign the boundary is wrong — refactor first.
