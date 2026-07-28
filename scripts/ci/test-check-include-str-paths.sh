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

# ── Case 8: braceless #[cfg(test)] item must not swallow following code ─────
# A single-line `#[cfg(test)] use ...;` has no braces; the span scan must stop
# at the `;`, not run forward to the next unrelated `{` and mark a real
# (non-test) include_str! below it as test-only. Here the include_str! is
# compiled by cargo build and its target is NOT COPYd, so it must be blocked.
T8="$TMP/case8"
mk "$T8" "src/hooks/summary.rs" '#[cfg(test)]\nuse std::fmt;\n\nfn real() {\n    let _ = include_str!("../../prompts/needed.md");\n}\n'
mk "$T8" "prompts/needed.md" "hi\n"
mk "$T8" "Dockerfile" 'FROM rust AS builder\nCOPY src/ src/\nRUN cargo build --bin ironclaw\n'
assert_blocks "braceless cfg(test) does not hide a real include_str!" "$T8" "never COPYs \`prompts\`"

# ── Case 9: narrowed COPY of one crate must not cover a sibling crate ───────
# `COPY crates/foo/` copies only crates/foo; a prompt referenced from
# crates/bar is NOT in the build context and must be flagged (top-segment
# matching would wrongly treat all of `crates/` as covered).
T9="$TMP/case9"
mk "$T9" "crates/bar/src/lib.rs" 'const Q: &str = include_str!("../assets/x.md");\n'
mk "$T9" "crates/bar/assets/x.md" "y\n"
mk "$T9" "crates/foo/src/lib.rs" 'fn f() {}\n'
mk "$T9" "Dockerfile" 'FROM rust AS builder\nCOPY crates/foo/ crates/foo/\nRUN cargo build --bin ironclaw\n'
assert_allows "narrowed COPY crates/foo/ does not compile crates/bar (not blamed)" "$T9"

# ── Case 10: narrowed COPY covers its own crate's nested prompt ────────────
T10="$TMP/case10"
mk "$T10" "crates/foo/src/lib.rs" 'const Q: &str = include_str!("../prompts/p.md");\n'
mk "$T10" "crates/foo/prompts/p.md" "y\n"
mk "$T10" "Dockerfile" 'FROM rust AS builder\nCOPY crates/foo/ crates/foo/\nRUN cargo build --bin ironclaw\n'
assert_allows "narrowed COPY crates/foo/ covers crates/foo's own nested prompt" "$T10"

# ── Case 11: root build.rs include_str! target must be covered ─────────────
# `cargo build` compiles the repo-root build.rs; a prompt it reads that is not
# COPYd fails the Docker build exactly like a src/ reference.
T11="$TMP/case11"
mk "$T11" "build.rs" 'fn main() { let _ = include_str!("prompts/gen.md"); }\n'
mk "$T11" "prompts/gen.md" "hi\n"
mk "$T11" "Dockerfile" 'FROM rust AS builder\nCOPY build.rs build.rs\nRUN cargo build --bin ironclaw\n'
assert_blocks "root build.rs include_str! missing from COPY is flagged" "$T11" "never COPYs \`prompts\`"

# ── Case 12: include_str! target resolving outside the repo is flagged ─────
# The target exists on the host but lives outside the repo root, so no Docker
# COPY can include it — the image build would fail. `outside_ext.md` is a
# sibling of the repo tree; `../../../outside_ext.md` from src/hooks/ climbs
# above the repo root to reach it.
T12="$TMP/case12"
mk "$T12" "src/hooks/summary.rs" 'const P: &str = include_str!("../../../outside_ext.md");\n'
mk "$T12" "Dockerfile" 'FROM rust AS builder\nCOPY src/ src/\nRUN cargo build --bin ironclaw\n'
printf 'x\n' > "$TMP/outside_ext.md"
assert_blocks "include_str! target outside the repo is flagged" "$T12" "outside the build context"

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
