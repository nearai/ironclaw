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
cd "$(dirname "$0")/.."

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

# ── PROJECTION ────────────────────────────────────────────────
# Positive: direct sse.broadcast / .broadcast_for_user calls.
# Exclusions: `// projection-exempt: <category>, <detail>`, `// safety:`,
#             and diff-header lines (`+++ b/path`) via `:\+\+\+ `.
PROJ_POS='(\bsse\.(broadcast|broadcast_for_user)|^[^:]*:\+[[:space:]]*\.broadcast_for_user)\('
PROJ_NEG='// projection-exempt: [^,]+,|// safety:|:\+\+\+ '

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

# A rustfmt-wrapped call is flagged.
assert_flagged "PROJECTION: rustfmt-wrapped .broadcast_for_user is flagged" \
    "+    .broadcast_for_user(&user, event);" \
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
CREDNAME_POS='\bCredentialName\b'
CREDNAME_NEG='// web-identity-exempt:|// safety:|:\+\+\+ '

assert_filtered "CREDNAME: diff header line is filtered" \
    "+++ b/src/channels/web/features/settings.rs" \
    "$CREDNAME_POS" \
    "$CREDNAME_NEG"

assert_flagged "CREDNAME: bare CredentialName reference is flagged" \
    "+    let name: CredentialName = ...;" \
    "$CREDNAME_POS" \
    "$CREDNAME_NEG"

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
