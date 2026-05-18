# k6 Load Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Grafana k6 to the local dev stack so contributors can run smoke / read-heavy / full-journey load tests against the app and see results live in the existing Grafana stack.

**Architecture:** k6 runs as a peer service in a new `compose.k6.yaml` override, hits `app:3000` on the docker network, pushes results to the existing Prometheus via `--out experimental-prometheus-rw`, and writes a per-run JSON summary to `./k6/results/`. The only app-code change is a default-off, config-driven rate-limit bypass so seeding can issue many signups in a row.

**Tech Stack:** Rust + axum (existing). Grafana k6 v0.50+ (new — runs as a docker image, no host install). Prometheus remote-write (already running, just needs `--web.enable-remote-write-receiver`). Docker compose, just, Grafana dashboard JSON.

**Spec:** `docs/superpowers/specs/2026-05-17-k6-load-tests-design.md`

---

## Task 1: Add `RateLimitConfig` to app config

**Files:**
- Modify: `crates/app/src/config.rs`

This is a pure additive change with a default that preserves production behavior (`enabled = true`).

- [ ] **Step 1: Add the `RateLimitConfig` struct + field on `Config` + default + `from_env` entry**

In `crates/app/src/config.rs`, after the `AuthConfig` block, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_rate_limit_enabled")]
    pub enabled: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn default_rate_limit_enabled() -> bool { true }
```

Add a field to `Config`:

```rust
pub struct Config {
    pub server: ServerConfig,
    pub database: DbPoolConfig,
    pub cache: CacheConfig,
    pub auth: AuthConfig,
    pub observability: ObservabilityConfig,
    pub templates_dir: PathBuf,
    pub static_dir: PathBuf,
    #[serde(default = "default_template_autoreload")]
    pub template_autoreload: bool,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,   // NEW
}
```

Add to `Config::default()`:

```rust
rate_limit: RateLimitConfig::default(),
```

Add to `from_env()`'s `set_default` chain (immediately before the `// 12-factor shortcuts` comment):

```rust
.set_default("rate_limit.enabled", true)?
```

- [ ] **Step 2: Write the unit test**

In the existing `#[cfg(test)] mod tests` block of `crates/app/src/config.rs`, append:

```rust
#[test]
fn rate_limit_default_is_enabled() {
    let cfg = Config::default();
    assert!(cfg.rate_limit.enabled);
}

#[test]
fn rate_limit_config_serde_roundtrip() {
    // The field comes through env as APP__RATE_LIMIT__ENABLED, which the
    // `config` crate maps to `{rate_limit: {enabled: ...}}`. We exercise the
    // serde shape here so future renames break loudly.
    let disabled: RateLimitConfig =
        serde_json::from_str(r#"{"enabled": false}"#).unwrap();
    assert!(!disabled.enabled);
    let defaulted: RateLimitConfig = serde_json::from_str("{}").unwrap();
    assert!(defaulted.enabled);
}
```

- [ ] **Step 3: Run the tests**

```bash
cargo test -p todo-app --lib config:: -- --nocapture
```

Expected: both new tests PASS plus the existing session_key tests.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/config.rs
git commit -m "config: add RateLimitConfig field, default enabled

Adds a per-config knob so tests/load runs can disable the rate limiter
without touching the middleware. Defaults to true so production is
unaffected.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Add `enabled` toggle to RateLimiter + middleware short-circuit

**Files:**
- Modify: `crates/app/src/middleware/rate_limit.rs`

The bypass lives in the middleware function, not in `check()` — so the metric `auth_rate_limited_total` only ever increments when the limiter is actually limiting.

- [ ] **Step 1: Write the failing middleware test**

Append to the existing `#[cfg(test)] mod tests` at the bottom of `crates/app/src/middleware/rate_limit.rs`:

```rust
#[tokio::test]
async fn disabled_bypass_lets_all_requests_through() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware::from_fn_with_state,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    // Burst=1 and rate=0: without bypass, request 2+ would 429.
    let limiter = RateLimiter::new(0.0, 1.0).enabled(false);
    let app = Router::new()
        .route("/x", get(|| async { "ok" }))
        .route_layer(from_fn_with_state(
            limiter,
            super::rate_limit_middleware,
        ));

    for _ in 0..5 {
        let req = Request::builder().uri("/x").body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn enabled_limiter_still_429s_when_bucket_empty() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware::from_fn_with_state,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    // Default `enabled=true`; burst=1 so 2nd request must 429.
    let limiter = RateLimiter::new(0.0, 1.0);
    let app = Router::new()
        .route("/x", get(|| async { "ok" }))
        .route_layer(from_fn_with_state(
            limiter,
            super::rate_limit_middleware,
        ));

    let req = Request::builder().uri("/x").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let req = Request::builder().uri("/x").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}
```

- [ ] **Step 2: Run the tests; expect failure**

```bash
cargo test -p todo-app --lib middleware::rate_limit:: 2>&1 | tail -30
```

Expected: `disabled_bypass_lets_all_requests_through` fails to compile (`enabled` method does not exist) — or after we add the method but before the early-return, `disabled_bypass_lets_all_requests_through` fails because the 2nd request still 429s.

- [ ] **Step 3: Implement the bypass**

In `crates/app/src/middleware/rate_limit.rs`:

Add `enabled: bool` to the struct:

```rust
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    rate_per_sec: f64,
    burst: f64,
    pub(crate) trust_forwarded_for: bool,
    enabled: bool,   // NEW
}
```

Initialize it in `new()`:

```rust
impl RateLimiter {
    pub fn new(rate_per_sec: f64, burst: f64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            rate_per_sec,
            burst,
            trust_forwarded_for: false,
            enabled: true,   // NEW
        }
    }
```

Add the builder right after `trust_forwarded_for`:

```rust
    /// When set to `false`, every request bypasses the limiter. Off for prod;
    /// on for load tests (where one source IP fires thousands of requests).
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
```

Add the early return as the first thing in `rate_limit_middleware`:

```rust
pub async fn rate_limit_middleware(
    State(limiter): State<RateLimiter>,
    conn: Option<ConnectInfo<std::net::SocketAddr>>,
    req: Request,
    next: Next,
) -> Response {
    if !limiter.enabled {
        return next.run(req).await;
    }
    let peer_ip = conn
        .map(|c| c.0.ip())
        // … rest unchanged
```

- [ ] **Step 4: Run the tests; expect pass**

```bash
cargo test -p todo-app --lib middleware::rate_limit::
```

Expected: all 7 tests pass (5 existing + 2 new).

- [ ] **Step 5: Run clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/middleware/rate_limit.rs
git commit -m "middleware: add enabled() toggle on RateLimiter

Off-by-default toggle (i.e., default remains enabled=true) that
short-circuits the middleware before any bucket lookup or metric
increment. Used by the k6 load tests to avoid 429s on bulk signup.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Wire `cfg.rate_limit.enabled` into the router

**Files:**
- Modify: `crates/app/src/router.rs`

- [ ] **Step 1: Plumb the config field into the RateLimiter construction**

In `crates/app/src/router.rs`, find the existing block:

```rust
let login_limiter = RateLimiter::new(5.0 / 60.0, 5.0)
    .trust_forwarded_for(state.config.server.trust_forwarded_for);
```

Replace with:

```rust
let login_limiter = RateLimiter::new(5.0 / 60.0, 5.0)
    .trust_forwarded_for(state.config.server.trust_forwarded_for)
    .enabled(state.config.rate_limit.enabled);
```

- [ ] **Step 2: Run the existing integration tests to confirm nothing broke**

```bash
cargo test --workspace
```

Expected: all 21 existing tests pass + the 2 new unit tests from Task 2 = 23 passing.

- [ ] **Step 3: Run clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/router.rs
git commit -m "router: pipe rate_limit.enabled config into the login limiter

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Scaffold the `k6/` directory

**Files:**
- Create: `k6/scenarios/.gitkeep`
- Create: `k6/lib/.gitkeep`
- Create: `k6/results/.gitkeep`
- Create: `k6/README.md` (stub — filled out in Task 12)
- Modify: `.gitignore`

- [ ] **Step 1: Create directories and placeholder files**

```bash
mkdir -p k6/scenarios k6/lib k6/results
touch k6/scenarios/.gitkeep k6/lib/.gitkeep k6/results/.gitkeep
```

- [ ] **Step 2: Write the README stub**

`k6/README.md`:

```markdown
# k6 load tests

This directory holds Grafana k6 scenarios + helpers for load and perf
testing the app locally.

See `docs/superpowers/specs/2026-05-17-k6-load-tests-design.md` for the
design. Run `just k6-smoke`, `just k6-load`, or `just k6-journey` to
execute a scenario; see the justfile for the full set.

Filled out in Task 12 of the implementation plan.
```

- [ ] **Step 3: Append to `.gitignore`**

Append the following block at the end of `.gitignore` (the existing file already has its own sections — the comment header keeps this block findable):

```
# k6 load test results — local per-run artifacts, large and noisy.
k6/results/*
!k6/results/.gitkeep
```

- [ ] **Step 4: Verify `.gitkeep` files are not ignored**

```bash
git check-ignore -v k6/results/.gitkeep
```

Expected: exits 1 with no output (file is NOT ignored). If it prints a matching pattern, the negation `!k6/results/.gitkeep` is wrong — adjust.

- [ ] **Step 5: Commit**

```bash
git add k6/ .gitignore
git commit -m "k6: scaffold directory structure

Adds k6/{scenarios,lib,results}/ with placeholder .gitkeep files,
a README stub, and gitignores k6/results/ contents.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Create k6 shared lib

**Files:**
- Create: `k6/lib/config.js`
- Create: `k6/lib/checks.js`
- Create: `k6/lib/auth.js`
- Create: `k6/lib/seed.js`

The four files form one cohesive layer (config + assertions + auth + seeding). They are imported by every scenario. None of them is runnable on its own; they're tested by the scenarios in Tasks 8–10.

- [ ] **Step 1: Write `k6/lib/config.js`**

```javascript
// Shared k6 configuration: base URL, default thresholds, summary writer.
//
// BASE_URL defaults to the docker-internal app hostname; override via
// `-e BASE_URL=http://localhost:3000` when running k6 against a host
// binary instead of the in-compose service.

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.2/index.js';

export const BASE_URL = __ENV.BASE_URL || 'http://app:3000';

export const SHARED_THRESHOLDS = {
  http_req_failed: ['rate<0.01'],
  checks: ['rate>0.99'],
};

// Build a `handleSummary` that writes the full k6 summary, plus a small
// meta block (git sha + scenario name + base URL), to a JSON file under
// /results/. Path is timestamped + sha-suffixed so concurrent or repeat
// runs don't overwrite each other.
export function makeSummaryWriter(scenario) {
  return function handleSummary(data) {
    const ts = new Date().toISOString().replace(/[:.]/g, '-');
    const gitSha = __ENV.GIT_SHA || 'unknown';
    const meta = {
      scenario,
      git_sha: gitSha,
      base_url: BASE_URL,
      finished_at: new Date().toISOString(),
      test_run_duration_ms: data.state ? data.state.testRunDurationMs : null,
    };
    return {
      stdout: textSummary(data, { indent: ' ', enableColors: true }),
      [`/results/summary-${scenario}-${ts}-${gitSha}.json`]:
        JSON.stringify({ meta, ...data }, null, 2),
    };
  };
}
```

- [ ] **Step 2: Write `k6/lib/checks.js`**

```javascript
// Shared assertion helpers. Each scenario imports these instead of
// calling `check()` directly, so endpoint tagging stays consistent and
// the Grafana dashboard's per-endpoint panel finds the labels it wants.

import { check } from 'k6';

// Endpoint names — kept as a const so a typo at the call site doesn't
// silently create a new metric label.
export const Endpoint = {
  Healthz: 'healthz',
  Readyz: 'readyz',
  Index: 'index',
  Login: 'login',
  Signup: 'signup',
  TodoCreate: 'todo-create',
  TodoToggle: 'todo-toggle',
  TodoDelete: 'todo-delete',
  StaticCss: 'static-css',
  Metrics: 'metrics',
};

// Convenience: build a `params` object with the endpoint tag baked in.
//
//   http.get(url, params(Endpoint.Index, { redirects: 0 }))
//
export function params(endpoint, extra = {}) {
  return { ...extra, tags: { endpoint, ...(extra.tags || {}) } };
}

export function assertStatus(res, expected, label) {
  return check(res, {
    [`${label} status==${expected}`]: (r) => r.status === expected,
  });
}

export function assertBodyContains(res, needle, label) {
  return check(res, {
    [`${label} body contains "${needle}"`]: (r) =>
      typeof r.body === 'string' && r.body.includes(needle),
  });
}

export function assertHeader(res, header, expected, label) {
  return check(res, {
    [`${label} header ${header}==${expected}`]: (r) =>
      (r.headers[header] || r.headers[header.toLowerCase()]) === expected,
  });
}
```

- [ ] **Step 3: Write `k6/lib/auth.js`**

```javascript
// Auth helpers — signup + login. Each VU gets its own automatic cookie
// jar, so subsequent requests in the same iteration use the session
// transparently.

import http from 'k6/http';
import { BASE_URL } from './config.js';
import { params, Endpoint } from './checks.js';

// Tell k6 that 200-399 (normal success / redirect) AND 409 (user already
// exists) are both "expected" responses for signup. Without this, idempotent
// reruns of seedUsersWithTodos() would push every signup into
// http_req_failed_rate, pegging the Error rate dashboard panel at 100%
// during setup.
const SIGNUP_EXPECTED = http.expectedStatuses({ min: 200, max: 399 }, 409);

// POST /signup. Returns the response. Treats 409 (user already exists)
// as success for idempotent seeding.
export function signup(email, password) {
  const res = http.post(
    `${BASE_URL}/signup`,
    { email, password },
    params(Endpoint.Signup, { redirects: 0, responseCallback: SIGNUP_EXPECTED }),
  );
  if (res.status !== 303 && res.status !== 409) {
    throw new Error(
      `signup ${email} unexpected status ${res.status} body=${(res.body || '').slice(0, 200)}`,
    );
  }
  return res;
}

// POST /login. Returns the response. Throws on anything other than 303.
export function login(email, password) {
  const res = http.post(
    `${BASE_URL}/login`,
    { email, password },
    params(Endpoint.Login, { redirects: 0 }),
  );
  if (res.status !== 303) {
    throw new Error(
      `login ${email} unexpected status ${res.status} body=${(res.body || '').slice(0, 200)}`,
    );
  }
  return res;
}
```

- [ ] **Step 4: Write `k6/lib/seed.js`**

```javascript
// Bulk seeding for the read-heavy scenario. Called from setup().
//
// Idempotent: signups that hit 409 (user already exists) are treated
// as success — todos are NOT re-created in that case, so reruns are
// fast (~ms per existing user, no DB writes).

import http from 'k6/http';
import { BASE_URL } from './config.js';
import { signup } from './auth.js';
import { params, Endpoint } from './checks.js';

// Seeds N users, each with `todosPerUser` todos. Returns an array of
// `{ email, password }` records for the VU function to log in with.
export function seedUsersWithTodos(n, todosPerUser) {
  const users = [];
  const password = 'k6-load-test-password';

  for (let i = 1; i <= n; i++) {
    const email = `loadtest-u${i}@example.test`;
    const res = signup(email, password);
    const isNewUser = res.status === 303;

    if (isNewUser) {
      // Signup auto-logs in via Set-Cookie on the 303; the cookie jar
      // captures it and subsequent posts here are authenticated.
      for (let j = 1; j <= todosPerUser; j++) {
        const todoRes = http.post(
          `${BASE_URL}/todos`,
          { title: `task-${i}-${j}` },
          params(Endpoint.TodoCreate, { redirects: 0 }),
        );
        // POST /todos returns 201 on htmx-style success.
        if (todoRes.status !== 201 && todoRes.status !== 200) {
          throw new Error(
            `seed todo ${i}/${j} unexpected status ${todoRes.status}`,
          );
        }
      }
    }
    users.push({ email, password });
  }

  return users;
}
```

- [ ] **Step 5: Commit**

```bash
git add k6/lib/
git commit -m "k6: add shared lib (config, checks, auth, seed)

Imported by all scenarios. No runtime verification at this step —
exercised by the scenarios in subsequent commits.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Create `docker/compose.k6.yaml`

**Files:**
- Create: `docker/compose.k6.yaml`

This override is loaded only when running k6 (`just k6 …`). It (a) enables the Prometheus remote-write receiver, (b) disables the app's rate limiter, and (c) adds the k6 service.

- [ ] **Step 1: Write the override**

```yaml
# docker/compose.k6.yaml
#
# Loaded ONLY for k6 runs (`just k6-smoke` / `just k6-load` / `just k6-journey`).
# Adds the k6 service, enables Prometheus remote-write, and disables the
# app's per-IP rate limiter so seeding can issue many signups in a row.

services:
  app:
    environment:
      # Disable rate limiting only when this override is active. Default
      # production behavior (limiter on) is preserved by the base compose.
      APP__RATE_LIMIT__ENABLED: "false"

  prometheus:
    # Compose merges lists by replacement, so the full command list is
    # restated here.
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.retention.time=7d'
      - '--web.enable-remote-write-receiver'

  k6:
    image: grafana/k6:0.50.0
    profiles: ["k6"]   # not started by `docker compose up`; only on `run --rm k6`
    networks:
      - default
    environment:
      # Emit trend metrics as gauges with these stats. Without this,
      # Grafana panels can't easily compute percentiles via
      # histogram_quantile() on remote-write data.
      K6_PROMETHEUS_RW_TREND_STATS: "avg,p(50),p(95),p(99),max"
      # k6 expects scripts at /scripts, results at /results.
      K6_NO_USAGE_REPORT: "true"
    volumes:
      - ../k6/scenarios:/scripts/scenarios:ro
      - ../k6/lib:/scripts/lib:ro
      - ../k6/results:/results
    depends_on:
      app: { condition: service_started }
      prometheus: { condition: service_started }
```

- [ ] **Step 2: Validate the compose config**

```bash
docker compose -f docker/compose.yaml -f docker/compose.k6.yaml config > /dev/null && echo OK
```

Expected: `OK`. If validation fails, fix the YAML errors and retry.

- [ ] **Step 3: Confirm the k6 service is only started on-demand**

```bash
docker compose -f docker/compose.yaml -f docker/compose.k6.yaml config --services
```

Expected: lists every service. The `profiles: ["k6"]` means `up` won't start it; `run --rm k6` will.

- [ ] **Step 4: Commit**

```bash
git add docker/compose.k6.yaml
git commit -m "docker: add compose.k6.yaml override

Loaded only for k6 runs. Enables Prometheus remote-write, disables the
app rate limiter, and adds the k6 service (profile-gated).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Add justfile k6 recipes

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Append recipes**

Append the following at the end of `justfile`:

```make
# --- k6 load tests ---

# Bring up the stack with the k6 override (rate limit off, prom remote-write on).
# Idempotent: re-runs hit cached layers.
k6-up:
    docker compose -f docker/compose.yaml -f docker/compose.k6.yaml up -d --build app prometheus grafana otel-collector tempo loki db cache

# Run a single scenario by name. e.g. `just k6 smoke`, `just k6 read_heavy`, `just k6 journey`.
k6 scenario: k6-up
    docker compose -f docker/compose.yaml -f docker/compose.k6.yaml run --rm \
        -e GIT_SHA=$(git rev-parse --short HEAD) \
        k6 run \
        --out experimental-prometheus-rw=http://prometheus:9090/api/v1/write \
        --tag scenario={{scenario}} \
        --tag git_sha=$(git rev-parse --short HEAD) \
        /scripts/scenarios/{{scenario}}.js

# Convenience wrappers.
k6-smoke: (k6 "smoke")
k6-load: (k6 "read_heavy")
k6-journey: (k6 "journey")

# Run all three in sequence. just halts on non-zero exit, so a threshold
# violation on smoke stops before load runs.
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

- [ ] **Step 2: Confirm just discovers the recipes**

```bash
just --list | grep -E "^\s+k6"
```

Expected output (order may vary):

```
    k6 scenario     # Run a single scenario by name. ...
    k6-all          # Run all three in sequence. ...
    k6-clean-db     # Remove load-test users from the DB. ...
    k6-down         # Tear down the k6 stack ...
    k6-journey      # 
    k6-load         # 
    k6-smoke        # 
    k6-up           # Bring up the stack ...
```

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "just: add k6 recipes (up/down/smoke/load/journey/all/clean-db)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Create + verify the smoke scenario

**Files:**
- Create: `k6/scenarios/smoke.js`

Smoke is the first scenario because it has no auth, no seeding, and no rate-limit dependency — running it end-to-end verifies that the compose override, the k6 service, the Prometheus remote-write path, and the JSON summary writer are all working.

- [ ] **Step 1: Write `k6/scenarios/smoke.js`**

```javascript
// smoke.js — 30s sanity check that exercises unauth endpoints + the
// static asset pipeline. Run before every other scenario.

import http from 'k6/http';
import { sleep } from 'k6';
import { BASE_URL, SHARED_THRESHOLDS, makeSummaryWriter } from '../lib/config.js';
import {
  Endpoint,
  params,
  assertStatus,
  assertBodyContains,
} from '../lib/checks.js';

export const options = {
  scenarios: {
    smoke: {
      executor: 'constant-vus',
      vus: 1,
      duration: '30s',
    },
  },
  thresholds: {
    ...SHARED_THRESHOLDS,
    'http_req_duration{endpoint:healthz}': ['p(95)<20'],
  },
};

export default function () {
  let r;

  r = http.get(`${BASE_URL}/healthz`, params(Endpoint.Healthz));
  assertStatus(r, 200, 'healthz');

  r = http.get(`${BASE_URL}/readyz`, params(Endpoint.Readyz));
  assertStatus(r, 200, 'readyz');

  r = http.get(`${BASE_URL}/login`, params(Endpoint.Login));
  assertStatus(r, 200, 'GET /login');

  r = http.get(`${BASE_URL}/`, params(Endpoint.Index, { redirects: 0 }));
  assertStatus(r, 303, 'GET / unauth -> 303');

  r = http.get(`${BASE_URL}/static/css/app.css`, params(Endpoint.StaticCss));
  assertStatus(r, 200, 'static css');

  r = http.get(`${BASE_URL}/metrics`, params(Endpoint.Metrics));
  assertStatus(r, 200, 'metrics');
  assertBodyContains(r, 'http_requests_total', 'metrics body');

  sleep(0.2);
}

export const handleSummary = makeSummaryWriter('smoke');
```

- [ ] **Step 2: Run the smoke scenario**

```bash
just k6-smoke
```

Expected: k6 prints its standard summary at the end, including `checks........: 100.00%`, and exits 0. If you see `429` in the output, the rate-limit bypass didn't take effect — check that `APP__RATE_LIMIT__ENABLED=false` made it into the app container (`docker compose -f docker/compose.yaml -f docker/compose.k6.yaml exec app env | grep RATE_LIMIT`).

- [ ] **Step 3: Verify the JSON summary file was written**

```bash
ls -la k6/results/
```

Expected: at least one `summary-smoke-*.json` file. Inspect with `jq`:

```bash
jq '.meta' k6/results/summary-smoke-*.json | head
```

Expected: a `meta` object with `scenario: "smoke"`, the git SHA, and the base URL.

- [ ] **Step 4: Verify Prometheus received k6 metrics**

```bash
curl -sG --data-urlencode 'match[]={__name__=~"k6_.*"}' \
    http://localhost:9090/api/v1/series | jq '.data | length'
```

Expected: a positive integer (k6 metrics were ingested). If `0`, the remote-write receiver flag didn't take effect — check `docker compose -f docker/compose.yaml -f docker/compose.k6.yaml exec prometheus cat /etc/prometheus/prometheus.yml` and confirm the override's `command:` list applied.

- [ ] **Step 5: Commit**

```bash
git add k6/scenarios/smoke.js
git commit -m "k6: add smoke scenario

30s sanity check on unauth endpoints + static asset pipeline + /metrics.
Tight thresholds (p95<20ms on healthz) — it's a heartbeat, not a load test.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Create + verify the read-heavy scenario

**Files:**
- Create: `k6/scenarios/read_heavy.js`

This is the scenario that maps to the plan's perf target (1000 RPS on `GET /` with cache hot, p95 < 50 ms).

- [ ] **Step 1: Write `k6/scenarios/read_heavy.js`**

```javascript
// read_heavy.js — ramps to 1000 req/s on GET / with cache warm.
//
// setup() bulk-seeds users + todos via the API (idempotent).
// Each VU is assigned one seed user, logs in on iteration 0, then loops.
//
// Note on cookies: k6 resets the default per-VU cookie jar at the end of
// every iteration. To keep the session across iterations we instantiate
// a long-lived `http.CookieJar()` at module scope (init context, once
// per VU) and pass it as `{ jar }` on every request.

import http from 'k6/http';
import { sleep } from 'k6';
import { BASE_URL, SHARED_THRESHOLDS, makeSummaryWriter } from '../lib/config.js';
import { Endpoint, params, assertStatus } from '../lib/checks.js';
import { seedUsersWithTodos } from '../lib/seed.js';
import { login } from '../lib/auth.js';

const USERS = parseInt(__ENV.USERS || '50', 10);
const TODOS_PER_USER = parseInt(__ENV.TODOS_PER_USER || '10', 10);

// Per-VU module-scope jar. Survives the per-iteration VU reset that
// would otherwise clear the default jar.
const jar = new http.CookieJar();

export const options = {
  scenarios: {
    read_heavy: {
      executor: 'ramping-arrival-rate',
      startRate: 50,
      timeUnit: '1s',
      preAllocatedVUs: 50,
      maxVUs: 200,
      stages: [
        { duration: '1m', target: 100 },
        { duration: '2m', target: 500 },
        { duration: '2m', target: 1000 },
        { duration: '2m', target: 1000 },
        { duration: '1m', target: 0 },
      ],
    },
  },
  thresholds: {
    ...SHARED_THRESHOLDS,
    'http_req_duration{endpoint:index}': ['p(95)<500'],
  },
};

export function setup() {
  return { users: seedUsersWithTodos(USERS, TODOS_PER_USER) };
}

export default function (data) {
  // On the first iteration each VU logs in as its assigned user, pumping
  // the session cookie into the module-scope jar. Subsequent iterations
  // reuse it.
  if (__ITER === 0) {
    const user = data.users[(__VU - 1) % data.users.length];
    login(user.email, user.password, jar);
  }

  const roll = Math.random();
  let r;
  if (roll < 0.9) {
    r = http.get(`${BASE_URL}/`, params(Endpoint.Index, { redirects: 0, jar }));
    assertStatus(r, 200, 'GET /');
  } else if (roll < 0.95) {
    r = http.get(`${BASE_URL}/login`, params(Endpoint.Login, { jar }));
    assertStatus(r, 200, 'GET /login');
  } else {
    r = http.get(`${BASE_URL}/healthz`, params(Endpoint.Healthz, { jar }));
    assertStatus(r, 200, 'healthz');
  }
}

export const handleSummary = makeSummaryWriter('read_heavy');
```

Note: `login()` in `k6/lib/auth.js` needs to accept an optional `jar` arg for this to work. Update the signature to `login(email, password, jar = null)` and, when `jar` is provided, pass it through to `http.post` along with `redirects: 0`. `signup()` does not need this change since seeding runs in `setup()` where the default jar is fine.

- [ ] **Step 2: Run the read-heavy scenario**

```bash
just k6-load
```

Expected: ~8 minutes total. The k6 summary should show:

- `iterations` count in the high tens-of-thousands range (ramping to 1000/s for 2 min hold ≈ 120k iterations during peak alone)
- `http_req_failed: rate < 0.01`
- `http_req_duration{endpoint:index} p(95) < 500ms` (the threshold). The *aspirational* `p95 < 50ms` is not enforced and may or may not hold on a laptop — that's expected; the spec calls it out.

If k6 errors with `setup() timeout`, the seeding is taking longer than the default 1-minute setup timeout. Add `setupTimeout: '5m'` to `options` (this can happen on the first run, when argon2 is hashing 50 fresh passwords).

- [ ] **Step 3: Verify users + todos persisted**

```bash
docker compose -f docker/compose.yaml -f docker/compose.k6.yaml exec db \
    psql -U todo -d todo -c \
    "SELECT count(*) FROM users WHERE email LIKE 'loadtest-%';"
```

Expected: `50` (or whatever USERS was set to).

```bash
docker compose -f docker/compose.yaml -f docker/compose.k6.yaml exec db \
    psql -U todo -d todo -c \
    "SELECT count(*) FROM todos t JOIN users u ON u.id = t.owner_id WHERE u.email LIKE 'loadtest-%';"
```

Expected: `500` (USERS × TODOS_PER_USER).

- [ ] **Step 4: Verify per-endpoint metrics exist in Prometheus**

```bash
curl -sG --data-urlencode 'match[]={__name__=~"k6_http_req_duration.*", endpoint="index"}' \
    http://localhost:9090/api/v1/series | jq '.data | length'
```

Expected: a positive integer. If `0`, the `endpoint` tag from `checks.js` is not making it into Prometheus — verify with the per-tag query and adjust.

- [ ] **Step 5: Commit**

```bash
git add k6/scenarios/read_heavy.js
git commit -m "k6: add read-heavy scenario

Ramping arrival rate up to 1000 req/s on GET /. Pre-seeds USERS users
× TODOS_PER_USER todos via the API (idempotent). Each VU logs in once
on iteration 0; the cookie jar carries the session for the rest of the
test. Threshold: p95 < 500ms (the 1000 RPS + 50ms target is documented,
not enforced).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: Create + verify the journey scenario

**Files:**
- Create: `k6/scenarios/journey.js`

- [ ] **Step 1: Write `k6/scenarios/journey.js`**

```javascript
// journey.js — realistic user session: signup → CRUD → list.
// Each VU runs as its own user, signs up on iteration 0, then loops.

import http from 'k6/http';
import { sleep } from 'k6';
import { BASE_URL, SHARED_THRESHOLDS, makeSummaryWriter } from '../lib/config.js';
import {
  Endpoint,
  params,
  assertStatus,
  assertBodyContains,
} from '../lib/checks.js';
import { signup, login } from '../lib/auth.js';

export const options = {
  scenarios: {
    journey: {
      executor: 'ramping-vus',
      startVUs: 0,
      stages: [
        { duration: '1m', target: 30 },
        { duration: '5m', target: 30 },
        { duration: '1m', target: 0 },
      ],
    },
  },
  thresholds: {
    ...SHARED_THRESHOLDS,
    http_req_duration: ['p(95)<500'],
    iteration_duration: ['p(95)<3000'],
  },
};

// VU-scoped state: a unique email per VU, generated once.
// (k6 reinitializes the module per-VU, so module-level `let` is
// VU-scoped — it does NOT bleed across VUs.)
const vuEmail = `journey-vu${__VU}-${Date.now()}@example.test`;
const vuPassword = 'k6-journey-password';

export default function () {
  let r;

  // Signup on first iteration; subsequent iterations are already logged in.
  if (__ITER === 0) {
    signup(vuEmail, vuPassword);
  } else {
    // Re-login defensively if the cookie jar lost the session (e.g., on
    // long runs near session expiry). login() throws on non-303.
    // Skip on iteration 0 — signup auto-logs in.
    login(vuEmail, vuPassword);
  }

  // Step 1: list (verify empty state on first iter, populated later)
  r = http.get(`${BASE_URL}/`, params(Endpoint.Index));
  assertStatus(r, 200, 'GET / step 1');

  // Step 2: create three todos, with a realistic typing pause between
  const titles = [`todo-${__VU}-${__ITER}-a`, `todo-${__VU}-${__ITER}-b`, `todo-${__VU}-${__ITER}-c`];
  const createdIds = [];
  for (const title of titles) {
    r = http.post(
      `${BASE_URL}/todos`,
      { title },
      params(Endpoint.TodoCreate, { redirects: 0 }),
    );
    if (r.status !== 201 && r.status !== 200) {
      throw new Error(`POST /todos status=${r.status}`);
    }
    // POST /todos returns the new <li> partial; the id is embedded in
    // hx-target / data-todo-id. We don't strictly need it for the
    // toggle/delete steps below — we re-list to grab ids.
  }

  // Step 3: list again, grab ids from the response body
  r = http.get(`${BASE_URL}/`, params(Endpoint.Index));
  assertStatus(r, 200, 'GET / step 3');
  assertBodyContains(r, titles[0], 'step 3 contains created todo');

  // The todo.html template renders each <li id="todo-<uuid>">, so we extract
  // the UUID from that attribute.
  const ids = [...(r.body || '').matchAll(/id="todo-([0-9a-f-]{36})"/g)].map((m) => m[1]);
  if (ids.length >= 2) {
    // Step 4: toggle one
    r = http.post(
      `${BASE_URL}/todos/${ids[0]}/toggle`,
      null,
      params(Endpoint.TodoToggle, { redirects: 0 }),
    );
    assertStatus(r, 200, 'toggle');

    // Step 5: delete another
    r = http.del(
      `${BASE_URL}/todos/${ids[1]}`,
      null,
      params(Endpoint.TodoDelete, { redirects: 0 }),
    );
    assertStatus(r, 200, 'delete');
  }

  // Step 6: final list
  r = http.get(`${BASE_URL}/`, params(Endpoint.Index));
  assertStatus(r, 200, 'GET / step 6');

  sleep(0.2);
}

export const handleSummary = makeSummaryWriter('journey');
```

- [ ] **Step 2: Run the journey scenario**

```bash
just k6-journey
```

Expected: ~7 minutes total. Summary shows positive iteration counts and `checks rate > 99%`. If the `data-todo-id` regex finds nothing (k6 logs warn-level messages but the test passes the assertStatus), check the template — the attribute may use a different name. Inspect with:

```bash
curl -s -c /tmp/jar -b /tmp/jar -d 'email=spy@example.test&password=spypassword' \
    -L http://localhost:3000/signup
curl -s -b /tmp/jar -d 'title=hello' http://localhost:3000/todos
curl -s -b /tmp/jar http://localhost:3000/ | grep -o 'data-todo-id="[^"]*"' | head
```

If the attribute is named differently in the template (e.g. `data-id`), update the regex in `journey.js` accordingly.

- [ ] **Step 3: Verify VU-scoped users persisted**

```bash
docker compose -f docker/compose.yaml -f docker/compose.k6.yaml exec db \
    psql -U todo -d todo -c \
    "SELECT count(*) FROM users WHERE email LIKE 'journey-%';"
```

Expected: a positive integer (one per VU that signed up — should be 30 if ramp completed fully, less if k6 was interrupted).

- [ ] **Step 4: Commit**

```bash
git add k6/scenarios/journey.js
git commit -m "k6: add journey scenario

30 VUs ramping over 7 minutes. Each VU runs the full session loop:
signup → list → create×3 → toggle → delete → list.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: Create the Grafana k6 dashboard

**Files:**
- Create: `docker/grafana/dashboards/k6.json`

The dashboard is provisioned alongside the existing `app.json`. Grafana picks it up on next start (or sidecar reload).

The exact k6 metric names depend on k6's prometheus-rw output and vary across versions. We use k6 v0.53.0 (bumped from v0.50 during implementation for native ESM support). With `K6_PROMETHEUS_RW_TREND_STATS=avg,p(50),p(95),p(99),max` (set in `compose.k6.yaml`), v0.53 emits trends as separate metrics with the stat baked into the name suffix:

- counters → `k6_http_reqs_total`, `k6_data_received_total`
- rates → `k6_http_req_failed_rate`, `k6_checks_rate` (already a value in [0, 1])
- trends → `k6_http_req_duration_p95`, `k6_http_req_duration_p99`, etc. (one metric per stat)
- gauges → `k6_vus`

The labels (`scenario`, `git_sha`, `endpoint`, `method`, `status`, `url`) propagate to every series, so dashboard templating works as designed.

**Trend units are seconds.** k6's prometheus-rw output emits duration values in seconds, not milliseconds — even though the CLI summary formats them as ms. So the latency panels must use `"unit": "s"` (Grafana auto-formats: `0.036` displays as `36 ms`). Using `"unit": "ms"` would label a seconds value as ms, showing 0.036 ms when the truth is 36 ms.

**Aggregate to control series fan-out.** k6 attaches a rich label set to every series (`endpoint`, `method`, `status`, `url`, `name`, `expected_response`, `proto`). Each unique combination becomes its own Prometheus series — easily a hundred+ for a real run. Naked queries like `k6_http_req_failed_rate{...}` render the legend unreadable. Each panel must aggregate to its intent:

- Overall latency panel → `max(k6_http_req_duration_pXX{...})` for one line per percentile.
- Overall error rate / checks pass rate → `max(...)` / `min(...)` for one line.
- Per-endpoint p95 → `max by (endpoint) (...)` for one line per endpoint.
- VUs gauge → `max(k6_vus{...})` because k6 emits one VUs series per executor / stage.

- [ ] **Step 1: Write the dashboard JSON**

```json
{
  "title": "k6 load tests",
  "uid": "k6-load-tests",
  "schemaVersion": 38,
  "refresh": "5s",
  "time": { "from": "now-15m", "to": "now" },
  "templating": {
    "list": [
      {
        "name": "scenario",
        "type": "query",
        "datasource": { "type": "prometheus", "uid": "prometheus" },
        "query": "label_values(k6_http_reqs_total, scenario)",
        "refresh": 2,
        "includeAll": true,
        "current": { "text": "All", "value": "$__all" }
      },
      {
        "name": "git_sha",
        "type": "query",
        "datasource": { "type": "prometheus", "uid": "prometheus" },
        "query": "label_values(k6_http_reqs_total{scenario=~\"$scenario\"}, git_sha)",
        "refresh": 2,
        "includeAll": true,
        "current": { "text": "All", "value": "$__all" }
      }
    ]
  },
  "panels": [
    {
      "id": 1,
      "title": "RPS achieved",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 0, "w": 12, "h": 8 },
      "targets": [
        {
          "expr": "sum(rate(k6_http_reqs_total{scenario=~\"$scenario\", git_sha=~\"$git_sha\"}[30s]))",
          "legendFormat": "rps"
        }
      ]
    },
    {
      "id": 2,
      "title": "HTTP latency (p50/p95/p99)",
      "type": "timeseries",
      "gridPos": { "x": 12, "y": 0, "w": 12, "h": 8 },
      "fieldConfig": { "defaults": { "unit": "s" } },
      "targets": [
        {
          "expr": "max(k6_http_req_duration_p50{scenario=~\"$scenario\", git_sha=~\"$git_sha\"})",
          "legendFormat": "p50 (worst endpoint)"
        },
        {
          "expr": "max(k6_http_req_duration_p95{scenario=~\"$scenario\", git_sha=~\"$git_sha\"})",
          "legendFormat": "p95 (worst endpoint)"
        },
        {
          "expr": "max(k6_http_req_duration_p99{scenario=~\"$scenario\", git_sha=~\"$git_sha\"})",
          "legendFormat": "p99 (worst endpoint)"
        }
      ]
    },
    {
      "id": 3,
      "title": "Error rate",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 8, "w": 12, "h": 8 },
      "fieldConfig": { "defaults": { "unit": "percentunit" } },
      "targets": [
        {
          "expr": "max(k6_http_req_failed_rate{scenario=~\"$scenario\", git_sha=~\"$git_sha\"})",
          "legendFormat": "failed rate (worst endpoint)"
        }
      ]
    },
    {
      "id": 4,
      "title": "Per-endpoint p95 latency",
      "type": "timeseries",
      "gridPos": { "x": 12, "y": 8, "w": 12, "h": 8 },
      "fieldConfig": { "defaults": { "unit": "s" } },
      "targets": [
        {
          "expr": "max by (endpoint) (k6_http_req_duration_p95{scenario=~\"$scenario\", git_sha=~\"$git_sha\"})",
          "legendFormat": "{{endpoint}}"
        }
      ]
    },
    {
      "id": 5,
      "title": "VUs",
      "type": "timeseries",
      "gridPos": { "x": 0, "y": 16, "w": 12, "h": 8 },
      "targets": [
        {
          "expr": "max(k6_vus{scenario=~\"$scenario\", git_sha=~\"$git_sha\"})",
          "legendFormat": "vus"
        }
      ]
    },
    {
      "id": 6,
      "title": "Checks pass rate",
      "type": "timeseries",
      "gridPos": { "x": 12, "y": 16, "w": 12, "h": 8 },
      "fieldConfig": { "defaults": { "unit": "percentunit" } },
      "targets": [
        {
          "expr": "min(k6_checks_rate{scenario=~\"$scenario\", git_sha=~\"$git_sha\"})",
          "legendFormat": "checks pass rate (worst check)"
        }
      ]
    }
  ]
}
```

- [ ] **Step 2: Restart Grafana so the new dashboard provisions**

```bash
docker compose -f docker/compose.yaml -f docker/compose.k6.yaml restart grafana
```

- [ ] **Step 3: Confirm the dashboard appears**

Open `http://localhost:3001/dashboards` in a browser. Expected: a dashboard titled **"k6 load tests"** alongside the existing **"todo-app"**.

- [ ] **Step 4: Run smoke and verify panels populate**

```bash
just k6-smoke
```

Then open `http://localhost:3001/d/k6-load-tests` and confirm at least the **RPS achieved** and **HTTP latency** panels show non-empty time series within ~30 seconds.

**If panels are empty:**
- The most common cause is metric-name mismatch. Find the actual names with:

  ```bash
  curl -sG --data-urlencode 'match[]={__name__=~"k6_.*"}' \
      http://localhost:9090/api/v1/series | jq -r '.data[] | .__name__' | sort -u
  ```

  Update the dashboard expressions accordingly (e.g., k6 may emit `k6_http_reqs_total` rather than `k6_http_reqs` depending on the version).

- A second cause is the `stat` label. Confirm with:

  ```bash
  curl -sG --data-urlencode 'match[]=k6_http_req_duration' \
      http://localhost:9090/api/v1/series | jq '.data[] | .stat' | sort -u
  ```

  Expected: `"avg"`, `"p(50)"`, `"p(95)"`, `"p(99)"`, `"max"`. If absent, the `K6_PROMETHEUS_RW_TREND_STATS` env var didn't take effect on the k6 container — `docker compose -f docker/compose.yaml -f docker/compose.k6.yaml run --rm k6 env | grep TREND_STATS`.

- [ ] **Step 5: Commit**

```bash
git add docker/grafana/dashboards/k6.json
git commit -m "grafana: add k6 load test dashboard

Six panels: RPS, latency percentiles, error rate, per-endpoint p95,
VUs, checks pass rate. Templated on scenario + git_sha so multiple
runs can be compared side by side.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Final validation + flesh out the README

**Files:**
- Modify: `k6/README.md`

- [ ] **Step 1: Verify production rate-limit behavior unchanged**

Bring up the prod (non-k6) stack and confirm `/login` still rate-limits at the 6th attempt:

```bash
just k6-down
just up-d   # base + dev override, no k6 override
sleep 5     # let the app finish starting
for i in 1 2 3 4 5 6; do
  curl -s -o /dev/null -w "attempt $i: %{http_code}\n" \
    -d 'email=foo@example.com&password=wrong' \
    http://localhost:3000/login
done
```

Expected: attempts 1–5 return `200` (form re-render with error), attempt 6 returns `429`. If the 6th is still `200`, the bypass leaked into prod — re-check Task 3's wiring and Task 1's defaults.

- [ ] **Step 2: Threshold violation check**

Confirm a violated threshold makes `k6 run` exit non-zero. Temporarily edit `k6/scenarios/smoke.js`'s thresholds block to an impossible-to-pass value:

```javascript
  thresholds: {
    ...SHARED_THRESHOLDS,
    'http_req_duration{endpoint:healthz}': ['p(95)<1'],   // 1ms — will fail
  },
```

```bash
just k6-down
just k6-smoke; echo "exit code: $?"
```

Expected: k6 prints `✗ p(95)<1` (or equivalent threshold-violated marker) and the recipe exits non-zero (typical: `99`). Revert the change:

```bash
git checkout -- k6/scenarios/smoke.js
```

- [ ] **Step 3: Write the full README**

Overwrite `k6/README.md` with the full guide:

````markdown
# k6 Load Tests

Grafana k6 scenarios for load and perf testing the app locally. Runs entirely
inside docker compose — no host install of k6 needed.

## Quick start

```bash
just k6-smoke      # 30s sanity check
just k6-load       # ramping arrival rate up to 1000 req/s on GET /
just k6-journey    # 30 VUs running the full signup → CRUD → list loop
just k6-all        # all three in sequence; stops on first failure
just k6-clean-db   # remove load-test users + their todos
just k6-down       # tear down the k6 stack (preserves volumes)
```

While a run is in progress, open Grafana at <http://localhost:3001/d/k6-load-tests>
for live charts. The default time range is the last 15 minutes; templated
variables on the top bar let you filter to a specific scenario + git SHA.

Per-run JSON summaries land in `k6/results/summary-<scenario>-<utc-ts>-<git-sha>.json`.
The directory is gitignored — these are local artifacts only.

## Scenarios

### `smoke.js`
1 VU for 30 seconds. Exercises `/healthz`, `/readyz`, `/login` (GET),
`/` (303 redirect when unauth), `/static/css/app.css`, `/metrics`.

Thresholds: tight (p95 < 20 ms on `/healthz`; error rate < 1%; checks > 99%).

### `read_heavy.js`
Maps to the design's perf target: ~1000 RPS on `GET /` with cache hot,
p95 < 50 ms (aspirational; not threshold-enforced).

Setup pre-seeds users + todos via the API (idempotent — reruns skip
existing users). Each VU is assigned a seed user on iteration 0 and
logs in once; subsequent iterations reuse the cookie jar.

Override defaults:
```bash
just k6 read_heavy   # uses USERS=50 TODOS_PER_USER=10
# Or with custom values:
docker compose -f docker/compose.yaml -f docker/compose.k6.yaml run --rm \
    -e USERS=200 -e TODOS_PER_USER=20 \
    k6 run /scripts/scenarios/read_heavy.js
```

Thresholds: loose (p95 < 500 ms on `/`; error rate < 1%; checks > 99%).
The aspirational `p95 < 50 ms / 1000 RPS` target is documented for eyeballing
on the dashboard — not enforced.

### `journey.js`
30 VUs ramping over 7 minutes. Each VU is its own user. Per iteration:
signup (first iter) → list → create×3 → toggle → delete → list.

Thresholds: loose (p95 < 500 ms; iteration p95 < 3s; checks > 99%).

## Rate-limit bypass

The app's per-IP rate limiter (5 hits/min/IP on `/login` + `/signup`) would
429 any meaningful load test sourced from one machine. We bypass it via
`APP__RATE_LIMIT__ENABLED=false`, set on the app service only when
`compose.k6.yaml` is loaded. The base `compose.yaml` (and prod deploys)
leave it enabled.

To verify the bypass is doing what you expect, watch the
`auth_rate_limited_total` counter on the app dashboard — it should stay flat
during k6 runs.

## Adding a scenario

1. Create `k6/scenarios/<name>.js`. Import the lib helpers:
   ```javascript
   import { BASE_URL, SHARED_THRESHOLDS, makeSummaryWriter } from '../lib/config.js';
   import { Endpoint, params, assertStatus } from '../lib/checks.js';
   ```
2. Tag every request with `params(Endpoint.SomeName)` — the dashboard's
   per-endpoint panel needs the tag.
3. Export `handleSummary = makeSummaryWriter('<name>')` so a JSON summary
   lands in `k6/results/`.
4. Optionally add a convenience recipe in the justfile (`k6-<name>:
   (k6 "<name>")`).

## Adding a new endpoint name

`Endpoint` in `k6/lib/checks.js` is a closed enum-as-const. Update it
when a new app route gets load-tested, otherwise the dashboard's
per-endpoint panel won't group cleanly.
````

- [ ] **Step 4: Verify the final state**

Make sure the bypass and all three scenarios still work cleanly:

```bash
just k6-clean-db   # start from a clean slate
just k6-all
```

Expected: all three scenarios complete with `checks rate > 99%`, `http_req_failed rate < 1%`, and the recipe exits 0.

- [ ] **Step 5: Commit**

```bash
git add k6/README.md
git commit -m "docs: flesh out k6/README.md

How to run, what each scenario does, threshold rationale, and how to
add new scenarios or endpoint tags.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Wrap-up

After Task 12, the branch should contain ~12 commits adding:

- Rate-limit bypass config + middleware change (Tasks 1–3)
- k6 directory + shared lib (Tasks 4–5)
- Compose override + just recipes (Tasks 6–7)
- Three scenarios (Tasks 8–10)
- Grafana dashboard (Task 11)
- README + validation (Task 12)

To open the PR:

```bash
git push -u origin feat/k6-load-tests
gh pr create --title "feat: add k6 load tests" --body "$(cat <<'EOF'
## Summary
- Adds Grafana k6 to the local stack with three scenarios: smoke, read-heavy, journey
- Runs entirely in docker compose — no host k6 install needed
- Streams results to existing Prometheus via remote-write; new Grafana dashboard at `k6-load-tests`
- One small app change: default-off `rate_limit.enabled` config flag so seeding can burst-signup without 429s

## Test plan
- [ ] `just k6-smoke` exits 0
- [ ] `just k6-load` exits 0 and DB shows 50 `loadtest-%` users with 500 todos
- [ ] `just k6-journey` exits 0 and DB shows `journey-%` users
- [ ] `just up-prod` (no k6 override) — 6th `/login` returns 429 (prod rate-limit unchanged)
- [ ] Grafana dashboard `k6-load-tests` populates panels during a run
- [ ] `cargo test --workspace` passes (23 tests: 21 pre-existing + 2 new rate-limit middleware tests)

Design: `docs/superpowers/specs/2026-05-17-k6-load-tests-design.md`
Plan: `docs/superpowers/plans/2026-05-17-k6-load-tests.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
