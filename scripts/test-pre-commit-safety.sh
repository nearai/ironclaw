#!/usr/bin/env bash
# Regression tests for the grep pipelines in `pre-commit-safety.sh`.
#
# The PROJECTION / DISPATCH / CREDNAME checks all pipe through
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
    "src/auth_tests.rs" \
    "fn existing_test() {}\n" \
    "fn existing_test() {}\n#[test]\nfn new_test() { Result::<(), &str>::Ok(()).expect(\"fixture\"); }\n"

assert_precommit_allows "PANIC: sibling test_*.rs files are test-only" \
    "src/test_auth.rs" \
    "fn existing_test() {}\n" \
    "fn existing_test() {}\n#[test]\nfn new_test() { Result::<(), &str>::Ok(()).expect(\"fixture\"); }\n"

assert_precommit_allows "PANIC: sibling *_test.rs files are test-only" \
    "src/auth_test.rs" \
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

# ── PROJECTION ────────────────────────────────────────────────
# Positive: any `.broadcast_for_user(` (SseManager-unique method) or
#           `sse.broadcast(` with a portable word boundary.
# Exclusions: `// projection-exempt: <category>, <detail>`, `// safety:`,
#             and diff-header lines (`+++ b/path`) via `:\+\+\+ `.
PROJ_POS='(\.broadcast_for_user|(^|[^[:alnum:]_])sse\.broadcast)[[:space:]]*\('
PROJ_NEG='// projection-exempt: [^,]+,[[:space:]]*[^[:space:]]|// safety:|:\+\+\+ '

# Diff header lines must be filtered.
assert_filtered "PROJECTION: diff header line is filtered" \
    "+++ b/src/bridge/router.rs" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# A real broadcast call is flagged.
assert_flagged "PROJECTION: bare sse.broadcast_for_user is flagged" \
    "+    sse.broadcast_for_user(&user, event);" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# Chained receiver (state.sse.broadcast_for_user) is flagged.
assert_flagged "PROJECTION: chained state.sse.broadcast_for_user is flagged" \
    "+    state.sse.broadcast_for_user(&user, event);" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# A rustfmt-wrapped call is flagged.
assert_flagged "PROJECTION: rustfmt-wrapped .broadcast_for_user is flagged" \
    "+    .broadcast_for_user(&user, event);" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# Non-`sse` receiver must still fire — `broadcast_for_user` is unique to
# SseManager, so the method name alone is authoritative.
assert_flagged "PROJECTION: non-sse receiver .broadcast_for_user is flagged" \
    "+    manager.broadcast_for_user(&user, event);" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# Plain sse.broadcast call is flagged via the portable word boundary.
assert_flagged "PROJECTION: bare sse.broadcast is flagged" \
    "+    sse.broadcast(event);" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# The portable boundary must not fire on a longer identifier that ends
# in 'sse' (e.g. `usse.broadcast(...)` — not a real SseManager).
assert_filtered "PROJECTION: identifier ending in sse is not flagged" \
    "+    usse.broadcast(event);" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# Correctly annotated call is exempted.
assert_filtered "PROJECTION: annotated call with category+detail is exempt" \
    "+    sse.broadcast_for_user(&user, event); // projection-exempt: bridge dispatcher, auth gate" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# Bare `// projection-exempt: legacy` (no comma, no detail) does NOT exempt.
assert_flagged "PROJECTION: unnamed 'legacy' suppression still flagged" \
    "+    sse.broadcast_for_user(&user, event); // projection-exempt: legacy" \
    "$PROJ_POS" \
    "$PROJ_NEG"

# Empty detail after the comma (`// projection-exempt: foo,`) does NOT
# exempt — the documented format requires a non-empty detail.
assert_flagged "PROJECTION: empty detail after comma still flagged" \
    "+    sse.broadcast_for_user(&user, event); // projection-exempt: foo," \
    "$PROJ_POS" \
    "$PROJ_NEG"

# Trailing whitespace after the comma without a detail also does NOT exempt.
assert_flagged "PROJECTION: comma + whitespace-only detail still flagged" \
    "+    sse.broadcast_for_user(&user, event); // projection-exempt: foo,   " \
    "$PROJ_POS" \
    "$PROJ_NEG"

# ── DISPATCH ──────────────────────────────────────────────────
DISPATCH_POS='state\.(store|workspace|workspace_pool|extension_manager|skill_registry|session_manager)\.'
DISPATCH_NEG='// dispatch-exempt:|// safety:|:\+\+\+ '

assert_filtered "DISPATCH: diff header line is filtered" \
    "+++ b/src/channels/web/handlers/foo.rs" \
    "$DISPATCH_POS" \
    "$DISPATCH_NEG"

assert_flagged "DISPATCH: direct state.store touch is flagged" \
    "+    state.store.create_project(...)" \
    "$DISPATCH_POS" \
    "$DISPATCH_NEG"

# ── CREDNAME ──────────────────────────────────────────────────
# Portable word boundary: `(^|[^[:alnum:]_])` / `([^[:alnum:]_]|$)` —
# `grep -E`'s `\b` is a GNU extension and not recognised by BSD grep.
CREDNAME_POS='(^|[^[:alnum:]_])CredentialName([^[:alnum:]_]|$)'
CREDNAME_NEG='// web-identity-exempt:|// safety:|:\+\+\+ '

assert_filtered "CREDNAME: diff header line is filtered" \
    "+++ b/src/channels/web/features/settings.rs" \
    "$CREDNAME_POS" \
    "$CREDNAME_NEG"

# A similarly-named but distinct identifier must not fire.
assert_filtered "CREDNAME: CredentialNameExt (different type) is not flagged" \
    "+    let ext: CredentialNameExt = ...;" \
    "$CREDNAME_POS" \
    "$CREDNAME_NEG"

assert_flagged "CREDNAME: bare CredentialName reference is flagged" \
    "+    let name: CredentialName = ...;" \
    "$CREDNAME_POS" \
    "$CREDNAME_NEG"

# ── MULTITENANT ───────────────────────────────────────────────
# A new unscoped `sse.broadcast(...)` must either be transport-only or
# carry an explicit `// multi-tenant-safe: <reason>` annotation.
# `broadcast_for_user(...)` is the safe path and must be exempt.
MT_POS='(^|[^[:alnum:]_])sse\.broadcast[[:space:]]*\('
MT_NEG='\.broadcast_for_user|// projection-exempt: transport-only,[[:space:]]*[^[:space:]]|//.*multi-tenant-safe: [^[:space:]]|// safety:|:\+\+\+ '

assert_filtered "MULTITENANT: diff header line is filtered" \
    "+++ b/src/extensions/manager.rs" \
    "$MT_POS" \
    "$MT_NEG"

assert_filtered "MULTITENANT: broadcast_for_user is exempt" \
    "+    sse.broadcast_for_user(&user, event);" \
    "$MT_POS" \
    "$MT_NEG"

assert_filtered "MULTITENANT: heartbeat (transport-only) is exempt" \
    "+    sse.broadcast(AppEvent::Heartbeat); // projection-exempt: transport-only, heartbeat" \
    "$MT_POS" \
    "$MT_NEG"

# Receiver-prefixed call sites: the boundary regex matches `.sse.broadcast(`
# because the leading `.` is non-alnum-and-non-underscore, so the existing
# check covers production patterns like `state.sse.broadcast(`,
# `gw_state.sse.broadcast(`, and rustfmt-wrapped chains. These tests pin
# that behaviour against a future regex tightening.
assert_flagged "MULTITENANT: state.sse.broadcast (receiver-prefixed) is flagged" \
    "+    state.sse.broadcast(event);" \
    "$MT_POS" \
    "$MT_NEG"

assert_flagged "MULTITENANT: gw_state.sse.broadcast (snake_case receiver) is flagged" \
    "+    gw_state.sse.broadcast(event);" \
    "$MT_POS" \
    "$MT_NEG"

assert_filtered "MULTITENANT: state.sse.broadcast with annotation is exempt" \
    "+    state.sse.broadcast(event); // multi-tenant-safe: single-tenant fallback" \
    "$MT_POS" \
    "$MT_NEG"

assert_filtered "MULTITENANT: explicit multi-tenant-safe annotation is exempt" \
    "+    sse.broadcast(event); // multi-tenant-safe: only reached when multi_tenant_mode=false" \
    "$MT_POS" \
    "$MT_NEG"

# Compound annotation: a single `// ` comment can carry both
# `projection-exempt:` and `multi-tenant-safe:` because Rust line
# comments don't nest. The marker scanner must accept either marker
# anywhere in the trailing comment, not only when the comment opens
# with it. See `src/channels/web/mod.rs::dispatch_status_event` and
# `src/main.rs` sandbox JobEvent dispatcher.
assert_filtered "MULTITENANT: compound projection-exempt + multi-tenant-safe annotation is exempt" \
    "+    sse.broadcast(event); // projection-exempt: bridge dispatcher, single-tenant unscoped status; multi-tenant-safe: only reached when multi_tenant_mode=false" \
    "$MT_POS" \
    "$MT_NEG"

assert_flagged "MULTITENANT: bare unscoped sse.broadcast is flagged" \
    "+    sse.broadcast(event);" \
    "$MT_POS" \
    "$MT_NEG"

assert_flagged "MULTITENANT: unscoped broadcast with bridge-dispatcher projection-exempt is still flagged" \
    "+    sse.broadcast(event); // projection-exempt: bridge dispatcher, status update" \
    "$MT_POS" \
    "$MT_NEG"

assert_flagged "MULTITENANT: unscoped broadcast with empty multi-tenant-safe detail is still flagged" \
    "+    sse.broadcast(event); // multi-tenant-safe: " \
    "$MT_POS" \
    "$MT_NEG"

# ── ARCH-SPRAWL integration checks ────────────────────────────
assert_precommit_blocks "ARCH-SPRAWL: too_many_arguments allow needs arch-exempt plan" \
    "src/runtime.rs" \
    "fn existing() {}\n" \
    "#[allow(clippy::too_many_arguments)]\nfn execute(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) {}\n" \
    "ARCH-SPRAWL"

assert_precommit_allows "ARCH-SPRAWL: annotated too_many_arguments allow is accepted" \
    "src/runtime.rs" \
    "fn existing() {}\n" \
    "// arch-exempt: too_many_args, needs RuntimeInputs bundle, plan #2800\n#[allow(clippy::too_many_arguments)]\nfn execute(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) {}\n"

assert_precommit_allows "ARCH-SPRAWL: exemption reason may contain commas" \
    "src/runtime.rs" \
    "fn existing() {}\n" \
    "// arch-exempt: too_many_args, split struct, add builder, plan #2800\n#[allow(clippy::too_many_arguments)]\nfn execute(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) {}\n"

assert_precommit_blocks "ARCH-SPRAWL: optional Arc plus with-builder needs arch-exempt plan" \
    "src/runtime.rs" \
    "fn existing() {}\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\nimpl Runtime {\n    fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self {\n        self.baz = Some(baz);\n        self\n    }\n}\n" \
    "ARCH-SPRAWL"

assert_precommit_allows "ARCH-SPRAWL: annotated optional Arc plus with-builder is accepted" \
    "src/runtime.rs" \
    "fn existing() {}\n" \
    "use std::sync::Arc;\ntrait Baz {}\n// arch-exempt: optional_arc, feature-gated runtime adapter, plan #2800\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\nimpl Runtime {\n    fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self {\n        self.baz = Some(baz);\n        self\n    }\n}\n"

assert_precommit_allows "ARCH-SPRAWL: optional Arc and unrelated with-builder in separate hunks are allowed" \
    "src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n}\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n    fn with_cache(mut self, enabled: bool) -> Self {\n        self\n    }\n}\n"

assert_precommit_blocks "ARCH-SPRAWL: same-name optional Arc and with-builder in separate hunks still block" \
    "src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n}\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n    fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self {\n        self.baz = Some(baz);\n        self\n    }\n}\n" \
    "ARCH-SPRAWL"

assert_precommit_allows "ARCH-SPRAWL: annotated optional Arc and with-builder in separate hunks are accepted" \
    "src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n}\n" \
    "use std::sync::Arc;\ntrait Baz {}\n// arch-exempt: optional_arc, feature-gated runtime adapter, plan #2800\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\n\n\n\n\n\n\n\n\nimpl Runtime {\n    fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self {\n        self.baz = Some(baz);\n        self\n    }\n}\n"

assert_precommit_allows "ARCH-SPRAWL: optional Arc and unrelated with-builder in same hunk are allowed" \
    "src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n}\nimpl Runtime {\n}\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime {\n    baz: Option<Arc<dyn Baz>>,\n}\nimpl Runtime {\n    fn with_cache(mut self, enabled: bool) -> Self {\n        self\n    }\n}\n"

assert_precommit_allows_unstaged_diff "ARCH-SPRAWL: existing optional Arc pair near unrelated edit is allowed" \
    "src/runtime.rs" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime { baz: Option<Arc<dyn Baz>> }\nimpl Runtime { fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self { self } }\n" \
    "use std::sync::Arc;\ntrait Baz {}\nstruct Runtime { baz: Option<Arc<dyn Baz>> }\nimpl Runtime { fn with_baz(mut self, baz: Arc<dyn Baz>) -> Self { self } }\nfn unrelated() {}\n"

large_base=""
for i in $(seq 1 1501); do
    large_base="${large_base}// existing ${i}
"
done
assert_precommit_blocks "ARCH-SPRAWL: large file additions need arch-exempt plan" \
    "src/large.rs" \
    "$large_base" \
    "${large_base}// added line
" \
    "ARCH-SPRAWL"

large_base_with_exempt="// arch-exempt: large_file, tracked decomposition, plan #2800
${large_base}"
assert_precommit_allows "ARCH-SPRAWL: existing large-file exemption covers later edits" \
    "src/large.rs" \
    "$large_base_with_exempt" \
    "${large_base_with_exempt}// added line
"

assert_precommit_allows_with_unstaged "ARCH-SPRAWL: large-file check uses staged content, not unstaged working tree" \
    "src/large.rs" \
    "$large_base_with_exempt" \
    "${large_base_with_exempt}// staged added line
" \
    "${large_base}// unstaged edit removed exemption
"

assert_precommit_allows "ARCH-SPRAWL: single dispatcher call is allowed" \
    "src/runtime.rs" \
    "fn existing() {}\n" \
    "fn execute(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n"

assert_precommit_blocks "ARCH-SPRAWL: repeated dispatcher calls need arch-exempt plan" \
    "src/runtime.rs" \
    "fn existing() {}\n" \
    "fn execute_one(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\nfn execute_two(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n" \
    "ARCH-SPRAWL"

assert_precommit_blocks "ARCH-SPRAWL: repeated dispatcher calls in separate hunks need arch-exempt plan" \
    "src/runtime.rs" \
    "fn existing() {}\n\n\n\n\n\n\n\n\n" \
    "fn existing() {}\nfn execute_one(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n\n\n\n\n\n\n\n\nfn execute_two(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n" \
    "ARCH-SPRAWL"

assert_precommit_allows "ARCH-SPRAWL: annotated repeated dispatcher calls are accepted" \
    "src/runtime.rs" \
    "fn existing() {}\n" \
    "// arch-exempt: parallel_dispatch, temporary split while gateway lands, plan #2800\nfn execute_one(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\nfn execute_two(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n"

assert_precommit_allows "ARCH-SPRAWL: annotated repeated dispatcher calls in separate hunks are accepted" \
    "src/runtime.rs" \
    "fn existing() {}\n\n\n\n\n\n\n\n\n" \
    "fn existing() {}\n// arch-exempt: parallel_dispatch, temporary split while gateway lands, plan #2800\nfn execute_one(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n\n\n\n\n\n\n\n\nfn execute_two(dispatcher: &Dispatcher, request: Request) {\n    dispatcher.dispatch(request);\n}\n"

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
