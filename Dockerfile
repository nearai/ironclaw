# Multi-stage Dockerfile for the standalone Reborn CLI HTTP service.
#
# Build:
#   docker build -f Dockerfile -t ironclaw-reborn:latest .
#
# Run locally:
#   docker run --rm --env-file .env.reborn -p 127.0.0.1:3000:3000 ironclaw-reborn:latest
#
# Railway:
#   Set Dockerfile path to Dockerfile and IRONCLAW_REBORN_SERVE_HOST=0.0.0.0.
#   Railway supplies PORT. Set IRONCLAW_REBORN_PROFILE=hosted-single-tenant for
#   Postgres-backed storage, hosted-single-tenant-volume for a volume-backed
#   preview, or hosted-single-tenant-volume-sandboxed-railway for the explicit
#   Railway Sandbox preview described in docs/internal/reborn/railway-sandbox-operator.md.

FROM node:22.23.1-bookworm-slim@sha256:813a7480f28fdadac1f7f5c824bcdad435b5bc1322a5968bbbdef8d058f9dff4 AS node_toolchain

FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818 AS railway_cli

ARG TARGETARCH
ARG RAILWAY_CLI_VERSION=5.30.4
RUN apt-get -o Acquire::Retries=3 update \
    && apt-get -o Acquire::Retries=3 install -y --no-install-recommends \
        ca-certificates \
        curl \
    && case "$TARGETARCH" in \
        amd64) \
            railway_target="x86_64-unknown-linux-gnu"; \
            railway_sha256="33addd7729e99291f329ac671b02e9fe14fec8b7d9cdc11be77569739dae5c0e"; \
            ;; \
        arm64) \
            railway_target="aarch64-unknown-linux-musl"; \
            railway_sha256="11c24392e5e3551687c5e35ade2eec63e2ea7689603117de83f4f480dbb2d2a7"; \
            ;; \
        *) \
            echo "unsupported Railway CLI architecture: $TARGETARCH" >&2; \
            exit 1 \
            ;; \
    esac \
    && railway_archive="railway-v${RAILWAY_CLI_VERSION}-${railway_target}.tar.gz" \
    && curl -fsSL \
        "https://github.com/railwayapp/cli/releases/download/v${RAILWAY_CLI_VERSION}/${railway_archive}" \
        -o "/tmp/${railway_archive}" \
    && echo "${railway_sha256}  /tmp/${railway_archive}" | sha256sum -c - \
    && tar -xzf "/tmp/${railway_archive}" -C /tmp railway \
    && install -m 0755 /tmp/railway /usr/local/bin/railway \
    && railway --version

FROM rust:1.96-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS chef

COPY --from=node_toolchain /usr/local/bin/node /usr/local/bin/node
COPY --from=node_toolchain /usr/local/lib/node_modules/ /usr/local/lib/node_modules/

WORKDIR /app
COPY .cargo/config.toml .cargo/config.toml

RUN ln -sf ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -sf ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && ln -sf ../lib/node_modules/corepack/dist/corepack.js /usr/local/bin/corepack \
    && node --version \
    && npm --version \
    && corepack --version \
    && corepack enable pnpm \
    && cargo install --locked cargo-chef@0.1.77

FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY tools/ironclaw_stress/ tools/ironclaw_stress/
COPY skills/ skills/
COPY tests/ tests/
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf '\n' > src/lib.rs

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS deps

ENV CARGO_PROFILE_DIST_PANIC=abort \
    CARGO_PROFILE_DIST_CODEGEN_UNITS=1

COPY --from=planner /app/recipe.json recipe.json
COPY crates/product/ironclaw_webui/frontend/ crates/product/ironclaw_webui/frontend/
WORKDIR /app/crates/product/ironclaw_webui/frontend
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
RUN mkdir -p src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf '\n' > src/lib.rs

WORKDIR /app/crates/product/ironclaw_webui/frontend
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
        curl \
        gosu \
        openssh-server \
        postgresql-client \
        sqlite3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/dist/ironclaw /usr/local/bin/ironclaw
COPY --from=railway_cli /usr/local/bin/railway /usr/local/bin/railway
COPY docker/reborn/config.toml /opt/ironclaw/reborn/config.toml
COPY docker/reborn/config.hosted-single-tenant.toml /opt/ironclaw/reborn/config.hosted-single-tenant.toml
COPY docker/reborn/config.hosted-single-tenant-volume.toml /opt/ironclaw/reborn/config.hosted-single-tenant-volume.toml
COPY docker/reborn/config.production.toml /opt/ironclaw/reborn/config.production.toml
COPY docker/reborn/entrypoint.sh /usr/local/bin/ironclaw-reborn-entrypoint
COPY docker/reborn/start-sshd.sh /usr/local/bin/ironclaw-reborn-start-sshd

ENV HOME=/home/ironclaw \
    IRONCLAW_REBORN_LOG=info \
    IRONCLAW_REBORN_SERVE_HOST=127.0.0.1

# `agent` is a deliberate uid-1000 alias of `ironclaw`, not a separate,
# lower-privileged account. A session logged in as `agent` (e.g. over SSH)
# holds the full runtime identity and can read/write everything the IronClaw
# process owns.
RUN useradd -m -d /home/ironclaw -u 1000 ironclaw \
    && useradd --non-unique --uid 1000 --gid ironclaw --home-dir /workspace --shell /bin/bash agent \
    && passwd -d agent \
    && mkdir -p /data/ironclaw-reborn /workspace \
    && chown -R ironclaw:ironclaw /home/ironclaw /data/ironclaw-reborn /workspace \
    && chmod +x /usr/local/bin/ironclaw-reborn-entrypoint /usr/local/bin/ironclaw-reborn-start-sshd

WORKDIR /workspace

EXPOSE 3000 2222

# The container starts as root; ironclaw-reborn-entrypoint performs the ONLY
# privilege drop (exec gosu ironclaw). Anything that bypasses the entrypoint —
# `docker run --entrypoint`, `docker exec` without `--user`, or a platform
# custom start command — runs as root instead.
USER root

ENTRYPOINT ["ironclaw-reborn-entrypoint"]
