#!/usr/bin/env bash
set -euo pipefail

# CI script: check that version bumps accompany WIT or extension source changes.
# Exit 0 if all checks pass, exit 1 if any version wasn't bumped.

ERRORS=0
ALLOW_SKIP_VERSION_CHECK="${ALLOW_SKIP_VERSION_CHECK:-true}"
RAW_BASE_BRANCH="${GITHUB_BASE_REF:-main}"
BASE_BRANCH="${RAW_BASE_BRANCH#refs/heads/}"

# Ensure the base branch ref is available for skip checks and diffs.
if ! git rev-parse "origin/${BASE_BRANCH}" >/dev/null 2>&1; then
    echo "Fetching origin/${BASE_BRANCH}..."
    git fetch origin "$BASE_BRANCH" --depth=1
fi

MERGE_BASE=$(git merge-base "origin/${BASE_BRANCH}" HEAD)

# --- Skip mechanism -----------------------------------------------------------

if [[ "${ALLOW_SKIP_VERSION_CHECK}" == "true" ]]; then
    if [[ "${PR_LABELS:-}" == *"skip-version-check"* ]]; then
        echo "skip-version-check label detected — skipping all version checks."
        exit 0
    fi

    # Check commit messages for [skip-version-check]
    if git log "${MERGE_BASE}..HEAD" --pretty=format:"%s %b" 2>/dev/null \
        | grep -qF '[skip-version-check]'; then
        echo "[skip-version-check] found in commit message — skipping all version checks."
        exit 0
    fi
fi

# --- Determine base branch and changed files ----------------------------------

echo "Base branch: $BASE_BRANCH"

CHANGED_FILES=$(git diff --name-only "$MERGE_BASE" HEAD)

if [[ -z "$CHANGED_FILES" ]]; then
    echo "No changed files detected. Nothing to check."
    exit 0
fi

# --- Helper functions ---------------------------------------------------------

# Extract the version from a WIT package line like: package near:agent@1.2.3;
#
# `[[:space:]][[:space:]]*` rather than `[[:space:]]\+`: `\+` is a GNU BRE
# extension that BSD sed (the macOS default) does not implement, where it
# matched nothing and returned an empty version. That degraded silently and in
# the fail-open direction — the WIT_TOOL_VERSION cross-check below is guarded
# on a non-empty version, so this hook printed "All version checks passed"
# having compared nothing (#7085). The two forms are identical under GNU sed,
# so the enforced Linux CI lane is unchanged.
extract_wit_version() {
    local file="$1"
    if [[ ! -f "$file" ]]; then
        echo ""
        return
    fi
    sed -n 's/^[[:space:]]*package[[:space:]][[:space:]]*[^@]*@\([0-9][0-9.]*[0-9]\)[[:space:]]*;.*/\1/p' "$file" \
        | head -n1
}

# Extract version from the base branch copy of a file
extract_wit_version_base() {
    local file="$1"
    git show "origin/${BASE_BRANCH}:${file}" 2>/dev/null \
        | sed -n 's/^[[:space:]]*package[[:space:]][[:space:]]*[^@]*@\([0-9][0-9.]*[0-9]\)[[:space:]]*;.*/\1/p' \
        | head -n1 || true
}

# Extract a Rust string constant value: pub const NAME: &str = "value";
extract_rust_const() {
    local file="$1"
    local const_name="$2"
    if [[ ! -f "$file" ]]; then
        echo ""
        return
    fi
    sed -n "s/^.*${const_name}[[:space:]]*:[[:space:]]*&str[[:space:]]*=[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$file" \
        | head -n1
}

# Extract JSON "version" field using jq
extract_json_version() {
    local file="$1"
    if [[ ! -f "$file" ]]; then
        echo ""
        return
    fi
    jq -r '.version // empty' "$file" 2>/dev/null || true
}

# Extract JSON "version" from the base branch copy of a file
extract_json_version_base() {
    local file="$1"
    git show "origin/${BASE_BRANCH}:${file}" 2>/dev/null | jq -r '.version // empty' 2>/dev/null || true
}

# Return 0 if $1 (new) is strictly greater than $2 (old) via sort -V, or old is empty.
version_was_bumped() {
    local new="$1"
    local old="$2"
    if [[ -z "$old" ]]; then
        # No prior version — treat as new, no bump required
        return 0
    fi
    if [[ -z "$new" ]]; then
        # Version was removed — that's a problem
        return 1
    fi
    if [[ "$new" == "$old" ]]; then
        return 1
    fi
    # Check new > old via sort -V
    local highest
    highest=$(printf '%s\n%s\n' "$new" "$old" | sort -V | tail -n1)
    [[ "$highest" == "$new" ]]
}

# --- 1. WIT changes ----------------------------------------------------------

WIT_TOOL_CHANGED=false
WIT_CHANNEL_CHANGED=false

# `wit/` lives inside the `ironclaw_wasm` crate, and WS7 moved that crate into
# its family directory (`crates/lanes/ironclaw_wasm/`, PROPOSAL §5). The
# trigger paths below are therefore resolved by crate NAME through the shared
# inventory (scripts/ci/lib/crate_tree.py via scripts/ci/crate-dir.sh) rather
# than written as literals: a literal that the crate moved out from under
# matches nothing, and this gate would then pass *vacuously* on every WIT
# change — the WS10 silent-dark failure mode
# (docs/reborn/target-architecture/CHECKLIST.md WS10, #6963). Resolution
# failure exits non-zero so "the crate moved" is an actionable repoint rather
# than a quietly disabled gate.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WASM_CRATE_DIR=$("${SCRIPT_DIR}/ci/crate-dir.sh" ironclaw_wasm) || exit 1
WIT_TOOL_FILE="${WASM_CRATE_DIR}/wit/tool.wit"
WIT_CHANNEL_FILE="${WASM_CRATE_DIR}/wit/channel.wit"

if echo "$CHANGED_FILES" | grep -qxF "${WIT_TOOL_FILE}"; then
    WIT_TOOL_CHANGED=true
fi
if echo "$CHANGED_FILES" | grep -qxF "${WIT_CHANNEL_FILE}"; then
    WIT_CHANNEL_CHANGED=true
fi

if $WIT_TOOL_CHANGED; then
    echo ""
    echo "=== ${WIT_TOOL_FILE} changed ==="

    NEW_VER=$(extract_wit_version "${WIT_TOOL_FILE}")
    OLD_VER=$(extract_wit_version_base "${WIT_TOOL_FILE}")
    echo "  WIT package version: ${OLD_VER:-<none>} -> ${NEW_VER:-<missing>}"

    if ! version_was_bumped "${NEW_VER}" "${OLD_VER}"; then
        echo "  ERROR: ${WIT_TOOL_FILE} package version was not bumped (${OLD_VER} -> ${NEW_VER:-<missing>})."
        ERRORS=$((ERRORS + 1))
    else
        echo "  OK: WIT package version bumped."
    fi

    # Check WIT_TOOL_VERSION constant matches (the Reborn host lives in the
    # ironclaw_wasm crate's src/config.rs; the v1 src/tools/wasm/mod.rs was
    # deleted under Tier B).
    WASM_CONFIG_FILE="${WASM_CRATE_DIR}/src/config.rs"
    CONST_VER=$(extract_rust_const "$WASM_CONFIG_FILE" "WIT_TOOL_VERSION")
    if [[ -z "$CONST_VER" ]]; then
        echo "  ERROR: could not read WIT_TOOL_VERSION from ${WASM_CONFIG_FILE} (file missing or constant not found). If the ironclaw_wasm crate moved or was renamed, repoint check-version-bumps.sh in the same change."
        ERRORS=$((ERRORS + 1))
    elif [[ -n "$NEW_VER" && "$CONST_VER" != "$NEW_VER" ]]; then
        echo "  ERROR: WIT_TOOL_VERSION in ${WASM_CONFIG_FILE} is '${CONST_VER}' but ${WIT_TOOL_FILE} has '${NEW_VER}'. They must match."
        ERRORS=$((ERRORS + 1))
    elif [[ -n "$NEW_VER" ]]; then
        echo "  OK: WIT_TOOL_VERSION matches ${WIT_TOOL_FILE}."
    fi
fi

if $WIT_CHANNEL_CHANGED; then
    echo ""
    echo "=== ${WIT_CHANNEL_FILE} changed ==="

    NEW_VER=$(extract_wit_version "${WIT_CHANNEL_FILE}")
    OLD_VER=$(extract_wit_version_base "${WIT_CHANNEL_FILE}")
    echo "  WIT package version: ${OLD_VER:-<none>} -> ${NEW_VER:-<missing>}"

    if ! version_was_bumped "${NEW_VER}" "${OLD_VER}"; then
        echo "  ERROR: ${WIT_CHANNEL_FILE} package version was not bumped (${OLD_VER} -> ${NEW_VER:-<missing>})."
        ERRORS=$((ERRORS + 1))
    else
        echo "  OK: WIT package version bumped."
    fi

    # No host-side WIT_CHANNEL_VERSION constant to cross-check: the v1
    # src/tools/wasm/mod.rs (which defined it) was deleted under Tier B and the
    # Reborn WASM host does not pin a channel WIT version constant. The WIT
    # package version bump above remains the enforced contract.
fi

if $WIT_TOOL_CHANGED || $WIT_CHANNEL_CHANGED; then
    echo ""
    echo "  WARNING: WIT interface changed. All published registry extensions should bump their versions for compatibility."
fi

# --- 2. Tool source changes ---------------------------------------------------

TOOL_NAMES=$(echo "$CHANGED_FILES" | sed -n 's|^tools-src/\([^/]*\)/.*|\1|p' | sort -u)

if [[ -n "$TOOL_NAMES" ]]; then
    echo ""
    echo "=== Tool source changes ==="
fi

for tool in $TOOL_NAMES; do
    REGISTRY_FILE="registry/tools/${tool}.json"
    echo ""
    echo "  --- tools-src/${tool}/ changed ---"

    if [[ ! -d "tools-src/${tool}" ]]; then
        echo "  SKIP: tools-src/${tool}/ no longer exists (retired local source)."
        continue
    fi

    if [[ ! -f "$REGISTRY_FILE" ]]; then
        echo "  SKIP: ${REGISTRY_FILE} does not exist yet (new extension?)."
        continue
    fi

    NEW_VER=$(extract_json_version "$REGISTRY_FILE")
    OLD_VER=$(extract_json_version_base "$REGISTRY_FILE")

    echo "  Registry version: ${OLD_VER:-<none>} -> ${NEW_VER:-<missing>}"

    if ! version_was_bumped "${NEW_VER}" "${OLD_VER}"; then
        echo "  ERROR: ${REGISTRY_FILE} version was not bumped (${OLD_VER} -> ${NEW_VER:-<missing>}). Bump the version when changing tools-src/${tool}/."
        ERRORS=$((ERRORS + 1))
    else
        echo "  OK: version bumped."
    fi
done

# --- 3. Channel source changes ------------------------------------------------

CHANNEL_NAMES=$(echo "$CHANGED_FILES" | sed -n 's|^channels-src/\([^/]*\)/.*|\1|p' | sort -u)

if [[ -n "$CHANNEL_NAMES" ]]; then
    echo ""
    echo "=== Channel source changes ==="
fi

for channel in $CHANNEL_NAMES; do
    REGISTRY_FILE="registry/channels/${channel}.json"
    echo ""
    echo "  --- channels-src/${channel}/ changed ---"

    if [[ ! -d "channels-src/${channel}" ]]; then
        echo "  SKIP: channels-src/${channel}/ no longer exists (retired local source)."
        continue
    fi

    if [[ ! -f "$REGISTRY_FILE" ]]; then
        echo "  SKIP: ${REGISTRY_FILE} does not exist yet (new extension?)."
        continue
    fi

    NEW_VER=$(extract_json_version "$REGISTRY_FILE")
    OLD_VER=$(extract_json_version_base "$REGISTRY_FILE")

    echo "  Registry version: ${OLD_VER:-<none>} -> ${NEW_VER:-<missing>}"

    if ! version_was_bumped "${NEW_VER}" "${OLD_VER}"; then
        echo "  ERROR: ${REGISTRY_FILE} version was not bumped (${OLD_VER} -> ${NEW_VER:-<missing>}). Bump the version when changing channels-src/${channel}/."
        ERRORS=$((ERRORS + 1))
    else
        echo "  OK: version bumped."
    fi
done

# --- Summary ------------------------------------------------------------------

echo ""
if [[ $ERRORS -gt 0 ]]; then
    echo "FAILED: ${ERRORS} version check(s) did not pass. See errors above."
    exit 1
else
    echo "All version checks passed."
    exit 0
fi
