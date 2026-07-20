#!/usr/bin/env bash
# Regression tests for check-hermetic-env.sh.
#
# Each case builds a throwaway git repo, commits a base, stages a change, and
# asserts the checker blocks/allows it. Mirrors the temp-repo harness in
# test-pre-commit-safety.sh.

set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT_DIR/scripts/ci/check-hermetic-env.sh"

PASS=0
FAIL=0

# assert <mode: blocks|allows> <label> <path> <base> <staged>
assert() {
    local mode="$1" label="$2" path="$3" base="$4" staged="$5"
    local tmp out status
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/hermetic-env.XXXXXX")
    (
        cd "$tmp"
        git init -q
        git config user.email t@e.com
        git config user.name t
        git checkout -q -b main
        mkdir -p "$(dirname "$path")"
        printf '%b' "$base" > "$path"
        git add "$path"
        git commit -qm base
        printf '%b' "$staged" > "$path"
        git add "$path"
    )
    set +e
    out=$(cd "$tmp" && bash "$CHECK" 2>&1)
    status=$?
    set -e
    rm -rf "$tmp"
    local ok=0
    if [ "$mode" = "blocks" ] && [ "$status" -ne 0 ]; then ok=1; fi
    if [ "$mode" = "allows" ] && [ "$status" -eq 0 ]; then ok=1; fi
    if [ "$ok" -eq 1 ]; then
        echo "OK: $label ($mode)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $label — expected $mode, got status $status"
        printf '%s\n' "$out" | sed 's/^/    /'
        FAIL=$((FAIL + 1))
    fi
}

# ── Unguarded env mutation in a new test → blocked ─────────────────────────
assert blocks "unguarded set_var in a new test" "src/lib.rs" \
    "fn a() {}\n" \
    'fn a() {}\n#[test]\nfn t() {\n    unsafe { std::env::set_var("FOO", "1"); }\n    assert_eq!(1, 1);\n}\n'

# ── Guarded with lock_env() in same fn → allowed ───────────────────────────
assert allows "set_var guarded by lock_env()" "src/lib.rs" \
    "fn a() {}\n" \
    'fn a() {}\n#[test]\nfn t() {\n    let _g = lock_env();\n    unsafe { std::env::set_var("FOO", "1"); }\n}\n'

# ── Guarded by module lock_runtime_env() → allowed ─────────────────────────
assert allows "set_var guarded by lock_runtime_env()" "src/runtime.rs" \
    "fn a() {}\n" \
    'fn a() {}\n#[test]\nfn t() {\n    let _g = lock_runtime_env();\n    unsafe { std::env::remove_var("BAR"); }\n}\n'

# ── EnvGuard RAII wrapper present → allowed ────────────────────────────────
assert allows "mutation inside EnvGuard helper" "src/env.rs" \
    "fn a() {}\n" \
    'fn a() {}\nimpl EnvGuard {\n    fn set(k: &str) {\n        unsafe { std::env::set_var(k, "1"); }\n    }\n}\n'

# ── Inline suppression → allowed ───────────────────────────────────────────
assert allows "explicit // env-hermetic: annotation" "src/boot.rs" \
    "fn a() {}\n" \
    'fn a() {}\nfn boot() {\n    unsafe { std::env::set_var("DB", "libsql"); } // env-hermetic: startup, pre-thread-spawn\n}\n'

# ── Thread-safe set_runtime_env is not raw mutation → allowed ──────────────
assert allows "set_runtime_env (map API) is not flagged" "src/lib.rs" \
    "fn a() {}\n" \
    'fn a() {}\n#[test]\nfn t() {\n    set_runtime_env("FOO", "1");\n    remove_runtime_env("FOO");\n}\n'

# ── Pre-existing unguarded mutation, unrelated edit → allowed (delta-only) ──
assert allows "pre-existing unguarded set_var, unrelated new fn" "src/lib.rs" \
    'fn t() {\n    unsafe { std::env::set_var("FOO", "1"); }\n}\n' \
    'fn t() {\n    unsafe { std::env::set_var("FOO", "1"); }\n}\nfn unrelated() { let _ = 2; }\n'

# ── A guard token in a COMMENT must not exempt a raw mutation → blocked ─────
# The guard tokens (EnvGuard/lock_env/…) only serialize when they name real
# lock/RAII construction. A bare `// EnvGuard` comment beside raw set_var is
# not serialization and must NOT clear the hunk.
assert blocks "comment 'EnvGuard' does not exempt raw set_var" "src/lib.rs" \
    "fn a() {}\n" \
    'fn a() {}\n#[test]\nfn t() {\n    // EnvGuard would go here\n    unsafe { std::env::set_var("FOO", "1"); }\n}\n'

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
