FROM rust:1.86-slim-bookworm

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    git \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install WASM targets
RUN rustup target add wasm32-wasip2 wasm32-unknown-unknown

# Install wasm-tools for component manipulation
RUN cargo install wasm-tools --locked

# Create non-root user for sandbox
RUN useradd -m -u 1000 sandbox
USER sandbox

WORKDIR /workspace

# Default command
CMD ["bash"]
