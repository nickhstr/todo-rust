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

Setup pre-seeds users + todos via the API (idempotent — reruns skip existing
users). Each VU is assigned a seed user, logs in once on iteration 0, then
loops. Note that k6 clears the default per-VU cookie jar between iterations,
so the scenario uses a module-scope `new http.CookieJar()` and passes it
explicitly on every request — see the inline comment in
[`scenarios/read_heavy.js`](scenarios/read_heavy.js).

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
on the dashboard — not enforced. Tighten the thresholds later as the perf
surface stabilises.

### `journey.js`
30 VUs ramping over 7 minutes. Each VU is its own user. Per iteration:
signup (first iter) → list → create×3 → toggle → delete → list.

Uses the same module-scope cookie jar pattern as `read_heavy.js`.

Thresholds: loose (p95 < 500 ms; iteration p95 < 3s; checks > 99%).

## Rate-limit bypass

The app's per-IP rate limiter (5 hits/min/IP on `/login` + `/signup`) would
429 any meaningful load test sourced from one machine. We bypass it via
`APP__RATE_LIMIT__ENABLED=false`, set on the app service only when
`docker/compose.k6.yaml` is loaded. The base `compose.yaml` (and prod deploys)
leave it enabled — verified by running `just up-d` and confirming the 6th
consecutive `/login` POST still returns 429.

To watch the bypass in action: the `auth_rate_limited_total` counter on the
app dashboard stays flat during k6 runs.

## Tooling notes

- **k6 image**: pinned to `grafana/k6:0.53.0` in `docker/compose.k6.yaml`.
  v0.53 is the first release with native ESM support; earlier versions
  (0.50) used a bundled Babel transpiler that couldn't parse object spread
  syntax.
- **Prometheus remote-write**: enabled only when the k6 override is loaded
  (`--web.enable-remote-write-receiver`). The flag is intentionally absent
  from the base compose so production deploys don't expose a write endpoint.
- **k6 metric names**: v0.53's prometheus-rw output emits trend stats as
  separate metrics with name suffixes (`k6_http_req_duration_p95`, etc.)
  rather than as a `stat=` label. Counters and rates get `_total` / `_rate`
  suffixes. The dashboard queries reflect this.

## Adding a scenario

1. Create `k6/scenarios/<name>.js`. Import the lib helpers:
   ```javascript
   import { BASE_URL, SHARED_THRESHOLDS, makeSummaryWriter } from '../lib/config.js';
   import { Endpoint, params, assertStatus } from '../lib/checks.js';
   ```
2. Tag every request with `params(Endpoint.SomeName)` — the dashboard's
   per-endpoint panel needs the tag.
3. If the scenario maintains a session across iterations (any executor),
   create a module-scope `const jar = new http.CookieJar()` and pass `{ jar }`
   on every request. The default per-VU jar is reset between iterations.
4. Export `handleSummary = makeSummaryWriter('<name>')` so a JSON summary
   lands in `k6/results/`.
5. Optionally add a convenience recipe in the justfile
   (`k6-<name>: (k6 "<name>")`).

## Adding a new endpoint name

`Endpoint` in `k6/lib/checks.js` is a closed enum-as-const. Update it when
a new app route gets load-tested, otherwise the dashboard's per-endpoint
panel won't group cleanly.

## Troubleshooting

**No k6 metrics in Prometheus after a run.** Confirm the receiver flag is
active: `docker compose -f docker/compose.yaml -f docker/compose.k6.yaml exec prometheus wget -q -O - http://localhost:9090/api/v1/status/flags | jq '.data["web.enable-remote-write-receiver"]'` should print `"true"`. If not, the override didn't merge — check that the `k6` recipe in the justfile passes both `-f docker/compose.yaml` and `-f docker/compose.k6.yaml`.

**Setup timeout during read_heavy.** Default is 60s. With N=50 + cold cache,
argon2 hashing for fresh signups takes ~4–6s; reruns hit 409s and finish in
under a second. If you push USERS very high, add `setupTimeout: '5m'` to
the scenario's `options`.

**Dashboard panels empty.** Run `just k6-smoke`, then check the metric
names that arrived:
```bash
curl -sG --data-urlencode 'match[]={__name__=~"k6_.*"}' \
    http://localhost:9090/api/v1/series | jq -r '.data[].__name__' | sort -u
```
The dashboard expects v0.53-style names (`*_total`, `*_rate`, `*_p95`).
If your k6 image differs, update the panel exprs in
`docker/grafana/dashboards/k6.json`.
