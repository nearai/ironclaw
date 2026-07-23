# Multi-stage Dockerfile for the standalone IronClaw CLI HTTP service.
#
# Build:
#   docker build -f Dockerfile -t ironclaw:latest .
#
# Run locally:
#   docker run --rm --env-file .env.ironclaw -p 127.0.0.1:3000:3000 ironclaw:latest
#
# Railway:
#   Set Dockerfile path to Dockerfile and IRONCLAW_SERVE_HOST=0.0.0.0.
#   Railway supplies PORT. Set IRONCLAW_PROFILE=hosted-single-tenant for
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
    --bin ironclaw

FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818 AS runtime

RUN apt-get -o Acquire::Retries=3 update \
    && apt-get -o Acquire::Retries=3 install -y --no-install-recommends \
        ca-certificates \
        postgresql-client \
        sqlite3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/dist/ironclaw /usr/local/bin/ironclaw
COPY docker/ironclaw/config.toml /opt/ironclaw/defaults/config.toml
COPY docker/ironclaw/config.hosted-single-tenant.toml /opt/ironclaw/defaults/config.hosted-single-tenant.toml
COPY docker/ironclaw/config.hosted-single-tenant-volume.toml /opt/ironclaw/defaults/config.hosted-single-tenant-volume.toml
COPY docker/ironclaw/config.production.toml /opt/ironclaw/defaults/config.production.toml
COPY docker/ironclaw/entrypoint.sh /usr/local/bin/ironclaw-entrypoint

ENV HOME=/home/ironclaw \
    IRONCLAW_LOG=info \
    IRONCLAW_SERVE_HOST=127.0.0.1

RUN useradd -m -d /home/ironclaw -u 1000 ironclaw \
    && mkdir -p /data/ironclaw /workspace \
    && chown -R ironclaw:ironclaw /home/ironclaw /data/ironclaw /workspace \
    && chmod +x /usr/local/bin/ironclaw-entrypoint

WORKDIR /workspace

EXPOSE 3000

USER ironclaw

ENTRYPOINT ["ironclaw-entrypoint"]
