set dotenv-load

default:
    @just --list

# --- Local (no docker) ---

run:
    SQLX_OFFLINE=false systemfd --no-pid -s http::0.0.0.0:3000 -- cargo watch -x 'run --bin todo-app'

run-once:
    cargo run --bin todo-app

css:
    npm run watch

css-build:
    npm run build

fmt:
    cargo fmt --all

# `cargo fmt --check` exits non-zero if anything would be reformatted; useful pre-commit.
fmt-check:
    cargo fmt --all --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Auto-fix clippy lints that have machine-applicable suggestions. Leaves the rest.
lint-fix:
    cargo clippy --workspace --all-targets --fix --allow-dirty -- -D warnings

test:
    cargo test --workspace

# Unit + bin tests only — skips the testcontainer-backed integration suites.
test-unit:
    cargo test --workspace --lib --bins

# The full pre-commit / pre-push gate.
check: fmt-check lint test

migrate:
    sqlx migrate run --source migrations

# Create a new sqlx migration. Usage: `just migrate-new add_thing`
migrate-new name:
    sqlx migrate add --source migrations {{name}}

prepare:
    cargo sqlx prepare --workspace -- --bin todo-app

# --- Docker ---

# Default `up` brings up the dev override (cargo-watch + tailwind polling).
up:
    GIT_SHA=$(git rev-parse HEAD 2>/dev/null || echo unknown) docker compose -f docker/compose.yaml -f docker/compose.dev.yaml up --build

up-prod:
    GIT_SHA=$(git rev-parse HEAD 2>/dev/null || echo unknown) docker compose -f docker/compose.yaml up --build

# Background mode; returns control to the shell instead of streaming logs.
up-d:
    GIT_SHA=$(git rev-parse HEAD 2>/dev/null || echo unknown) docker compose -f docker/compose.yaml -f docker/compose.dev.yaml up -d --build

down:
    docker compose -f docker/compose.yaml -f docker/compose.dev.yaml down

# Nuke everything including volumes (Postgres data, Loki blocks, etc.). Use after a
# breaking schema change or to reset Grafana state.
nuke:
    docker compose -f docker/compose.yaml -f docker/compose.dev.yaml down -v

# Down + up; preserves volumes.
restart: down up-d

# Tail one service's logs. `just logs app` / `just logs otel-collector` / etc.
logs svc='app':
    docker compose -f docker/compose.yaml -f docker/compose.dev.yaml logs -f {{svc}}

# Tail every service interleaved.
logs-all:
    docker compose -f docker/compose.yaml -f docker/compose.dev.yaml logs -f

# Service status / health.
ps:
    docker compose -f docker/compose.yaml -f docker/compose.dev.yaml ps

# Drop into a psql shell against the running db container.
psql:
    docker compose -f docker/compose.yaml -f docker/compose.dev.yaml exec db psql -U todo -d todo

# Drop into a valkey-cli shell against the running cache container.
valkey:
    docker compose -f docker/compose.yaml -f docker/compose.dev.yaml exec cache valkey-cli

# --- Browser ---

# Open the app + observability stack in the default browser.
open:
    open http://localhost:3000
    open http://localhost:3001    # Grafana
    open http://localhost:9090    # Prometheus

# --- Quick observability probes ---

# Generate N requests against /healthz to exercise the metric + trace + log pipelines.
traffic n='30':
    @for i in $(seq 1 {{n}}); do curl -sf http://localhost:3000/healthz > /dev/null; done; \
        echo "fired {{n}} requests against /healthz"

# Quick Prometheus query. Usage: `just prom 'sum(rate(http_requests_total[1m]))'`
prom query:
    @curl -sG --data-urlencode 'query={{query}}' http://localhost:9090/api/v1/query \
        | python3 -m json.tool

# Quick LogQL query over the last 5 minutes.
# Usage: `just loki '{service_name="todo-app"} | trace_id != ""'`
loki query limit='10':
    @curl -sG --data-urlencode 'query={{query}}' \
              --data-urlencode 'limit={{limit}}' \
              --data-urlencode 'direction=backward' \
              --data-urlencode "start=$(($(date +%s) - 300))000000000" \
              http://localhost:3100/loki/api/v1/query_range \
      | python3 -m json.tool

# Recent Tempo traces for todo-app.
tempo limit='5':
    @curl -sG --data-urlencode 'tags=service.name=todo-app' \
              --data-urlencode 'limit={{limit}}' \
              --data-urlencode "start=$(($(date +%s) - 300))" \
              --data-urlencode "end=$(date +%s)" \
              http://localhost:3200/api/search \
      | python3 -m json.tool

# --- Secrets ---

gen-session-key:
    @openssl rand -hex 64

# Quick: generate a session key and inject it into .env (creates from .env.example if missing)
init-env:
    @test -f .env || cp .env.example .env
    @key=$(openssl rand -hex 64); \
        sed -i.bak -e "s|^APP__AUTH__SESSION_KEY=.*|APP__AUTH__SESSION_KEY=$key|" \
                   -e "s|^SESSION_KEY=.*|SESSION_KEY=$key|" .env; \
        rm -f .env.bak; \
        echo "wrote new SESSION_KEY to .env"

# --- Maintenance ---

# Bump every dep across both lockfiles. Run `just check` after.
update:
    cargo update
    npm update

# Pre-compress vendor files for ServeDir's precompressed_gzip/br paths.
# Run this after vendoring new versions of htmx / alpine.
vendor-compress:
    @cd static/vendor && \
        for f in *.js; do \
            gzip -9 -k -f "$f"; \
            command -v brotli >/dev/null && brotli -9 -f -k "$f" || true; \
        done && \
        ls -la

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
