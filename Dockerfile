# Multi-stage Dockerfile for the IronClaw agent (cloud deployment).
#
# Uses cargo-chef for dependency caching — only rebuilds deps when
# Cargo.toml/Cargo.lock change, not on every source edit.
#
# Debian-based build + runtime. The bundled libSQL/SQLite C code has
# threading issues when statically linked against musl (segfault on
# database reopen), so we use glibc.
#
# Build:
#   docker build --platform linux/amd64 -t ironclaw:latest .
#
# Run:
#   docker run --env-file .env -p 3000:3000 ironclaw:latest

# Stage 1: Install cargo-chef
FROM rust:1.92-bookworm AS chef

RUN rustup target add wasm32-wasip2 \
    && cargo install cargo-chef@0.1.77 wasm-tools@1.246.1

WORKDIR /app

# Stage 2: Generate the dependency recipe (changes only when Cargo.toml/lock change)
FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY build.rs build.rs
COPY src/ src/
COPY tests/ tests/
COPY migrations/ migrations/
COPY registry/ registry/
COPY channels-src/ channels-src/
COPY wit/ wit/
COPY providers.json providers.json

RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Build dependencies (cached unless Cargo.toml/lock change)
FROM chef AS deps

# Docker-only overrides for the dist profile (not in Cargo.toml because
# cargo-dist uses dist for release binaries that need unwinding).
ENV CARGO_PROFILE_DIST_PANIC=abort \
    CARGO_PROFILE_DIST_CODEGEN_UNITS=1

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --profile dist --recipe-path recipe.json

# Stage 4: Build the actual binary (only recompiles ironclaw source)
FROM deps AS builder

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY build.rs build.rs
COPY src/ src/
COPY tests/ tests/
COPY migrations/ migrations/
COPY registry/ registry/
COPY channels-src/ channels-src/
COPY wit/ wit/
COPY providers.json providers.json

RUN cargo build --profile dist --bin ironclaw

# Stage 5: Minimal runtime
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/dist/ironclaw /usr/local/bin/ironclaw
COPY --from=builder /app/migrations /app/migrations

# Non-root user
ENV HOME=/home/ironclaw
RUN useradd -m -d /home/ironclaw -u 1000 ironclaw \
    && mkdir -p /home/ironclaw/.ironclaw \
    && chown -R ironclaw:ironclaw /home/ironclaw
WORKDIR /home/ironclaw
USER ironclaw

EXPOSE 3000

ENV RUST_LOG=ironclaw=info

ENTRYPOINT ["ironclaw"]
