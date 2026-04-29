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

# ── REBORN_BRIDGE ─────────────────────────────────────────────
# Issue #3026 acceptance criterion #13: no file may simultaneously
# touch the Reborn composition surface (`reborn_services` /
# `RebornProductionServices`) and reach into legacy state-bag fields
# (`state.{store,workspace,...}.*`) on the same edit. The check runs
# an awk pipeline over the unified diff to group added lines by file,
# then flags files that have both.

assert_reborn_bridge() {
    local label="$1" diff="$2" expect_flag="$3"
    local result
    if result=$(printf '%s\n' "$diff" | awk '
        /^\+\+\+ b\// { file = substr($0, 7); next }
        /^\+/ {
            line = substr($0, 2)
            if (line ~ /reborn_services|RebornProductionServices/) reborn[file] = reborn[file] "\n" line
            if (line ~ /state\.(store|workspace|workspace_pool|extension_manager|skill_registry|session_manager)\./ \
                && line !~ /\/\/ dispatch-exempt:|\/\/ reborn-bridge-exempt:|\/\/ safety:/) \
                legacy[file] = legacy[file] "\n" line
        }
        END {
            for (f in reborn) {
                if (legacy[f] != "") {
                    print f
                }
            }
        }
    '); then :; fi

    if [ "$expect_flag" = "yes" ]; then
        if [ -n "${result:-}" ]; then
            echo "OK: $label (correctly flagged)"
            PASS=$((PASS + 1))
        else
            echo "FAIL: $label — bridge violation not flagged"
            FAIL=$((FAIL + 1))
        fi
    else
        if [ -z "${result:-}" ]; then
            echo "OK: $label (correctly filtered)"
            PASS=$((PASS + 1))
        else
            echo "FAIL: $label — false positive: $result"
            FAIL=$((FAIL + 1))
        fi
    fi
}

# 1. Reborn-only file: not flagged.
assert_reborn_bridge "REBORN_BRIDGE: reborn-only file is not flagged" \
    "+++ b/src/app.rs
+    let services = build_reborn_production_services(input).await?;
+    components.reborn_services = Some(services);" \
    "no"

# 2. Legacy-only file: not flagged.
assert_reborn_bridge "REBORN_BRIDGE: legacy-only file is not flagged" \
    "+++ b/src/channels/web/handlers/projects.rs
+    let projects = state.store.list_projects(&owner_id).await?;" \
    "no"

# 3. Same file touches both: flagged.
assert_reborn_bridge "REBORN_BRIDGE: same-file dual touch is flagged" \
    "+++ b/src/channels/web/handlers/dual.rs
+    let r = components.reborn_services.readiness();
+    let projects = state.store.list_projects(&owner_id).await?;" \
    "yes"

# 4. Same file but legacy line is annotated as a documented bridge:
#    not flagged.
assert_reborn_bridge "REBORN_BRIDGE: documented bridge exemption suppresses" \
    "+++ b/src/bridge/legacy_compat.rs
+    let r = components.reborn_services.readiness();
+    let projects = state.store.list_projects(&owner_id).await?; // reborn-bridge-exempt: legacy v1 settings read during cutover #3029" \
    "no"

# 5. Cross-file: Reborn touch in one file, legacy touch in another —
#    not flagged. Bug only shows when same code path mixes both.
assert_reborn_bridge "REBORN_BRIDGE: cross-file touches are not flagged" \
    "+++ b/src/app.rs
+    components.reborn_services = Some(services);
+++ b/src/channels/web/handlers/projects.rs
+    let projects = state.store.list_projects(&owner_id).await?;" \
    "no"

# 6. Same file but legacy line uses the existing dispatch-exempt
#    annotation: not flagged (we share the exemption set).
assert_reborn_bridge "REBORN_BRIDGE: existing dispatch-exempt also suppresses" \
    "+++ b/src/cli/admin.rs
+    let r = components.reborn_services.readiness();
+    let users = state.store.list_users(scope).await?; // dispatch-exempt: cross-user aggregation read" \
    "no"

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
