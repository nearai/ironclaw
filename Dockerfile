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
COPY tools-src/ tools-src/
COPY wit/ wit/

# Build the main binary
RUN cargo build --release --no-default-features --features libsql --bin ironclaw

# Build WASM tools and install with canonical registry names.
# discover_tools() scans ~/.ironclaw/tools/ for *.wasm at the top level.
# The registry JSON filename is the canonical extension name; the crate
# produces a different filename (e.g. searxng_search_tool.wasm vs
# searxng_search.wasm). Rename to canonical name so that
# reconcile_extensions recognises the tool as already installed.
RUN mkdir -p /home/ironclaw/.ironclaw/tools \
    && for tool_dir in tools-src/*/; do \
         crate_name=$(basename "$tool_dir"); \
         if [ -f "$tool_dir/Cargo.toml" ]; then \
           echo "Building WASM tool: $crate_name" \
           && cargo build --release --target wasm32-wasip2 --manifest-path "$tool_dir/Cargo.toml" 2>&1 \
           && wasm_file=$(find "$tool_dir/target/wasm32-wasip2/release" -maxdepth 1 -name "*.wasm" | head -1) \
           && if [ -n "$wasm_file" ]; then \
                registry_name="" \
                && for rj in registry/tools/*.json; do \
                     rj_crate=$(sed -n 's/.*"crate_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$rj" | head -1) \
                     && if [ "$rj_crate" = "$crate_name" ]; then \
                          registry_name=$(basename "$rj" .json); \
                          break; \
                        fi; \
                   done \
                && if [ -z "$registry_name" ]; then \
                     registry_name=$(basename "$wasm_file" .wasm); \
                   fi \
                && cp "$wasm_file" "/home/ironclaw/.ironclaw/tools/${registry_name}.wasm" \
                && cap_file=$(find "$tool_dir" -maxdepth 1 -name "*.capabilities.json" | head -1) \
                && if [ -n "$cap_file" ]; then \
                     cp "$cap_file" "/home/ironclaw/.ironclaw/tools/${registry_name}.capabilities.json"; \
                   fi; \
                echo "  OK: $crate_name -> $registry_name"; \
              else \
                echo "  SKIP: $crate_name (no .wasm output)"; \
              fi; \
         fi; \
       done

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ironclaw /usr/local/bin/ironclaw
COPY --from=builder /home/ironclaw/.ironclaw/tools/ /home/ironclaw/.ironclaw/tools/

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
