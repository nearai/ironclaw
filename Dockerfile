# syntax=docker/dockerfile:1.7

# Multi-stage Dockerfile for the IronClaw agent (cloud deployment).
#
# Build:
#   docker build --platform linux/amd64 -t ironclaw:latest .
#
# Run:
#   docker run --env-file .env -p 3000:3000 ironclaw:latest

# Stage 1: Build base
FROM rust:1.92-slim-bookworm AS chef

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake gcc g++ \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-wasip2 \
    && cargo install wasm-tools cargo-chef

WORKDIR /app

# Stage 1a: Generate cargo-chef recipe
FROM chef AS planner

# Copy manifests and project files needed to compute the dependency recipe
COPY Cargo.toml Cargo.lock ./
COPY build.rs build.rs
COPY src/ src/
COPY tests/ tests/
COPY migrations/ migrations/
COPY registry/ registry/
COPY channels-src/ channels-src/
COPY wit/ wit/

RUN cargo chef prepare --recipe-path recipe.json

# Stage 1b: Build with cached dependencies
FROM chef AS builder

COPY --from=planner /app/recipe.json /app/recipe.json

RUN --mount=type=cache,id=ironclaw-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ironclaw-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ironclaw-target,target=/app/target \
    cargo chef cook --release --bin ironclaw --recipe-path recipe.json

# Copy source, build script, tests, and supporting directories
COPY Cargo.toml Cargo.lock ./
COPY build.rs build.rs
COPY src/ src/
COPY tests/ tests/
COPY migrations/ migrations/
COPY registry/ registry/
COPY channels-src/ channels-src/
COPY wit/ wit/

RUN --mount=type=cache,id=ironclaw-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=ironclaw-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=ironclaw-target,target=/app/target \
    cargo build --release --bin ironclaw \
    && install -D /app/target/release/ironclaw /app-out/ironclaw

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app-out/ironclaw /usr/local/bin/ironclaw
COPY --from=builder /app/migrations /app/migrations

# Non-root user
RUN useradd -m -u 1000 -s /bin/bash ironclaw
USER ironclaw

EXPOSE 3000

ENV RUST_LOG=ironclaw=info

ENTRYPOINT ["ironclaw"]
