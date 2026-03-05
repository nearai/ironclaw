#!/bin/bash
set -e

# Playwright runner for IronClaw screenshot tests
# Env loading is handled by playwright.config.ts via dotenv
# Called by: pnpm screenshots / pnpm screenshots:list / pnpm screenshots:update

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCS_DIR="$(dirname "$SCRIPT_DIR")"

cd "$DOCS_DIR/tests"
exec pnpm exec playwright test "$@"
