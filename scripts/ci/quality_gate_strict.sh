#!/usr/bin/env bash
set -euo pipefail

# Ensure we are running from the repository root
cd "$(git rev-parse --show-toplevel)"

require_command() {
    local command_name="$1"
    local install_hint="$2"
    if ! command -v "$command_name" &>/dev/null; then
        echo "ERROR: $command_name not installed ($install_hint)" >&2
        exit 1
    fi
}

require_node_22() {
    require_command node "install Node.js 22"
    local node_version
    node_version="$(node --version)"
    local node_major="${node_version#v}"
    node_major="${node_major%%.*}"
    if [ "$node_major" != "22" ]; then
        echo "ERROR: Node.js 22.x required for WebUI bundle builds; found $node_version" >&2
        exit 1
    fi
    echo "$node_version"
}

echo "==> fmt check"
cargo fmt --all -- --check

echo "==> WebUI bundle toolchain"
require_node_22
if ! command -v pnpm &>/dev/null; then
    require_command corepack "install Node.js Corepack or pnpm"
    corepack enable pnpm
fi
require_command pnpm "enable with: corepack enable pnpm"
pnpm --version

echo "==> WebUI frontend build"
(
    cd crates/ironclaw_webui/frontend
    pnpm install --frozen-lockfile
    pnpm build
)

echo "==> clippy (all warnings)"
cargo clippy --locked --all --benches --tests --examples --all-features -- -D warnings

# Feature-matrix leg: the libsql-only build surfaces `cfg`/`dead_code` lints
# that the all-features build hides (a variant constructed only under other
# features reads as never-constructed here). This is the class that reds
# `Clippy (libsql-only)` on main — e.g. the `Prebuilt` dead_code break (#5840).
echo "==> clippy (libsql-only feature leg)"
cargo clippy --locked --no-default-features --features libsql --all-targets -- -D warnings

echo "==> static: include_str! paths + Docker COPY coverage"
"$(git rev-parse --show-toplevel)/scripts/ci/check-include-str-paths.sh"

echo "==> cargo deny"
require_command cargo-deny "install with: cargo install cargo-deny"
cargo deny check

echo "==> tests"
cargo test --locked
