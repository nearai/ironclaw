#!/usr/bin/env bash
# Developer setup script for IronClaw.
#
# Gets a fresh checkout ready for development without requiring
# Docker, PostgreSQL, or any external services.
#
# Usage:
#   ./scripts/dev-setup.sh
#
# After running, you can:
#   cargo check                              # default local profile (libsql)
#   cargo test                               # default test suite (uses libsql temp DB)
#   cargo test --all-features                # full test suite
#   cargo run --features local-tui           # local profile + TUI

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
HOOKS_DIR=$(git rev-parse --git-path hooks 2>/dev/null) || true
if [ -n "$HOOKS_DIR" ]; then
    mkdir -p "$HOOKS_DIR"
    SCRIPTS_ABS="$(cd "$(dirname "$0")" && pwd)"
    ln -sf "$SCRIPTS_ABS/commit-msg-regression.sh" "$HOOKS_DIR/commit-msg"
    echo "  commit-msg hook installed (regression test enforcement)"
    ln -sf "$SCRIPTS_ABS/pre-commit-safety.sh" "$HOOKS_DIR/pre-commit"
    echo "  pre-commit hook installed (UTF-8, case-sensitivity, /tmp, redaction checks)"
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    ln -sf "$REPO_ROOT/.githooks/pre-push" "$HOOKS_DIR/pre-push"
    echo "  pre-push hook installed (quality gate + optional delta lint)"
else
    echo "  Skipped: not a git repository"
fi

echo ""
echo "=== Setup complete ==="
echo ""
echo "Quick start:"
echo "  cargo run                            # Run with the default local profile (libsql)"
echo "  cargo run --features local-tui       # Add the optional TUI"
echo "  cargo test                           # Test suite (libsql temp DB)"
echo "  cargo test --all-features            # Full test suite"
echo "  cargo clippy --all-features          # Lint all code"
