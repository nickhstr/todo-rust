# todo-rust

A small but production-shaped todo app in Rust. axum + sqlx + Postgres + Valkey, with htmx / Alpine / Tailwind v4 on the front, OpenTelemetry + Prometheus + Tempo + Loki + Grafana on the side.

The point isn't the todos — it's the scaffold: auth, observability, caching, hot reload, container-friendliness.

## Stack at a glance

| | |
|---|---|
| Web framework | `axum` 0.7 + `tower-http` |
| Templating | MiniJinja with autoreload in dev |
| Database | Postgres 16 (via `sqlx` 0.8) |
| Sessions | `axum-login` + `tower-sessions` with signed cookies, Postgres-backed store |
| Cache | Valkey 7 via `fred` 9.x |
| Frontend | htmx 4.0.0-beta3 + Alpine.js 3 + Tailwind v4 — all vendored locally; no CDN, no Google Fonts |
| Tracing | `tracing` → `tracing-opentelemetry` → OTLP/gRPC → otel-collector → Tempo |
| Logs | `tracing` → `opentelemetry-appender-tracing` → OTLP/gRPC → otel-collector → OTLP-HTTP → Loki |
| Metrics | `metrics` + `metrics-exporter-prometheus`, Prometheus scrapes `/metrics` |
| Dashboards | Grafana with provisioned Prometheus / Tempo / Loki datasources |
| Container | Multi-stage `Dockerfile` → distroless `cc-debian12` runtime |

## Quick start

```bash
cp .env.example .env
just init-env             # generates a real SESSION_KEY and writes it into .env

just up-prod              # full stack: app + Postgres + Valkey + OTel + Prometheus + Tempo + Loki + Grafana

# wait for readiness
until curl -sf localhost:3000/readyz; do sleep 1; done
```

Then visit:

- **App** — <http://localhost:3000>
- **Grafana** — <http://localhost:3001> (anonymous viewer; admin/admin to edit)
- **Prometheus** — <http://localhost:9090>
- **Tempo API** — <http://localhost:3200>
- **Loki API** — <http://localhost:3100>

The Grafana instance ships with one provisioned dashboard ("todo-app") plus Prometheus, Tempo and Loki datasources, all wired for click-through correlation:

- Log row in Loki Explorer → **View trace in Tempo** link (uses the `trace_id` structured metadata)
- Tempo span → **Logs for this span** button → Loki query scoped by `service_name` + `trace_id`

## Local dev (hot reload)

```bash
just up
```

This uses `docker/compose.dev.yaml` as an override. What you get:

- `cargo-watch --poll` running inside the app container; edit a `.rs` file, the binary rebuilds and `systemfd` hands the listening socket to the new process (zero-downtime port).
- A `tailwind` container running a 1s mtime-polling loop that calls the one-shot `tailwindcss` CLI whenever `static/css/app.src.css` or any file under `templates/` changes.
- `minijinja-autoreload` with `fast_reload(false)` in dev, so template edits show up on the next request without a server restart.

All three watchers use polling instead of native FS events. This is deliberate — native FS events don't propagate across podman's macOS bind-mounts and `CHOKIDAR_USEPOLLING` is ignored by Tailwind v4's watcher (which uses `@parcel/watcher`, not chokidar). Polling at 1s costs nothing and works on every container runtime.

If you'd rather run the app outside Docker:

```bash
# Need a Postgres and a Valkey somewhere (e.g. `docker compose up db cache`)
npm install
npm run build              # one-shot CSS build (or `npm run watch` for the polling watch loop)
cargo install --locked systemfd cargo-watch    # one-time
cargo install --locked sqlx-cli --no-default-features --features rustls,postgres  # if you want CLI migrations
just run                   # systemfd + cargo watch
```

## Repository layout

```
crates/
  app/            HTTP layer: axum router, handlers, templates, auth, middleware, cache helper
  domain/         Pure types: User, Todo, Credentials, error enums (no I/O)
  storage/        sqlx pool, UserRepository (with timing-equalized verify), TodoRepository, MIGRATOR
  observability/  tracing-subscriber wiring; OTel tracer + logger providers; Prometheus recorder;
                  custom layer that activates OTel context per tracing span (otel_context_layer.rs)
templates/        MiniJinja templates (base, index, login, signup + partials)
static/
  css/            Tailwind source + compiled (gitignored)
  vendor/         htmx + alpine + alpine-compat extension, each with .gz / .br siblings
migrations/       SQL migrations applied at startup via sqlx::migrate! at the workspace root
docker/
  Dockerfile          multi-stage production build → distroless runtime
  Dockerfile.dev      full Rust toolchain for the dev override
  compose.yaml        prod-like stack (8 services)
  compose.dev.yaml    override: bind-mounted source, cargo-watch, tailwind polling loop
  otel/               OpenTelemetry Collector config (OTLP in → Tempo + Loki out)
  prometheus/         scrape config
  tempo/              Tempo 3.0 config (OTLP receivers, local storage)
  loki/               Loki 3.x config with allow_structured_metadata: true
  grafana/            datasource + dashboard provisioning
```

## Tests

```bash
just test     # cargo test --workspace
just lint     # cargo clippy --workspace --all-targets -- -D warnings
just fmt      # cargo fmt --all
```

Integration tests (`crates/storage/tests/repos.rs`, `crates/app/tests/{auth_flow,todos_flow}.rs`) use [`testcontainers`](https://crates.io/crates/testcontainers) to spin up ephemeral Postgres per test module, so Docker must be running for them.

## Smoke test

Runs end-to-end against `just up-prod`. Sign up, create todos, toggle, delete, check observability fanout, signal a graceful shutdown.

```bash
just up-prod
until curl -sf localhost:3000/readyz; do sleep 1; done

# In a browser:
#   - visit /signup → fill in → land on /
#   - add 3 todos via the form
#   - toggle one complete (the line strikes through)
#   - delete one (row fades out)
#   - refresh → state persists
#   - filter tabs (All / Open / Done) work; empty state shows when the list is empty
#   - log out → /login; log back in → state persists

# Observability
curl -s localhost:9090/api/v1/query?query=http_requests_total | jq '.data.result | length'   # > 0
curl -s "localhost:3200/api/search?tags=service.name%3Dtodo-app&start=$(($(date +%s)-300))&end=$(date +%s)" | jq '.traces | length'  # > 0
curl -sG --data-urlencode 'query={service_name="todo-app"}' --data-urlencode "start=$(($(date +%s)-300))000000000" localhost:3100/loki/api/v1/query_range | jq '.data.result | length'  # > 0

# Graceful shutdown
docker kill --signal=SIGTERM $(docker compose -f docker/compose.yaml ps -q app)
# in-flight requests complete; container exits 0 within shutdown_timeout_secs
```

## Configuration

All config is via environment variables (see `.env.example`). The two 12-factor shortcuts `DATABASE_URL` and `REDIS_URL` override the nested `APP__*` keys.

Notable knobs:

| Variable | What it does |
|---|---|
| `APP__AUTH__SESSION_KEY` | Hex- or raw-encoded ≥64 byte secret. App exits non-zero if shorter. |
| `APP__AUTH__COOKIE_SECURE` | Set to `true` behind TLS. HSTS is only emitted when this is on. |
| `APP__AUTH__COOKIE_DOMAIN` | Empty by default; set to scope cookies to a subdomain group. |
| `APP__SERVER__TRUST_FORWARDED_FOR` | When `true`, the rate limiter reads the rightmost `X-Forwarded-For` entry as the client IP. Only enable behind a trusted reverse proxy. |
| `APP__OBSERVABILITY__OTEL_ENABLED` | Master switch for OTLP traces + logs. `true` in compose; `false` for local-only runs. |
| `APP__OBSERVABILITY__LOG_FORMAT` | `pretty` (dev) or `json` (prod). |
| `APP__TEMPLATE_AUTORELOAD` | `true` in dev — switches `Templates::dev` (env rebuilt every render) vs `Templates::production` (single static env). |
| `RUST_LOG` | Standard `tracing_subscriber::EnvFilter` syntax. |

## Security posture

- Argon2id password hashing on a `spawn_blocking` worker; `UserRepository::verify` runs a dummy verify on the unknown-email path so timing doesn't leak account existence.
- Signed session cookies (≥64-byte key, `SameSite=Lax`, `HttpOnly`).
- Per-IP token-bucket rate limit on `/login` and `/signup` (5 / minute, burst 5). Counts emitted as `auth_rate_limited_total`.
- CSP `default-src 'self'`. `'unsafe-eval'` on `script-src` is required by Alpine and htmx 4's `hx-on::*` attributes; everything else (scripts, fonts, CSS) is same-origin.
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`, `Permissions-Policy: geolocation=(), microphone=(), camera=()`. HSTS only when `cookie_secure=true`.
- `/login` redirects via `?next=` are filtered through `safe_next` — only same-origin, single-leading-slash paths are honored.
- `tower_http::sensitive_headers` hides `Authorization` and `Cookie` from trace output.

## Known limitations (POC scope)

- No email verification, password reset, OAuth/OIDC.
- Single Postgres, single Valkey. Run Postgres behind PgBouncer in real deployments.
- One user per email; no multi-tenancy beyond per-user isolation.
- The `tailwind` dev-watch loop runs `find` mtime polling at 1s — fine for one engineer's laptop; not appropriate as a CI build step (use `npm run build` instead).

## Further reading

See `CLAUDE.md` for an annotated tour of the tree, deviations from the spec, and a running list of sharp edges (CSP requirements for Alpine + htmx, `--watch=always` for Tailwind in detached containers, Tempo's `?start=&end=` requirement, OTel context activator for log↔trace correlation, etc.).
