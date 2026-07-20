# Multi-stage Dockerfile for the standalone Reborn CLI HTTP service.
#
# Build:
#   docker build -f Dockerfile.reborn -t ironclaw-reborn:latest .
#
# Run locally:
#   docker run --rm --env-file .env.reborn -p 127.0.0.1:3000:3000 ironclaw-reborn:latest
#
# Railway:
#   Set Dockerfile path to Dockerfile.reborn and IRONCLAW_REBORN_SERVE_HOST=0.0.0.0.
#   Railway supplies PORT. Set IRONCLAW_REBORN_PROFILE=hosted-single-tenant for
#   Postgres-backed hosted storage, or hosted-single-tenant-volume for a
#   Railway volume-backed single-tenant preview.

FROM node:22.23.1-bookworm-slim@sha256:813a7480f28fdadac1f7f5c824bcdad435b5bc1322a5968bbbdef8d058f9dff4 AS node_toolchain

FROM rust:1.96-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS chef

COPY --from=node_toolchain /usr/local/bin/node /usr/local/bin/node
COPY --from=node_toolchain /usr/local/lib/node_modules/ /usr/local/lib/node_modules/

RUN ln -sf ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -sf ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && ln -sf ../lib/node_modules/corepack/dist/corepack.js /usr/local/bin/corepack \
    && node --version \
    && npm --version \
    && corepack --version \
    && corepack enable pnpm \
    && cargo install --locked cargo-chef@0.1.77

WORKDIR /app

FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY tools/ironclaw_stress/ tools/ironclaw_stress/
COPY skills/ skills/
COPY tests/ tests/
COPY wit/ wit/
COPY providers.json providers.json
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf '\n' > src/lib.rs

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS deps

ENV CARGO_PROFILE_DIST_PANIC=abort \
    CARGO_PROFILE_DIST_CODEGEN_UNITS=1

COPY --from=planner /app/recipe.json recipe.json
COPY crates/ironclaw_webui/frontend/ crates/ironclaw_webui/frontend/
WORKDIR /app/crates/ironclaw_webui/frontend
RUN pnpm install --frozen-lockfile
WORKDIR /app
RUN cargo chef cook \
    --profile dist \
    --package ironclaw \
    --features libsql,postgres \
    --recipe-path recipe.json
RUN cargo chef cook \
    --profile dist \
    --package ironclaw_reborn_migration \
    --no-default-features \
    --features libsql \
    --recipe-path recipe.json

FROM deps AS builder

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY tools/ironclaw_stress/ tools/ironclaw_stress/
COPY migrations/ migrations/
COPY skills/ skills/
COPY tests/ tests/
COPY wit/ wit/
COPY providers.json providers.json
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf '\n' > src/lib.rs

WORKDIR /app/crates/ironclaw_webui/frontend
RUN pnpm install --frozen-lockfile
WORKDIR /app

RUN cargo build \
    --profile dist \
    --package ironclaw \
    --features libsql,postgres \
    --bin ironclaw

RUN cargo build \
    --profile dist \
    --package ironclaw_reborn_migration \
    --no-default-features \
    --features libsql \
    --bin ironclaw-reborn-extension-ownership-migration

FROM debian:bookworm-slim AS runtime

RUN apt-get -o Acquire::Retries=3 update \
    && apt-get -o Acquire::Retries=3 install -y --no-install-recommends \
        ca-certificates \
        postgresql-client \
        sqlite3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/dist/ironclaw /usr/local/bin/ironclaw
COPY --from=builder /app/target/dist/ironclaw-reborn-extension-ownership-migration /usr/local/bin/ironclaw-reborn-extension-ownership-migration
COPY docker/reborn/config.toml /opt/ironclaw/reborn/config.toml
COPY docker/reborn/config.hosted-single-tenant.toml /opt/ironclaw/reborn/config.hosted-single-tenant.toml
COPY docker/reborn/config.hosted-single-tenant-volume.toml /opt/ironclaw/reborn/config.hosted-single-tenant-volume.toml
COPY docker/reborn/config.production.toml /opt/ironclaw/reborn/config.production.toml
COPY docker/reborn/entrypoint.sh /usr/local/bin/ironclaw-reborn-entrypoint

ENV HOME=/home/ironclaw \
    IRONCLAW_REBORN_LOG=info \
    IRONCLAW_REBORN_SERVE_HOST=127.0.0.1

RUN useradd -m -d /home/ironclaw -u 1000 ironclaw \
    && mkdir -p /data/ironclaw-reborn /workspace \
    && chown -R ironclaw:ironclaw /home/ironclaw /data/ironclaw-reborn /workspace \
    && chmod +x /usr/local/bin/ironclaw-reborn-entrypoint

WORKDIR /workspace

EXPOSE 3000

USER ironclaw

ENTRYPOINT ["ironclaw-reborn-entrypoint"]
