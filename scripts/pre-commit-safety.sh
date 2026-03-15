#!/usr/bin/env bash
# Pre-commit safety checks for common issues caught by AI code reviewers.
#
# Can be run standalone: bash scripts/pre-commit-safety.sh
# Or installed as a git pre-commit hook via dev-setup.sh.
#
# Checks staged .rs files for:
#   1. Unsafe UTF-8 byte slicing (panics on multi-byte chars)
#   2. Case-sensitive file extension comparisons
#   3. Hardcoded /tmp paths in tests (flaky in parallel runs)
#   4. Tool parameters logged without redaction (secret leaks)
#   5. Multi-step DB operations without transaction wrapping
#   6. .unwrap(), .expect(), assert!() in production code (panics)
#
# Suppress individual lines with an inline "// safety: <reason>" comment.

set -euo pipefail

# Determine a suitable base ref for standalone diffs.
resolve_base_ref() {
    local candidates=(
        "@{upstream}"
        "origin/HEAD"
        "origin/main"
        "origin/master"
        "main"
        "master"
    )

    for ref in "${candidates[@]}"; do
        if git rev-parse --verify --quiet "$ref" >/dev/null 2>&1; then
            echo "$ref"
            return 0
        fi
    done

    echo "pre-commit-safety: could not determine a base Git ref for diff (tried: ${candidates[*]})." >&2
    echo "pre-commit-safety: ensure your repository has an upstream or a local main/master branch." >&2
    exit 1
}

# Support both pre-commit hook (staged files) and standalone (all changed vs base)
if git diff --cached --quiet 2>/dev/null; then
    # No staged changes -- compare working tree against a resolved base ref
    BASE_REF="$(resolve_base_ref)"
    DIFF_OUTPUT=$(git diff "$BASE_REF" -- '*.rs' 2>/dev/null || true)
else
    DIFF_OUTPUT=$(git diff --cached -U0 -- '*.rs' 2>/dev/null || true)
fi

# Early exit if there are no relevant .rs changes
if [ -z "$DIFF_OUTPUT" ]; then
    exit 0
fi

WARNINGS=0

warn() {
    if [ "$WARNINGS" -eq 0 ]; then
        echo ""
        echo "=== Pre-commit Safety Checks ==="
        echo ""
    fi
    WARNINGS=$((WARNINGS + 1))
    echo "  [$1] $2"
}

# 1. Unsafe UTF-8 byte slicing: &s[..N] or &s[..some_var] on strings
#    Safe patterns: is_char_boundary, char_indices, // safety:
if echo "$DIFF_OUTPUT" | grep -nE '^\+' | grep -E '\[\.\..*\]' | grep -vE 'is_char_boundary|char_indices|// safety:|as_bytes|Vec<|&\[u8\]|\[u8\]|bytes\(\)|&bytes' | head -3 | grep -q .; then
    warn "UTF8" "Possible unsafe byte-index string slicing. Use is_char_boundary() or char_indices()."
    echo "$DIFF_OUTPUT" | grep -nE '^\+' | grep -E '\[\.\..*\]' | grep -vE 'is_char_boundary|char_indices|// safety:|as_bytes|Vec<|&\[u8\]|\[u8\]|bytes\(\)|&bytes' | head -3 | sed 's/^/    /'
fi

# 2. Case-sensitive file extension checks
#    Match: .ends_with(".png") without prior to_lowercase
if echo "$DIFF_OUTPUT" | grep -nE '^\+.*ends_with\("\.([pP][nN][gG]|[jJ][pP][eE]?[gG]|[gG][iI][fF]|[wW][eE][bB][pP]|[mM][dD])"\)' | grep -vE 'to_lowercase|to_ascii_lowercase|// safety:' | head -3 | grep -q .; then
    warn "CASE" "Case-sensitive file extension comparison. Normalize to lowercase first."
    echo "$DIFF_OUTPUT" | grep -nE '^\+.*ends_with\("\.([pP][nN][gG]|[jJ][pP][eE]?[gG]|[gG][iI][fF]|[wW][eE][bB][pP]|[mM][dD])"\)' | grep -vE 'to_lowercase|to_ascii_lowercase|// safety:' | head -3 | sed 's/^/    /'
fi

# 3. Hardcoded /tmp paths in test files
if echo "$DIFF_OUTPUT" | grep -nE '^\+.*"/tmp/' | grep -vE 'tempfile|tempdir|// safety:' | head -3 | grep -q .; then
    warn "TMPDIR" "Hardcoded /tmp path. Use tempfile::tempdir() for parallel-safe tests."
    echo "$DIFF_OUTPUT" | grep -nE '^\+.*"/tmp/' | grep -vE 'tempfile|tempdir|// safety:' | head -3 | sed 's/^/    /'
fi

# 4. Logging tool parameters without redaction
if echo "$DIFF_OUTPUT" | grep -nE '^\+.*tracing::(info|debug|warn|error).*param' | grep -vE 'redact|// safety:' | head -3 | grep -q .; then
    warn "REDACT" "Logging tool parameters without redaction. Use redact_params() first."
    echo "$DIFF_OUTPUT" | grep -nE '^\+.*tracing::(info|debug|warn|error).*param' | grep -vE 'redact|// safety:' | head -3 | sed 's/^/    /'
fi

# 5. Multi-step DB operations without transaction
#    Uses -W (function context) to reduce false positives from existing transactions.
#    Suppressible with "// safety:" in the hunk.
DIFF_W_OUTPUT=$(git diff --cached -W -- '*.rs' 2>/dev/null || git diff "$(resolve_base_ref)" -W -- '*.rs' 2>/dev/null || true)
if [ -n "$DIFF_W_OUTPUT" ]; then
    HUNK_COUNT=$(echo "$DIFF_W_OUTPUT" | awk '
        /^@@/ {
            if (count >= 2 && !has_tx && !has_safety) found++
            count=0; has_tx=0; has_safety=0
        }
        /^\+.*\.(execute|query)\(/ { count++ }
        /^\+.*(transaction|\.tx\.|\.begin\()/ { has_tx=1 }
        / .*(transaction|\.tx\.|\.begin\()/ { has_tx=1 }
        /\/\/ safety:/ { has_safety=1 }
        END {
            if (count >= 2 && !has_tx && !has_safety) found++
            print found+0
        }
    ')
    if [ "$HUNK_COUNT" -gt 0 ]; then
        warn "TX" "Multiple DB operations in same function without transaction. Wrap in a transaction for atomicity."
        echo "$DIFF_W_OUTPUT" | awk '
            /^@@/ {
                if (count >= 2 && !has_tx && !has_safety) { print buf }
                buf=""; count=0; has_tx=0; has_safety=0
            }
            /^\+.*\.(execute|query)\(/ { count++ }
            /^\+.*(transaction|\.tx\.|\.begin\()/ { has_tx=1 }
            / .*(transaction|\.tx\.|\.begin\()/ { has_tx=1 }
            /\/\/ safety:/ { has_safety=1 }
            { buf = buf "\n" $0 }
            END {
                if (count >= 2 && !has_tx && !has_safety) { print buf }
            }
        ' | grep -E '^\+.*\.(execute|query)\(' | head -4 | sed 's/^/    /'
    fi
fi

# 6. .unwrap(), .expect(), assert!() in production code
#    Matches added lines containing panic-inducing calls.
#    Excludes test files, test modules, and debug_assert (compiled out in release).
#    Suppress with "// safety: <reason>".
PROD_DIFF="$DIFF_OUTPUT"
# Strip hunks from test-only files (tests/ directory, *_test.rs, test_*.rs)
PROD_DIFF=$(echo "$PROD_DIFF" | grep -v '^+++ b/tests/' || true)
# Strip hunks whose @@ context line indicates a test module.
# git diff includes the enclosing function/module name after @@.
# Only match `mod tests` (the conventional #[cfg(test)] module) — do NOT
# match `fn test_*` because production code can have functions named test_*.
PROD_DIFF=$(echo "$PROD_DIFF" | awk '
    /^@@ / { in_test = ($0 ~ /mod tests/) }
    !in_test { print }
' || true)
if echo "$PROD_DIFF" | grep -nE '^\+' \
    | grep -E '\.(unwrap|expect)\(|[^_]assert(_eq|_ne)?!' \
    | grep -vE 'debug_assert|// safety:|#\[cfg\(test\)\]|#\[test\]|mod tests' \
    | head -5 | grep -q .; then
    warn "PANIC" "Production code must not use .unwrap(), .expect(), or assert!(). Use proper error handling."
    echo "$PROD_DIFF" | grep -nE '^\+' \
        | grep -E '\.(unwrap|expect)\(|[^_]assert(_eq|_ne)?!' \
        | grep -vE 'debug_assert|// safety:|#\[cfg\(test\)\]|#\[test\]|mod tests' \
        | head -5 | sed 's/^/    /'
fi

if [ "$WARNINGS" -gt 0 ]; then
    echo ""
    echo "Found $WARNINGS potential issue(s). Fix them or add '// safety: <reason>' to suppress."
    echo ""
    exit 1
fi
