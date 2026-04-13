# Default runtime Dockerfile for the IronClaw gateway image.
#
# Build:
#   docker build -t ironclaw .
#
# Run:
#   docker run --rm -p 3003:3003 ironclaw

FROM rust:1.92-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake gcc g++ \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-wasip2 \
    && cargo install wasm-tools

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY build.rs build.rs
COPY src/ src/
COPY tests/ tests/
COPY migrations/ migrations/
COPY registry/ registry/
COPY channels-src/ channels-src/
COPY wit/ wit/

RUN cargo build --release --no-default-features --features libsql --bin ironclaw

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ironclaw /usr/local/bin/ironclaw

RUN useradd -m -d /home/ironclaw -u 1000 ironclaw \
    && mkdir -p /home/ironclaw/.ironclaw \
    && chown -R ironclaw:ironclaw /home/ironclaw

ENV HOME=/home/ironclaw \
    RUST_LOG=ironclaw=info \
    GATEWAY_ENABLED=true \
    GATEWAY_HOST=0.0.0.0 \
    GATEWAY_PORT=3003 \
    GATEWAY_AUTH_TOKEN=test \
    DATABASE_BACKEND=libsql \
    LIBSQL_PATH=/home/ironclaw/test.db \
    SANDBOX_ENABLED=false

USER ironclaw
WORKDIR /home/ironclaw

EXPOSE 3003

ENTRYPOINT ["ironclaw", "--no-onboard"]
