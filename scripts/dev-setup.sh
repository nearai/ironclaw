#!/usr/bin/env bash
# Developer setup script for IronClaw.
#
# Gets a fresh checkout ready for development without requiring
# Docker, PostgreSQL, or any external services. Installs repo-local git
# hooks via core.hooksPath so pre-push uses the hermetic local gate.
#
# Usage:
#   ./scripts/dev-setup.sh
#
# After running, you can:
#   cargo check                         # default features (postgres + libsql)
#   cargo test                          # default test suite (uses libsql temp DB)
#   scripts/ci/hermetic_gate.sh         # hermetic local gate used by pre-push
#   cargo test --all-features           # full test suite

set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== IronClaw Developer Setup ==="
echo ""

# 1. Check rustup
if ! command -v rustup &>/dev/null; then
    echo "ERROR: rustup not found. Install from https://rustup.rs"
    exit 1
fi
echo "[1/6] rustup found: $(rustup --version 2>/dev/null | head -1)"

# 2. Add WASM target (required by build.rs for channel compilation)
echo "[2/6] Adding wasm32-wasip2 target..."
rustup target add wasm32-wasip2

# 3. Install wasm-tools (required by build.rs for WASM component model)
echo "[3/6] Installing wasm-tools..."
if command -v wasm-tools &>/dev/null; then
    echo "  wasm-tools already installed: $(wasm-tools --version)"
else
    cargo install wasm-tools --locked
fi

# 4. Verify the project compiles
echo "[4/6] Running cargo check..."
cargo check

# 5. Run tests using libsql temp DB (no Docker/external DB needed)
echo "[5/6] Running tests (no external DB required)..."
cargo test

# 6. Install git hooks
echo "[6/6] Installing git hooks..."
if git rev-parse --git-dir >/dev/null 2>&1; then
    git config core.hooksPath .githooks
    echo "  core.hooksPath .githooks configured"
    echo "  pre-commit hook enabled (version bumps + safety checks)"
    echo "  commit-msg hook enabled (regression test enforcement)"
    echo "  pre-push hook enabled (hermetic gate + optional strict lint)"
else
    echo "  Skipped: not a git repository"
fi

echo ""
echo "=== Setup complete ==="
echo ""
echo "Quick start:"
echo "  cargo run                            # Run with default features"
echo "  cargo test                           # Test suite (libsql temp DB)"
echo "  scripts/ci/hermetic_gate.sh          # Hermetic local gate used by pre-push"
echo "  cargo test --all-features            # Full test suite"
echo "  cargo clippy --all-features          # Lint all code"
