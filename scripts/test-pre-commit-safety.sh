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

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
