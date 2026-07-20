# Default production Dockerfile for the canonical IronClaw Reborn HTTP service.
#
# Build:
#   docker build --target runtime -t ironclaw:latest .
#
# Run locally:
#   docker run --rm --env-file .env.reborn -p 127.0.0.1:3000:3000 ironclaw:latest
#
# Railway:
#   Use the default Dockerfile and set IRONCLAW_REBORN_SERVE_HOST=0.0.0.0.
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
COPY scripts/ci/reborn-shipping-features.txt scripts/ci/reborn-shipping-features.txt
WORKDIR /app/crates/ironclaw_webui/frontend
RUN pnpm install --frozen-lockfile
WORKDIR /app
RUN shipping_features="$(cat scripts/ci/reborn-shipping-features.txt)" \
    && cargo chef cook \
    --profile dist \
    --package ironclaw_reborn_cli \
    --features "$shipping_features" \
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

RUN shipping_features="$(cat scripts/ci/reborn-shipping-features.txt)" \
    && cargo build \
    --profile dist \
    --package ironclaw_reborn_cli \
    --features "$shipping_features" \
    --bin ironclaw

RUN cargo build \
    --profile dist \
    --package ironclaw_reborn_migration \
    --no-default-features \
    --features libsql \
    --bin ironclaw-reborn-extension-ownership-migration

FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818 AS runtime

LABEL io.nearai.ironclaw.runtime="reborn"

RUN apt-get -o Acquire::Retries=3 update \
    && apt-get -o Acquire::Retries=3 install -y --no-install-recommends \
        ca-certificates \
        curl \
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
    IRONCLAW_REBORN_SERVE_HOST=0.0.0.0

RUN useradd -m -d /home/ironclaw -u 1000 ironclaw \
    && mkdir -p /data/ironclaw-reborn /workspace \
    && chown -R ironclaw:ironclaw /home/ironclaw /data/ironclaw-reborn /workspace \
    && chmod +x /usr/local/bin/ironclaw-reborn-entrypoint

# Build stages use /app; runtime commands intentionally start in the user's workspace.
WORKDIR /workspace

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=10s --start-period=20s --retries=3 \
    CMD port="${PORT:-3000}"; \
        case "$port" in ''|*[!0-9]*) exit 1 ;; esac; \
        curl --fail --silent --show-error --max-time 5 "http://127.0.0.1:${port}/api/health" >/dev/null || exit 1

USER ironclaw

ENTRYPOINT ["ironclaw-reborn-entrypoint"]
