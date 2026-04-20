# GitHub Actions self-hosted runner for the `private-oauth` live-canary lane.
# Deployed on Railway (or any container host with a static outbound IP and a
# persistent volume mounted at /runner-data). See README.md.
#
# Rust toolchain and wasm32-wasip2 target are intentionally NOT installed in
# the image — the workflow provisions them per-job via dtolnay/rust-toolchain,
# cached onto the persistent volume via RUSTUP_HOME / CARGO_HOME.

# syntax=docker/dockerfile:1.7
FROM ubuntu:22.04

ARG DEBIAN_FRONTEND=noninteractive
ARG RUNNER_VERSION=2.321.0

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        gnupg \
        jq \
        libssl-dev \
        pkg-config \
        tar \
    && rm -rf /var/lib/apt/lists/*

# gh CLI — the lane's failure-issue step invokes `gh issue create`.
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

# All runtime state lives under /runner-data. Mount a Railway volume here so
# the runner binary (which self-updates in place), the cargo/rustup caches,
# and — critically — the ironclaw libsql DB that holds rotated OAuth refresh
# tokens all survive container restarts and deploys.
ENV RUNNER_VERSION=${RUNNER_VERSION} \
    RUNNER_DATA=/runner-data \
    HOME=/runner-data/home \
    RUNNER_TOOL_CACHE=/runner-data/tool-cache \
    RUNNER_TEMP=/runner-data/tmp \
    CARGO_HOME=/runner-data/home/.cargo \
    RUSTUP_HOME=/runner-data/home/.rustup \
    RUNNER_ALLOW_RUNASROOT=1

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

WORKDIR /runner-data

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
