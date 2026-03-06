#!/usr/bin/env bash
# Generate an HTML coverage report for a given set of tests.
#
# Usage:
#   ./scripts/coverage.sh                          # all tests (lib only)
#   ./scripts/coverage.sh safety                   # tests matching "safety"
#   ./scripts/coverage.sh safety::sanitizer        # specific module tests
#   ./scripts/coverage.sh test_a test_b test_c     # multiple test filters
#
# Options (env vars):
#   COV_OPEN=1          Auto-open the report in a browser (default: 1)
#   COV_FORMAT=html     Output format: html, text, json, lcov (default: html)
#   COV_OUT=coverage    Output directory (default: coverage/)
#   COV_FEATURES=""     Extra --features to pass (default: none)
#   COV_ALL_TARGETS=0   Set to 1 to include integration tests (default: lib only)
#
# Requires: cargo-llvm-cov (install: cargo install cargo-llvm-cov)

set -euo pipefail

COV_OPEN="${COV_OPEN:-1}"
COV_FORMAT="${COV_FORMAT:-html}"
COV_OUT="${COV_OUT:-coverage}"
COV_FEATURES="${COV_FEATURES:-}"
COV_ALL_TARGETS="${COV_ALL_TARGETS:-0}"

cd "$(git rev-parse --show-toplevel)"

if ! command -v cargo-llvm-cov &>/dev/null; then
    echo "ERROR: cargo-llvm-cov not found. Install with: cargo install cargo-llvm-cov"
    exit 1
fi

# Clean stale profiling data to avoid "mismatched data" warnings.
cargo llvm-cov clean --workspace 2>/dev/null || true

# Build the cargo llvm-cov command
cmd=(cargo llvm-cov)

# Features
if [[ -n "$COV_FEATURES" ]]; then
    cmd+=(--features "$COV_FEATURES")
else
    cmd+=(--all-features)
fi

# By default, only run the lib unit tests (fast, no integration test compilation).
# Set COV_ALL_TARGETS=1 to include integration tests.
if [[ "$COV_ALL_TARGETS" != "1" ]]; then
    cmd+=(--lib)
fi

# Output format
case "$COV_FORMAT" in
    html)
        cmd+=(--html --output-dir "$COV_OUT")
        ;;
    text)
        cmd+=(--text)
        ;;
    json)
        cmd+=(--json --output-path "$COV_OUT/coverage.json")
        ;;
    lcov)
        cmd+=(--lcov --output-path "$COV_OUT/lcov.info")
        ;;
    *)
        echo "ERROR: Unknown format '$COV_FORMAT'. Use: html, text, json, lcov"
        exit 1
        ;;
esac

# Test name filters (passed after -- to cargo test)
if [[ $# -gt 0 ]]; then
    if [[ $# -eq 1 ]]; then
        cmd+=(-- "$1")
    else
        # Join filters with | for regex matching
        filter=$(IFS='|'; echo "$*")
        cmd+=(-- "$filter")
    fi
fi

echo "Running: ${cmd[*]}"
echo ""

"${cmd[@]}"

# Open report
if [[ "$COV_FORMAT" == "html" && "$COV_OPEN" == "1" ]]; then
    index="$COV_OUT/html/index.html"
    if [[ -f "$index" ]]; then
        echo ""
        echo "Report: $index"
        if command -v open &>/dev/null; then
            open "$index"
        elif command -v xdg-open &>/dev/null; then
            xdg-open "$index"
        fi
    fi
fi
