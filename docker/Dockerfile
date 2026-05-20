# syntax=docker/dockerfile:1.7

ARG RUST_VERSION=1.94

# -----------------------------------------------------------------------------
# 1. Plan (cargo-chef): captures the dependency graph independent of source.
# -----------------------------------------------------------------------------
FROM lukemathwalker/cargo-chef:latest-rust-${RUST_VERSION}-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# -----------------------------------------------------------------------------
# 2. Build deps (cached layer) + 3. Build app
# -----------------------------------------------------------------------------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
# Stamped into the binary as `X-App-Version`. `.git/` is excluded by
# `.dockerignore`, so the build script can't shell out — pass it in via
# `docker build --build-arg GIT_SHA=$(git rev-parse HEAD)` (or compose).
ARG GIT_SHA=unknown
# We use `query_as` not `query!`, so SQLX_OFFLINE isn't strictly required, but
# we set it for safety in case anyone adds a macro later.
ENV SQLX_OFFLINE=true GIT_SHA=$GIT_SHA
RUN cargo build --release --bin todo-app

# -----------------------------------------------------------------------------
# 4a. Tailwind binary (cacheable: rebuilt only when the script or version changes)
# -----------------------------------------------------------------------------
FROM debian:bookworm-slim AS css-builder
WORKDIR /app
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
COPY scripts/install-tailwind.sh /tmp/install-tailwind.sh
RUN INSTALL_DIR=/usr/local/bin sh /tmp/install-tailwind.sh

# -----------------------------------------------------------------------------
# 4b. Tailwind CSS build
# -----------------------------------------------------------------------------
FROM css-builder AS css
COPY templates ./templates
COPY static ./static
RUN tailwindcss -i ./static/css/app.src.css -o ./static/css/app.css --minify

# -----------------------------------------------------------------------------
# 5. Runtime: distroless cc (sqlx with runtime-tokio-rustls needs glibc).
# -----------------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12 AS runtime
WORKDIR /app

COPY --from=builder /app/target/release/todo-app /usr/local/bin/todo-app
COPY --from=css     /app/static                  /app/static
COPY templates                                   /app/templates
COPY migrations                                  /app/migrations
COPY locales                                     /app/locales

ENV APP__TEMPLATES_DIR=/app/templates \
    APP__STATIC_DIR=/app/static \
    APP__LOCALES_DIR=/app/locales \
    APP__TEMPLATE_AUTORELOAD=false

EXPOSE 3000
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/todo-app"]
