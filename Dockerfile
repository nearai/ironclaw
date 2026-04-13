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
COPY tools-src/ tools-src/
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
COPY tools-src/ tools-src/
COPY wit/ wit/
COPY providers.json providers.json
COPY profiles/ profiles/

RUN cargo build --profile dist --bin ironclaw

# Stage 4b: Build all WASM extensions from source (only used by runtime-staging)
FROM builder AS wasm-builder
ARG CACHE_BUST

RUN apt-get update && apt-get install -y --no-install-recommends jq && rm -rf /var/lib/apt/lists/*
RUN echo "cache-bust=${CACHE_BUST}"

RUN set -eux; \
    mkdir -p /app/wasm-bundles/tools /app/wasm-bundles/channels; \
    for manifest in registry/tools/*.json registry/channels/*.json; do \
      [ -f "$manifest" ] || continue; \
      kind=$(jq -r '.kind' "$manifest"); \
      ext_name=$(jq -r '.name' "$manifest"); \
      source_dir=$(jq -r '.source.dir' "$manifest"); \
      caps_file=$(jq -r '.source.capabilities' "$manifest"); \
      crate_name=$(jq -r '.source.crate_name' "$manifest"); \
      [ -d "$source_dir" ] || continue; \
      # Telegram is embedded in the binary at build time; skip it
      [ "$ext_name" = "telegram" ] && continue; \
      echo "=== Building $ext_name from $source_dir ==="; \
      if [ -f "$source_dir/Cargo.lock" ]; then \
        CARGO_TARGET_DIR=/app/target cargo build --locked --release --target wasm32-wasip2 \
          --manifest-path "$source_dir/Cargo.toml" || { echo "WARN: build failed for $ext_name"; continue; }; \
      else \
        CARGO_TARGET_DIR=/app/target cargo build --release --target wasm32-wasip2 \
          --manifest-path "$source_dir/Cargo.toml" || { echo "WARN: build failed for $ext_name"; continue; }; \
      fi; \
      wasm_artifact=$(echo "${crate_name}" | tr '-' '_'); \
      raw_wasm="/app/target/wasm32-wasip2/release/${wasm_artifact}.wasm"; \
      [ -f "$raw_wasm" ] || continue; \
      dest_dir="/app/wasm-bundles/tools"; \
      [ "$kind" = "channel" ] && dest_dir="/app/wasm-bundles/channels"; \
      wasm-tools component new "$raw_wasm" -o "$dest_dir/${ext_name}.wasm" 2>/dev/null \
        || cp "$raw_wasm" "$dest_dir/${ext_name}.wasm"; \
      wasm-tools strip "$dest_dir/${ext_name}.wasm" -o "$dest_dir/${ext_name}.wasm.tmp" 2>/dev/null \
        && mv "$dest_dir/${ext_name}.wasm.tmp" "$dest_dir/${ext_name}.wasm" \
        || true; \
      [ -f "$source_dir/$caps_file" ] && cp "$source_dir/$caps_file" "$dest_dir/${ext_name}.capabilities.json"; \
      echo "  -> $dest_dir/${ext_name}.wasm"; \
    done; \
    count=$(find /app/wasm-bundles -name '*.wasm' | wc -l); \
    echo "Built $count WASM extensions"; \
    [ "$count" -gt 0 ] || { echo "ERROR: No WASM extensions were built"; exit 1; }

# Stage 5a: Shared runtime base
FROM debian:bookworm-slim AS runtime-base

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

EXPOSE 3000

ENV RUST_LOG=ironclaw=info

ENTRYPOINT ["ironclaw"]

# Stage 5b: Staging runtime (with pre-built WASM extensions)
FROM runtime-base AS runtime-staging
COPY --from=wasm-builder --chown=ironclaw:ironclaw /app/wasm-bundles/tools/ /home/ironclaw/.ironclaw/tools/
COPY --from=wasm-builder --chown=ironclaw:ironclaw /app/wasm-bundles/channels/ /home/ironclaw/.ironclaw/channels/
USER ironclaw

# Stage 5c: Production runtime (default — no pre-bundled extensions)
FROM runtime-base AS runtime
USER ironclaw
