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
# We use `query_as` not `query!`, so SQLX_OFFLINE isn't strictly required, but
# we set it for safety in case anyone adds a macro later.
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin todo-app

# -----------------------------------------------------------------------------
# 4. Tailwind CSS build
# -----------------------------------------------------------------------------
FROM node:24-bookworm-slim AS css
WORKDIR /app
COPY package.json ./
RUN npm install --no-audit --no-fund
COPY tailwind.config.js* ./
COPY templates ./templates
COPY static ./static
RUN npm run build

# -----------------------------------------------------------------------------
# 5. Runtime: distroless cc (sqlx with runtime-tokio-rustls needs glibc).
# -----------------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12 AS runtime
WORKDIR /app

COPY --from=builder /app/target/release/todo-app /usr/local/bin/todo-app
COPY --from=css     /app/static                  /app/static
COPY templates                                   /app/templates
COPY migrations                                  /app/migrations

ENV APP__TEMPLATES_DIR=/app/templates \
    APP__STATIC_DIR=/app/static \
    APP__TEMPLATE_AUTORELOAD=false

EXPOSE 3000
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/todo-app"]
