# Multi-stage Dockerfile for the IronClaw agent (cloud deployment).
#
# Build:
#   docker build --platform linux/amd64 -t ironclaw:latest .
#
# Build with OTEL tracing:
#   docker build --build-arg CARGO_FEATURES="postgres,libsql,html-to-markdown,otel" -t ironclaw:latest .
#
# Run:
#   docker run --env-file .env -p 3001:3001 ironclaw:latest

# Stage 1: Build
FROM rust:1.92-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake gcc g++ protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-wasip2 \
    && cargo install wasm-tools

WORKDIR /app

# Copy manifests and build script first for layer caching
COPY Cargo.toml Cargo.lock build.rs ./

# Copy source, build script, tests, and supporting directories
COPY src/ src/
COPY tests/ tests/
COPY migrations/ migrations/
COPY registry/ registry/
COPY channels-src/ channels-src/
COPY wit/ wit/

# Default features match the standard build; override with --build-arg for OTEL etc.
ARG CARGO_FEATURES="postgres,libsql,html-to-markdown"
RUN cargo build --release --bin ironclaw --features "${CARGO_FEATURES}"

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ironclaw /usr/local/bin/ironclaw
COPY --from=builder /app/migrations /app/migrations

# Non-root user
RUN useradd -m -u 1000 -s /bin/bash ironclaw
USER ironclaw

EXPOSE 3000

ENV RUST_LOG=ironclaw=info

ENTRYPOINT ["ironclaw"]
