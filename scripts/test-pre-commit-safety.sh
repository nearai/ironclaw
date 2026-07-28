#!/usr/bin/env bash
# Regression tests for the grep pipelines in `pre-commit-safety.sh`.
#
# The PANIC / ARCH-SPRAWL checks pipe through
# `grep -nE '^\+' | grep -E <positive> | grep -vE <exclusions>`.
# A previous version of the exclusion regex used `^\+\+\+` to filter
# diff header lines (`+++ b/file.rs`), which silently never fired —
# `grep -n` prepends a `N:` line-number prefix, so `^` no longer
# anchors against the `+++` bytes. This test locks in the corrected
# `:\+\+\+ ` shape.

set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PASS=0
FAIL=0

assert_filtered() {
    local label="$1" input="$2" positive="$3" exclusions="$4"
    # Emulate the production pipeline: `grep -n '^+'` adds the line-number
    # prefix, then positive/negative filters run against that shape.
    local result
    if result=$(printf '%s\n' "$input" \
        | grep -nE '^\+' \
        | grep -E "$positive" \
        | grep -vE "$exclusions" \
        | head -5 || true); then :; fi
    if [ -z "${result:-}" ]; then
        echo "OK: $label (correctly filtered)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $label — line leaked past exclusions:"
        echo "$result" | sed 's/^/    /'
        FAIL=$((FAIL + 1))
    fi
}

assert_flagged() {
    local label="$1" input="$2" positive="$3" exclusions="$4"
    local result
    if result=$(printf '%s\n' "$input" \
        | grep -nE '^\+' \
        | grep -E "$positive" \
        | grep -vE "$exclusions" \
        | head -5 || true); then :; fi
    if [ -n "${result:-}" ]; then
        echo "OK: $label (correctly flagged)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $label — line not flagged by positive pattern"
        FAIL=$((FAIL + 1))
    fi
}

assert_precommit_blocks() {
    local label="$1" path="$2" base_content="$3" changed_content="$4" expected="$5"
    local tmp output status
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/precommit-safety.XXXXXX")
    set +e
    (
        cd "$tmp"
        git init -q
        git config user.email test@example.com
        git config user.name "Test User"
        mkdir -p "$(dirname "$path")"
        printf '%b' "$base_content" > "$path"
        git add "$path"
        git commit -qm base
        printf '%b' "$changed_content" > "$path"
        git add "$path"
        set +e
        output=$("$ROOT_DIR/scripts/pre-commit-safety.sh" 2>&1)
        status=$?
        set -e
        if [ "$status" -ne 0 ] && printf '%s\n' "$output" | grep -Fq "$expected"; then
            echo "OK: $label (pre-commit blocked)"
            exit 0
        fi
        echo "FAIL: $label — expected block containing '$expected', got status $status"
        printf '%s\n' "$output" | sed 's/^/    /'
        exit 1
    )
    status=$?
    set -e
    rm -rf "$tmp"
    if [ "$status" -eq 0 ]; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
}

assert_precommit_allows() {
    local label="$1" path="$2" base_content="$3" changed_content="$4"
    local tmp output status
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/precommit-safety.XXXXXX")
    set +e
    (
        cd "$tmp"
        git init -q
        git config user.email test@example.com
        git config user.name "Test User"
        mkdir -p "$(dirname "$path")"
        printf '%b' "$base_content" > "$path"
        git add "$path"
        git commit -qm base
        printf '%b' "$changed_content" > "$path"
        git add "$path"
        set +e
        output=$("$ROOT_DIR/scripts/pre-commit-safety.sh" 2>&1)
        status=$?
        set -e
        if [ "$status" -eq 0 ]; then
            echo "OK: $label (pre-commit allowed)"
            exit 0
        fi
        echo "FAIL: $label — expected allow, got status $status"
        printf '%s\n' "$output" | sed 's/^/    /'
        exit 1
    )
    status=$?
    set -e
    rm -rf "$tmp"
    if [ "$status" -eq 0 ]; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
}

assert_precommit_allows_with_unstaged() {
    local label="$1" path="$2" base_content="$3" staged_content="$4" unstaged_content="$5"
    local tmp output status
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/precommit-safety.XXXXXX")
    set +e
    (
        cd "$tmp"
        git init -q
        git config user.email test@example.com
        git config user.name "Test User"
        mkdir -p "$(dirname "$path")"
        printf '%b' "$base_content" > "$path"
        git add "$path"
        git commit -qm base
        printf '%b' "$staged_content" > "$path"
        git add "$path"
        printf '%b' "$unstaged_content" > "$path"
        set +e
        output=$("$ROOT_DIR/scripts/pre-commit-safety.sh" 2>&1)
        status=$?
        set -e
        if [ "$status" -eq 0 ]; then
            echo "OK: $label (pre-commit allowed)"
            exit 0
        fi
        echo "FAIL: $label — expected allow, got status $status"
        printf '%s\n' "$output" | sed 's/^/    /'
        exit 1
    )
    status=$?
    set -e
    rm -rf "$tmp"
    if [ "$status" -eq 0 ]; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
}

# Rust commonly keeps large unit-test modules in a sibling `*_tests.rs` file
# that is included by a `#[cfg(test)] mod ...;` declaration. Those files are
# test-only even though they do not live under a `tests/` directory.
assert_precommit_allows "PANIC: sibling *_tests.rs files are test-only" \
    "crates/demo/src/auth_tests.rs" \
    "fn existing_test() {}\n" \
    "fn existing_test() {}\n#[test]\nfn new_test() { Result::<(), &str>::Ok(()).expect(\"fixture\"); }\n"

assert_precommit_allows "PANIC: sibling test_*.rs files are test-only" \
    "crates/demo/src/test_auth.rs" \
    "fn existing_test() {}\n" \
    "fn existing_test() {}\n#[test]\nfn new_test() { Result::<(), &str>::Ok(()).expect(\"fixture\"); }\n"

assert_precommit_allows "PANIC: sibling *_test.rs files are test-only" \
    "crates/demo/src/auth_test.rs" \
    "fn existing_test() {}\n" \
    "fn existing_test() {}\n#[test]\nfn new_test() { Result::<(), &str>::Ok(()).expect(\"fixture\"); }\n"

assert_precommit_allows_unstaged_diff() {
    local label="$1" path="$2" base_content="$3" changed_content="$4"
    local tmp output status
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/precommit-safety.XXXXXX")
    set +e
    (
        cd "$tmp"
        git init -q
        git config user.email test@example.com
        git config user.name "Test User"
        mkdir -p "$(dirname "$path")"
        printf '%b' "$base_content" > "$path"
        git add "$path"
        git commit -qm base
        git branch -M main
        printf '%b' "$changed_content" > "$path"
        set +e
        output=$("$ROOT_DIR/scripts/pre-commit-safety.sh" 2>&1)
        status=$?
        set -e
        if [ "$status" -eq 0 ]; then
            echo "OK: $label (standalone diff allowed)"
            exit 0
        fi
        echo "FAIL: $label — expected allow, got status $status"
        printf '%s\n' "$output" | sed 's/^/    /'
        exit 1
    )
    status=$?
    set -e
    rm -rf "$tmp"
    if [ "$status" -eq 0 ]; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
}


# ── ARCH-SPRAWL integration checks ────────────────────────────
assert_precommit_blocks "ARCH-SPRAWL: too_many_arguments allow needs arch-exempt plan" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n" \
    "#[allow(clippy::too_many_arguments)]\nfn execute(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) {}\n" \
    "ARCH-SPRAWL"

assert_precommit_allows "ARCH-SPRAWL: annotated too_many_arguments allow is accepted" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n" \
    "// arch-exempt: too_many_args, needs RuntimeInputs bundle, plan #2800\n#[allow(clippy::too_many_arguments)]\nfn execute(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) {}\n"

assert_precommit_allows "ARCH-SPRAWL: exemption reason may contain commas" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n" \
    "// arch-exempt: too_many_args, split struct, add builder, plan #2800\n#[allow(clippy::too_many_arguments)]\nfn execute(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) {}\n"

assert_precommit_blocks "ARCH-SPRAWL: optional Arc plus with-builder needs arch-exempt plan" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\nimpl Runtime {\n    fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self {\n        self.baz = Some(baz);\n        self\n    }\n}\n" \
    "ARCH-SPRAWL"

assert_precommit_allows "ARCH-SPRAWL: annotated optional Arc plus with-builder is accepted" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n" \
    "use std::sync::Arc;\ntrait Baz {}\n// arch-exempt: optional_arc, feature-gated runtime adapter, plan #2800\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\nimpl Runtime {\n    fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self {\n        self.baz = Some(baz);\n        self\n    }\n}\n"

assert_precommit_allows "ARCH-SPRAWL: optional Arc and unrelated with-builder in separate hunks are allowed" \
    "crates/demo/src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n}\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n    fn with_cache(mut self, enabled: bool) -> Self {\n        self\n    }\n}\n"

assert_precommit_blocks "ARCH-SPRAWL: same-name optional Arc and with-builder in separate hunks still block" \
    "crates/demo/src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n}\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n    fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self {\n        self.baz = Some(baz);\n        self\n    }\n}\n" \
    "ARCH-SPRAWL"

assert_precommit_allows "ARCH-SPRAWL: annotated optional Arc and with-builder in separate hunks are accepted" \
    "crates/demo/src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n}\n" \
    "use std::sync::Arc;\ntrait Baz {}\n// arch-exempt: optional_arc, feature-gated runtime adapter, plan #2800\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n    fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self {\n        self.baz = Some(baz);\n        self\n    }\n}\n"

assert_precommit_allows "ARCH-SPRAWL: optional Arc and unrelated with-builder in same hunk are allowed" \
    "crates/demo/src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n}\nimpl Runtime {\n}\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\nimpl Runtime {\n    fn with_cache(mut self, enabled: bool) -> Self {\n        self\n    }\n}\n"

assert_precommit_allows_unstaged_diff "ARCH-SPRAWL: existing optional Arc pair near unrelated edit is allowed" \
    "crates/demo/src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime { baz: Option<Arc<dyn Baz>> }\nimpl Runtime { fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self { self } }\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime { baz: Option<Arc<dyn Baz>> }\nimpl Runtime { fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self { self } }\nfn unrelated() {}\n"

large_base=""
for i in $(seq 1 1501); do
    large_base="${large_base}// existing ${i}
"
done
assert_precommit_blocks "ARCH-SPRAWL: large file additions need arch-exempt plan" \
    "crates/demo/src/large.rs" \
    "$large_base" \
    "${large_base}// added line
" \
    "ARCH-SPRAWL"

large_base_with_exempt="// arch-exempt: large_file, tracked decomposition, plan #2800
${large_base}"
assert_precommit_allows "ARCH-SPRAWL: existing large-file exemption covers later edits" \
    "crates/demo/src/large.rs" \
    "$large_base_with_exempt" \
    "${large_base_with_exempt}// added line
"

assert_precommit_allows_with_unstaged "ARCH-SPRAWL: large-file check uses staged content, not unstaged working tree" \
    "crates/demo/src/large.rs" \
    "$large_base_with_exempt" \
    "${large_base_with_exempt}// staged added line
" \
    "${large_base}// unstaged edit removed exemption
"

assert_precommit_allows "ARCH-SPRAWL: single dispatcher call is allowed" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n" \
    "fn execute(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n"

assert_precommit_blocks "ARCH-SPRAWL: repeated dispatcher calls need arch-exempt plan" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n" \
    "fn execute_one(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\nfn execute_two(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n" \
    "ARCH-SPRAWL"

assert_precommit_blocks "ARCH-SPRAWL: repeated dispatcher calls in separate hunks need arch-exempt plan" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n\n\n\n\n\n\n\n\n" \
    "fn existing() {}\nfn execute_one(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n\n\n\n\n\n\n\n\nfn execute_two(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n" \
    "ARCH-SPRAWL"

assert_precommit_allows "ARCH-SPRAWL: annotated repeated dispatcher calls are accepted" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n" \
    "// arch-exempt: parallel_dispatch, temporary split while gateway lands, plan #2800\nfn execute_one(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\nfn execute_two(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n"

assert_precommit_allows "ARCH-SPRAWL: annotated repeated dispatcher calls in separate hunks are accepted" \
    "crates/demo/src/runtime.rs" \
    "fn existing() {}\n\n\n\n\n\n\n\n\n" \
    "fn existing() {}\n// arch-exempt: parallel_dispatch, temporary split while gateway lands, plan #2800\nfn execute_one(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n\n\n\n\n\n\n\n\nfn execute_two(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n"

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
