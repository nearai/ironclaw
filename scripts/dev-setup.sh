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
#   cargo check           # default build
#   cargo test            # default test suite
#   cargo test --all-features         # full test suite (requires Node.js 22 + Corepack/pnpm for WebUI bundle)

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

# 6. Install git hooks — one story, worktree-safe. core.hooksPath makes git
# read hooks from the tracked .githooks/ directory in EVERY checkout of this
# repository (worktrees included: a relative core.hooksPath resolves against
# each worktree's own top-level directory), which the old absolute-path
# git-path-hooks symlinks never did — they pointed every worktree at
# whichever checkout last ran this script.
echo "[6/6] Installing git hooks..."
if git rev-parse --git-dir >/dev/null 2>&1; then
    git config core.hooksPath .githooks
    echo "  core.hooksPath -> .githooks (commit-msg, pre-commit incl. safety checks, tiered pre-push)"
    echo "  pre-push default: bash scripts/preflight-gates.sh + changed-package clippy (~5-6 min warm); full gauntlet: IRONCLAW_PREPUSH_FULL=1"
else
    echo "  Skipped: not a git repository"
fi

echo ""
# Codebase knowledge graph (codebase-memory MCP) — powers agent code discovery.
# Single static binary, no deps, no API keys, 100% local. See CLAUDE.md -> "Code Discovery".
if command -v codebase-memory-mcp &>/dev/null; then
    echo "[graph] codebase-memory-mcp found: $(command -v codebase-memory-mcp)"
else
    echo "[graph] Installing codebase-memory-mcp (agent code-discovery graph)..."
    if curl -fsSL https://raw.githubusercontent.com/DeusData/codebase-memory-mcp/main/install.sh | bash; then
        echo "  installed. The repo's .mcp.json wires it into Claude Code automatically."
    else
        echo "  WARN: install failed — agents will fall back to grep."
        echo "        Install manually: https://github.com/DeusData/codebase-memory-mcp"
    fi
fi

echo ""
echo "=== Setup complete ==="
echo ""
echo "Quick start:"
echo "  cargo run                            # Run the default build"
echo "  cargo test                           # Test suite"
echo "  cargo test --all-features            # Full test suite (requires Node.js 22 + Corepack/pnpm for WebUI bundle)"
echo "  cargo clippy --all-features          # Lint all code (requires Node.js 22 + Corepack/pnpm for WebUI bundle)"
