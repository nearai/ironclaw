#!/usr/bin/env bash
set -euo pipefail

# Static check: newly-added raw `std::env::set_var(...)` / `remove_var(...)`
# must be guarded by the crate env lock (`lock_env()` / `lock_runtime_env()` /
# `ENV_MUTEX`) or the `EnvGuard` RAII helper.
#
# Motivation: issue #6015 and the broader coverage-flake class in #6014. The
# largest source of `Code Coverage` red on main is non-hermetic tests that
# mutate process-global env in parallel and race each other (e.g. the
# `build_runtime_input_production_*` block, 14 tests failing together). Since
# Rust 1.82 `std::env::set_var` is also UB in a multi-threaded program unless
# serialized. `crates/ironclaw_common/src/env_helpers.rs` documents the
# sanctioned pattern: acquire `lock_env()` and mutate through an `EnvGuard`.
#
# Delta-scoped and function-scoped: it inspects `git diff -W` (whole-function
# context) so it only fires on NEW env mutation whose enclosing function has no
# lock guard. This keeps it quiet on the ~700 pre-existing (guarded) call sites
# and only gates newly-introduced unguarded ones.
#
# Suppress a genuine single-threaded case (e.g. a startup shim that runs before
# any thread spawns) with an inline `// env-hermetic: <reason>` comment on the
# mutating line.

cd "$(git rev-parse --show-toplevel)"

# Guard tokens whose presence in the same function means the mutation is
# serialized (or wrapped in the restore-on-drop helper).
GUARD_RE='lock_env|lock_runtime_env|ENV_MUTEX|EnvGuard'
# Raw process-env mutation (not the thread-safe `set_runtime_env` map API).
# Literal `(` written as `[(]` so it survives awk's ERE (where `\(` is invalid).
MUT_RE='env::(set_var|remove_var)[[:space:]]*[(]'
SUPPRESS_RE='// env-hermetic:'

# Diff source: staged changes if any, else the working tree vs the upstream base.
if git diff --cached --quiet 2>/dev/null; then
    BASE_REF=""
    for ref in "@{upstream}" "origin/HEAD" "origin/main" "origin/master" "main" "master"; do
        if git rev-parse --verify --quiet "$ref" >/dev/null 2>&1; then
            BASE_REF="$ref"
            break
        fi
    done
    if [ -z "$BASE_REF" ]; then
        # No base ref (fresh repo / detached) — nothing to diff against.
        echo "hermetic-env: no base ref to diff against; skipping"
        exit 0
    fi
    DIFF=$(git diff -W "$BASE_REF" -- '*.rs' 2>/dev/null || true)
else
    DIFF=$(git diff --cached -W -- '*.rs' 2>/dev/null || true)
fi

[ -n "$DIFF" ] || { echo "hermetic-env: no Rust changes; OK"; exit 0; }

HITS=$(printf '%s\n' "$DIFF" | awk -v guard="$GUARD_RE" -v mut="$MUT_RE" -v suppress="$SUPPRESS_RE" '
    function flush() {
        if (buf != "" && !has_guard) {
            printf "%s", buf
        }
        buf = ""
        has_guard = 0
    }
    /^\+\+\+ b\// { flush(); file = substr($0, 7); next }
    /^--- / { next }
    /^@@ / { flush(); next }
    {
        # A guard token anywhere in the function hunk (added or context)
        # clears the whole hunk.
        if ($0 ~ guard) has_guard = 1
        # Only ADDED lines that mutate env and are not suppressed accumulate.
        if ($0 ~ /^\+/ && $0 ~ mut && $0 !~ suppress) {
            line = $0
            sub(/^\+/, "", line)
            buf = buf "    " file ": " line "\n"
        }
    }
    END { flush() }
')

if [ -n "$HITS" ]; then
    echo "✗ Newly-added raw std::env::set_var/remove_var without an env guard:"
    printf '%s' "$HITS"
    echo ""
    echo "Process env is global; unguarded mutation races parallel tests (see #6015)"
    echo "and is UB on Rust 1.82+. Acquire crate::env_helpers::lock_env() (or the"
    echo "module's lock_runtime_env()) and mutate through an EnvGuard, per"
    echo "crates/ironclaw_common/src/env_helpers.rs."
    echo "Genuine single-threaded case? Annotate the line with '// env-hermetic: <reason>'."
    echo "Bypass (not recommended): git push --no-verify"
    exit 1
fi

echo "hermetic-env: OK (no unguarded env mutation added)"
