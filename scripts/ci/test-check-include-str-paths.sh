#!/usr/bin/env bash
# Regression tests for check-include-str-paths.sh.
#
# Builds synthetic repo trees and asserts the checker blocks/allows the right
# ones — including a direct reproduction of the #5603 Docker outage (a
# repo-root prompt referenced from `src/` but never COPYd into the builder).

set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT_DIR/scripts/ci/check-include-str-paths.sh"

PASS=0
FAIL=0

# Create a minimal tree: $1=dir. Populated by the caller before running.
run_check() {
    bash "$CHECK" "$1" 2>&1
}

assert_blocks() {
    local label="$1" tree="$2" expect="$3"
    local out status
    set +e
    out=$(run_check "$tree")
    status=$?
    set -e
    if [ "$status" -ne 0 ] && printf '%s\n' "$out" | grep -Fq "$expect"; then
        echo "OK: $label (blocked)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $label — expected block containing '$expect', got status $status"
        printf '%s\n' "$out" | sed 's/^/    /'
        FAIL=$((FAIL + 1))
    fi
}

assert_allows() {
    local label="$1" tree="$2"
    local out status
    set +e
    out=$(run_check "$tree")
    status=$?
    set -e
    if [ "$status" -eq 0 ]; then
        echo "OK: $label (allowed)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $label — expected allow, got status $status"
        printf '%s\n' "$out" | sed 's/^/    /'
        FAIL=$((FAIL + 1))
    fi
}

mk() { # mk <tree> <relpath> <content>
    mkdir -p "$1/$(dirname "$2")"
    printf '%b' "$3" > "$1/$2"
}

TMP=$(mktemp -d "${TMPDIR:-/tmp}/check-include-str.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

# ── Case 1: #5603 reproduction — repo-root prompt not COPYd ────────────────
T1="$TMP/case1"
mk "$T1" "src/hooks/summary.rs" 'const P: &str = include_str!("../../prompts/session_summary.md");\n'
mk "$T1" "prompts/session_summary.md" "hello\n"
mk "$T1" "Dockerfile" 'FROM rust AS builder\nCOPY src/ src/\nRUN cargo build --bin ironclaw\n'
assert_blocks "missing COPY prompts/ (the #5603 outage)" "$T1" "never COPYs \`prompts\`"

# ── Case 2: same tree, but Dockerfile now COPYs prompts/ ───────────────────
T2="$TMP/case2"
mk "$T2" "src/hooks/summary.rs" 'const P: &str = include_str!("../../prompts/session_summary.md");\n'
mk "$T2" "prompts/session_summary.md" "hello\n"
mk "$T2" "Dockerfile" 'FROM rust AS builder\nCOPY src/ src/\nCOPY prompts/ prompts/\nRUN cargo build --bin ironclaw\n'
assert_allows "prompts/ COPYd — build context complete" "$T2"

# ── Case 3: include_str! target does not exist at all ──────────────────────
T3="$TMP/case3"
mk "$T3" "src/hooks/summary.rs" 'const P: &str = include_str!("../../prompts/missing.md");\n'
mk "$T3" "Dockerfile" 'FROM rust AS builder\nCOPY . .\nRUN cargo build --bin ironclaw\n'
assert_blocks "nonexistent include_str! target" "$T3" "file not found"

# ── Case 4: test-only include_str! (#[cfg(test)]) is NOT required in image ──
T4="$TMP/case4"
mk "$T4" "src/lib.rs" '#[cfg(test)]\nmod tests {\n    const F: &str = include_str!("../fixtures/data.toml");\n}\n'
mk "$T4" "fixtures/data.toml" "x\n"
mk "$T4" "Dockerfile" 'FROM rust AS builder\nCOPY src/ src/\nRUN cargo build --bin ironclaw\n'
assert_allows "cfg(test) include_str! not enforced in build context" "$T4"

# ── Case 5: Dockerfile that never compiles src/ is not blamed ──────────────
# (models Dockerfile.reborn: builds only crates/, so a src/ prompt is moot)
T5="$TMP/case5"
mk "$T5" "src/hooks/summary.rs" 'const P: &str = include_str!("../../prompts/session_summary.md");\n'
mk "$T5" "prompts/session_summary.md" "hello\n"
mk "$T5" "crates/foo/src/lib.rs" 'const Q: &str = include_str!("../assets/x.md");\n'
mk "$T5" "crates/foo/assets/x.md" "y\n"
mk "$T5" "Dockerfile.reborn" 'FROM rust AS builder\nCOPY crates/ crates/\nRUN cargo build --bin ironclaw-reborn\n'
assert_allows "Dockerfile that omits src/ is not blamed for a src/ prompt" "$T5"

# ── Case 6: crate-local prompt under crates/ is fine (top segment covered) ──
T6="$TMP/case6"
mk "$T6" "crates/foo/src/lib.rs" 'const Q: &str = include_str!("../prompts/p.md");\n'
mk "$T6" "crates/foo/prompts/p.md" "y\n"
mk "$T6" "Dockerfile" 'FROM rust AS builder\nCOPY crates/ crates/\nRUN cargo build --bin ironclaw\n'
assert_allows "crate-local prompt covered by COPY crates/" "$T6"

# ── Case 7: full-context COPY . . is never blamed ──────────────────────────
T7="$TMP/case7"
mk "$T7" "src/hooks/summary.rs" 'const P: &str = include_str!("../../prompts/session_summary.md");\n'
mk "$T7" "prompts/session_summary.md" "hello\n"
mk "$T7" "Dockerfile" 'FROM rust AS builder\nCOPY . .\nRUN cargo build --bin ironclaw\n'
assert_allows "COPY . . copies the whole context" "$T7"

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
