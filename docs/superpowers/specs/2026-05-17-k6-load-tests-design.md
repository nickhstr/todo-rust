# k6 Load Tests — Design Spec

**Date:** 2026-05-17
**Branch:** `feat/k6-load-tests`
**Status:** Draft, awaiting review

## Motivation

The plan that produced this codebase set a perf target of **~1000 RPS on `GET /` with cache hot, p95 < 50 ms on a developer laptop**. That target is currently unverified — there is no load-testing surface in the repo. Without one, perf regressions land silently and the target stays aspirational.

This spec introduces Grafana's `k6` as a local-first load and perf testing tool, integrated with the existing observability stack so that running a test produces both pass/fail signal (via thresholds + exit codes) and a live Grafana dashboard.

The integration is scoped to local use. CI wiring is explicitly out of scope but the design accommodates it later.

## Goals

- Run k6 scenarios locally with a single `just` command, no host-level k6 install required.
- Cover three levels of realism: an unauth smoke check, an authenticated read-heavy load test, and a full signup → CRUD user journey.
- Stream metrics live to the existing Prometheus + Grafana stack; emit a per-run JSON summary for later diffing.
- Make scenarios fail with non-zero exit on threshold violation, so the same recipes will slot into CI later without rework.
- Keep production code unchanged in behavior — the only new app branch is an explicitly-disabled-by-default rate-limit bypass for the test environment.

## Non-Goals

- No CI integration in this work.
- No `k6-diff` recipe for comparing runs across PRs (the JSON summary contains the inputs it'd need; building the diff is a follow-up).
- No xk6 extensions; vanilla k6 only.
- No load-test of `/static/*` beyond the smoke scenario.
- No host-binary execution path (`brew install k6` etc.) — compose-only.

## Decisions Recap

| # | Decision | Choice |
|---|---|---|
| 1 | Scenario scope | All three: smoke + read-heavy + journey (separate scenario files, picked per-run). |
| 2 | Rate-limit handling | Config-driven bypass (env var, default off in prod) + one login per VU. |
| 3 | Execution mode | k6 in compose, on the app network. No host install. |
| 4 | Output destination | Both: Prometheus remote-write (live dashboard) + JSON summary file per run. |
| 5 | Thresholds | Per-scenario; tight on smoke, loose-but-meaningful on read-heavy and journey. Plan's "1000 RPS / p95 < 50 ms" lives as documented *goals*, not enforced thresholds. |

## Architecture

`k6` runs as a peer service in the existing compose stack via a new `compose.k6.yaml` override. It runs on-demand (no restart policy), hits `http://app:3000` directly on the docker network, and pushes results to two destinations in parallel.

```
just k6 read_heavy
  → docker compose -f docker/compose.yaml -f docker/compose.k6.yaml run --rm k6 run \
      --out experimental-prometheus-rw=http://prometheus:9090/api/v1/write \
      --tag scenario=read_heavy --tag git_sha=<short> \
      /scripts/scenarios/read_heavy.js
  → k6 setup() seeds users via POST /signup against app:3000 (rate-limit bypass enabled)
  → k6 main loop hits target endpoints
  → metrics stream to prometheus:9090 (remote-write) AND a JSON summary
     lands in ./k6/results/
  → Grafana dashboard at :3001 shows live charts during the run
  → CLI prints summary on exit; non-zero exit if thresholds violated
```

Three independent change areas:

1. **App** — one config field + middleware skip (rate-limit bypass toggle). Off by default; only `compose.k6.yaml` sets it on.
2. **Infra** — enable `--web.enable-remote-write-receiver` on Prometheus (only in the k6 override); provision a `k6.json` Grafana dashboard.
3. **k6 surface** — new `k6/` directory at repo root with scenarios, shared helpers, and results dir; new `compose.k6.yaml`; new `just` recipes.

The boundaries stay clean: domain, storage, and observability crates are not touched. The app crate gets one config field + one middleware branch. Everything else is infra and JS test scripts.

## File Layout

```
k6/
  scenarios/
    smoke.js          # unauth endpoints; small VU count, tight thresholds
    read_heavy.js     # pre-seed users + todos, then hammer GET / (cache hot)
    journey.js        # signup → login → create → toggle → delete loop
  lib/
    config.js         # BASE_URL, scenario defaults, shared thresholds, handleSummary
    auth.js           # signup() and login() helpers; cookie-jar handling
    seed.js           # setup() helpers: bulk-create users + todos via the API
    checks.js         # status/redirect/contains assertions used across scenarios
  results/
    .gitkeep          # JSON summaries land here; the dir is gitignored except for .gitkeep
  README.md           # how to run, what each scenario does, threshold rationale

docker/
  compose.k6.yaml     # NEW: adds k6 service + prom remote-write flag + APP rate-limit-disabled env
  grafana/
    dashboards/
      k6.json         # NEW: k6 metrics dashboard

justfile              # MODIFIED: adds k6 recipes (see § Just Recipes)

.gitignore            # MODIFIED: ignore k6/results/* except .gitkeep

crates/app/src/
  config.rs           # MODIFIED: add `rate_limit.enabled: bool` config field, default true
  router.rs           # MODIFIED: pipe `cfg.rate_limit.enabled` into RateLimiter
  middleware/
    rate_limit.rs     # MODIFIED: bail early when disabled; add builder; add unit test
```

### Layout rationale

- **`k6/` at repo root, not `tests/k6/`.** It's not a Rust integration test (doesn't run under `cargo test`, doesn't compile against the crates). Keeping it sibling to `crates/` mirrors how `migrations/`, `static/`, and `templates/` already sit at root.
- **`k6/lib/` for shared JS.** All three scenarios share the same auth + seed helpers; one source of truth.
- **`k6/results/` gitignored except `.gitkeep`.** Per-run JSON files are noisy local artifacts. A checked-in baseline can be added explicitly later if a diff workflow lands.
- **`compose.k6.yaml` is its own override**, not a section of `compose.dev.yaml`. Load tests aren't part of the dev loop — they get a separate compose invocation, used only when you opt in via `just k6 ...`.

## Rate-Limit Bypass

Smallest possible change to the production path: one config field, one builder method, one early-return in middleware. Defaults preserve production behavior exactly.

### Config (`crates/app/src/config.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}
fn default_true() -> bool { true }
```

Hung off `Config` as `pub rate_limit: RateLimitConfig`, with a matching `set_default("rate_limit.enabled", true)` in `from_env()`.

### Middleware (`crates/app/src/middleware/rate_limit.rs`)

```rust
pub struct RateLimiter {
    /* existing fields */,
    enabled: bool,
}

impl RateLimiter {
    pub fn new(rate_per_sec: f64, burst: f64) -> Self {
        Self { /* existing init */, enabled: true }
    }

    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

pub async fn rate_limit_middleware(/* … */) -> Response {
    if !limiter.enabled {
        return next.run(req).await;
    }
    /* existing body unchanged */
}
```

### Router wiring (`crates/app/src/router.rs`)

The existing line:

```rust
let login_limiter = RateLimiter::new(5.0 / 60.0, 5.0)
    .trust_forwarded_for(cfg.server.trust_forwarded_for);
```

becomes:

```rust
let login_limiter = RateLimiter::new(5.0 / 60.0, 5.0)
    .trust_forwarded_for(cfg.server.trust_forwarded_for)
    .enabled(cfg.rate_limit.enabled);
```

### Compose override (`docker/compose.k6.yaml`)

Sets `APP__RATE_LIMIT__ENABLED: "false"` on the `app` service. The base `compose.yaml` is untouched; prod is unaffected.

### Notes

- The metric `auth_rate_limited_total` keeps incrementing only when the middleware *actually* rate-limits — so the metric stays prod-faithful regardless of the bypass.
- Unit test added: with `enabled(false)`, ten requests from one IP all succeed instead of one 200 + nine 429s.
- Config round-trip test added: `APP__RATE_LIMIT__ENABLED=false` deserializes, default is `true` when unset.

## Seeding Strategy

The three scenarios have different seeding needs. Helpers in `k6/lib/seed.js` handle the variations — scenarios import and call, never wiring it up manually.

### smoke.js

No seeding. Hits unauth endpoints only.

### read_heavy.js

Needs `N` pre-seeded users, each with `M` todos, in the DB before the main loop starts.

- k6's `setup()` runs once before VUs spin up. It signs up `N` users (`loadtest-u{i}@example.test` / a fixed password), logs each in once to grab a session cookie, then `POST /todos` × `M` times each.
- `setup()` returns the array of `{email, password, cookie}` records.
- Main loop pulls one record per VU (round-robin via `__VU` index) and hammers `GET /` with that cookie.
- The first run with cold cache pays the seeding cost (argon2 is ~50–100 ms/user → ~5 s for N=50). Subsequent runs against the same DB re-use the seeded users — `setup()` does an idempotent signup that swallows the "user already exists" 409 and just logs in.
- Defaults: `N=50` VUs, `M=10` todos each. Override via CLI: `-e USERS=200 -e TODOS_PER_USER=20`.

### journey.js

Each VU signs up its own user fresh; no shared seed.

- VU-scoped state: a UUID-suffixed email (`journey-{__VU}-{uuid}@example.test`) generated once per VU, so each iteration logs in as the same user across the rest of that iteration's actions.
- No cleanup. Test users accumulate until you run `just k6-clean-db` or `just nuke`. Cleanup logic in the hot loop would slow the test and obscure failures.

### Why API-driven, not direct-to-DB seeding

- Stays inside the public contract — if signup ever changes (e.g. adds an email-verify step), the load test breaks loudly and you fix it in lockstep.
- No new dependency on `sqlx` from k6.
- The signup-bulk-at-setup cost is paid once per scenario run, not per request — at N=50 it's ~5 s, acceptable.

**Trade-off acknowledged**: argon2 makes setup slower than DB-level seeding. If `N` is pushed to 1000+, revisit and add a `seed` binary that talks to Postgres directly with `password_auth::generate_hash`. Not now — premature.

### Test cleanup escape hatch

`just k6-clean-db` drops rows from `users` where email matches `loadtest-%@example.test` or `journey-%@example.test`. Idempotent.

## Scenarios

All three live under `k6/scenarios/` and import shared helpers from `k6/lib/`. Defaults below are tunable per-run via `-e VAR=value`.

### `smoke.js`

A 30-second sanity check. Confirms the stack is up, endpoints respond, no auth tripwires, and metrics flow. Run before every other scenario.

- **Load**: `constant-vus`, 1 VU, 30 s.
- **Hits**:
  - `GET /healthz` → 200
  - `GET /readyz` → 200
  - `GET /login` → 200
  - `GET /` → 303 redirect to `/login`
  - `GET /static/css/app.css` → 200, `Content-Encoding: gzip|br`, `Content-Type: text/css`
  - `GET /metrics` → 200, body contains `http_requests_total`
- **Thresholds (tight)**:
  - `http_req_failed: ['rate<0.01']`
  - `http_req_duration{endpoint:healthz}: ['p(95)<20']`
  - `checks: ['rate>0.99']`

### `read_heavy.js`

The scenario that maps to the plan's perf target.

- **Load**: `ramping-arrival-rate` — ramp 100 → 500 → 1000 req/s over 5 minutes, hold 1000 req/s for 2 minutes, ramp down over 1 minute.
- **VUs**: `preAllocatedVUs: 50`, `maxVUs: 200`. Each VU gets a stable user → cookie from `setup()`.
- **Mix**:
  - 90% `GET /` (the cache target)
  - 5% `GET /login` (cookie refresh path)
  - 5% `GET /healthz` (smoke noise for realistic mixed dashboards)
- **Setup**: per § Seeding Strategy — N=50 users × M=10 todos, idempotent.
- **Thresholds (loose-but-meaningful)**:
  - `http_req_failed: ['rate<0.01']`
  - `http_req_duration{endpoint:index}: ['p(95)<500']`
  - `checks: ['rate>0.99']`
- **Documented goals (not enforced)**: `p(95)<50` on `GET /`, sustained 1000 RPS. Live in the README + dashboard annotation for eyeballing.

### `journey.js`

A realistic user session, run by many VUs concurrently.

- **Load**: `ramping-vus`, ramp 0 → 30 VUs over 1 min, hold 5 min, ramp down 1 min.
- **Per-iteration flow**:
  1. `POST /signup` with VU-scoped email (once per VU; subsequent iterations skip and re-use cookie).
  2. `GET /` (verify empty-state template renders).
  3. `POST /todos` × 3 (with `sleep(0.2)` between — realistic typing pace).
  4. `POST /todos/{id}/toggle` on one of them.
  5. `DELETE /todos/{id}` on another.
  6. `GET /` (verify the updated list contains the expected titles).
- **Thresholds**:
  - `http_req_failed: ['rate<0.01']`
  - `http_req_duration: ['p(95)<500']`
  - `iteration_duration: ['p(95)<3000']` (sleeps included)
  - `checks: ['rate>0.99']`

### Shared via `k6/lib/config.js`

`BASE_URL = __ENV.BASE_URL || 'http://app:3000'` so the host can be overridden. Threshold defaults live here; scenarios import and merge.

## Output, Prometheus, Grafana

### Output configuration

Every run emits two streams:

1. **Prometheus remote-write** — live metrics, persisted in the existing TSDB.
   ```
   k6 run \
     --out experimental-prometheus-rw=http://prometheus:9090/api/v1/write \
     --tag scenario=read_heavy \
     --tag git_sha=$(git rev-parse --short HEAD) \
     /scripts/scenarios/read_heavy.js
   ```
   Tags become labels on every k6 metric so the dashboard can filter by scenario and trace specific runs back to commits.

2. **JSON summary file** — written by `handleSummary()` in `k6/lib/config.js` (shared across scenarios) to `/results/summary-<scenario>-<UTC-ts>-<git-sha>.json`. Volume-mounted at `./k6/results/`. Contains the full k6 summary plus a top-level `meta` block:
   ```json
   { "meta": { "git_sha": "...", "scenario": "...", "started_at": "...", "ended_at": "...", "base_url": "..." } }
   ```
   The CLI text summary still prints to stdout (k6's default).

### Prometheus change

One flag added to the `prometheus` service in `compose.k6.yaml`:

```yaml
prometheus:
  command:
    - '--config.file=/etc/prometheus/prometheus.yml'
    - '--storage.tsdb.retention.time=7d'
    - '--web.enable-remote-write-receiver'
```

Compose overrides replace lists by default — so `compose.k6.yaml` repeats all three flags. The base file in `docker/compose.yaml` is untouched; production deploys never enable the receiver. (Important: an open remote-write endpoint is a write-amplification footgun in prod.)

No change to `prometheus.yml`. The receiver is a process flag.

### Grafana dashboard (`docker/grafana/dashboards/k6.json`)

Provisioned alongside the existing `app.json`. Six k6 panels + a second row mirroring relevant app panels for correlation.

**Row 1 — k6 metrics**, all templated on `scenario` and `git_sha`:

1. **RPS** — `sum(rate(k6_http_reqs_total[30s]))` — req/s achieved vs. the scenario's target arrival rate.
2. **Latency percentiles** — `histogram_quantile(0.50|0.95|0.99, sum by (le) (rate(k6_http_req_duration_seconds_bucket[30s])))`. Plan's "p95 < 50 ms" annotated as a constant line.
3. **Error rate** — `sum(rate(k6_http_req_failed_total[30s])) / sum(rate(k6_http_reqs_total[30s]))`.
4. **Per-endpoint latency** — panel 2 grouped by an `endpoint` tag (attached at each request site, e.g. `http.get(url, { tags: { endpoint: 'index' } })`; `k6/lib/checks.js` provides a thin helper to keep tag values consistent across scenarios).
5. **VU count** — `k6_vus` — confirms ramping behaviour matched the scenario config.
6. **Checks pass rate** — `sum(rate(k6_checks_total{passed="true"}[30s])) / sum(rate(k6_checks_total[30s]))`.

**Row 2 — app's own metrics** during the run (request rate, cache hit ratio, todos counters, http duration histogram). Queries copied from `app.json` so both views sit next to each other.

`refresh: 5s`, default range `Last 15 minutes`.

## Just Recipes

```
# --- k6 load tests ---

# Bring up the stack with the k6 override (rate limit off, prom remote-write on).
k6-up:
    docker compose -f docker/compose.yaml -f docker/compose.k6.yaml up -d --build app prometheus grafana otel-collector tempo loki db cache

# Run a single scenario by name. e.g. `just k6 smoke`, `just k6 read_heavy`, `just k6 journey`.
k6 scenario: k6-up
    docker compose -f docker/compose.yaml -f docker/compose.k6.yaml run --rm k6 run \
        --out experimental-prometheus-rw=http://prometheus:9090/api/v1/write \
        --tag scenario={{scenario}} \
        --tag git_sha=$(git rev-parse --short HEAD) \
        /scripts/scenarios/{{scenario}}.js

# Convenience wrappers.
k6-smoke:      (k6 "smoke")
k6-load:       (k6 "read_heavy")
k6-journey:    (k6 "journey")

# Run all three in sequence. Stops on first failure (just halts on non-zero exit).
k6-all: k6-smoke k6-load k6-journey

# Remove load-test users from the DB. Idempotent.
k6-clean-db:
    docker compose -f docker/compose.yaml -f docker/compose.k6.yaml exec db \
        psql -U todo -d todo -c \
        "DELETE FROM users WHERE email LIKE 'loadtest-%@example.test' OR email LIKE 'journey-%@example.test';"

# Tear down the k6 stack (leaves volumes). Use `just nuke` to wipe volumes.
k6-down:
    docker compose -f docker/compose.yaml -f docker/compose.k6.yaml down
```

The chain `just k6 read_heavy` brings the stack up if needed, runs the scenario, writes the JSON summary to `./k6/results/`, and pushes metrics to Prometheus where the dashboard picks them up live.

## Validation Plan

In increasing scope:

1. **Unit test** — `crates/app/src/middleware/rate_limit.rs::disabled_bypass_allows_unlimited`. Constructs a `RateLimiter` with `enabled(false)`, exhausts the burst, asserts `check()` keeps returning true.
2. **Config round-trip test** — extend the existing config test module: `APP__RATE_LIMIT__ENABLED=false` deserializes; default is `true` when unset.
3. **`just k6-smoke`** — end-to-end manual check. Confirms compose works, k6 reaches `app:3000`, Prometheus accepts remote-write (dashboard "RPS" panel goes non-zero), JSON summary lands in `./k6/results/`.
4. **`just k6-load`** — confirms the bypass works (the existing limiter wraps both `/login` and `/signup` at 5/min/IP; without the bypass, signing up N=50 users in `setup()` would 429 after the 5th). Confirms users + todos persist. Confirms per-endpoint dashboard panel shows distinct lines for `/`, `/login`, `/healthz`.
5. **`just k6-journey`** — confirms VU-scoped state works, full session renders correctly, POST/PATCH/DELETE appear distinctly in dashboard.
6. **Threshold-violation check** — manual: temporarily set `http_req_duration: ['p(95)<1']` in smoke.js, run it, confirm k6 exits non-zero. Revert.
7. **Rate-limit prod behavior unchanged** — manual: `just up-prod` (no k6 override), curl `/login` 6× in a row, confirm the 6th returns 429.

## Out of Scope (explicit)

- No CI job. Local-only for now. The design accommodates CI later (clean exit codes, deterministic JSON summaries) but we're not wiring it.
- No `k6-diff` recipe yet. JSON summary has the inputs it'd need; building the diff is a separate task.
- No xk6 extensions. Vanilla k6 only.
- No load test of `/static/*`. ServeDir's serving is well-understood; smoke exercises it once for completeness.

## Open Questions

None at design time. Implementation may surface details — e.g., the exact pattern for matching k6's `endpoint` tag against Grafana variable templates, or whether `ramping-arrival-rate` saturates `maxVUs: 200` at 1000 RPS on a laptop — but those are tuning, not design.
