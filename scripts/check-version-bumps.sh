#!/usr/bin/env bash
set -euo pipefail

# CI script: check that version bumps accompany WIT or extension source changes.
# Exit 0 if all checks pass, exit 1 if any version wasn't bumped.

ERRORS=0

# --- Skip mechanism -----------------------------------------------------------

if [[ "${PR_LABELS:-}" == *"skip-version-check"* ]]; then
    echo "skip-version-check label detected — skipping all version checks."
    exit 0
fi

# Check commit messages for [skip-version-check]
if git log "origin/${GITHUB_BASE_REF:-main}...HEAD" --pretty=format:"%s %b" 2>/dev/null \
    | grep -qF '[skip-version-check]'; then
    echo "[skip-version-check] found in commit message — skipping all version checks."
    exit 0
fi

# --- Determine base branch and changed files ----------------------------------

BASE_BRANCH="${GITHUB_BASE_REF:-main}"
echo "Base branch: $BASE_BRANCH"

CHANGED_FILES=$(git diff --name-only "origin/${BASE_BRANCH}...HEAD" 2>/dev/null || true)

if [[ -z "$CHANGED_FILES" ]]; then
    echo "No changed files detected. Nothing to check."
    exit 0
fi

# --- Helper functions ---------------------------------------------------------

# Extract the version from a WIT package line like: package near:agent@1.2.3;
extract_wit_version() {
    local file="$1"
    if [[ ! -f "$file" ]]; then
        echo ""
        return
    fi
    sed -n 's/^[[:space:]]*package[[:space:]]\+[^@]*@\([0-9][0-9.]*[0-9]\)[[:space:]]*;.*/\1/p' "$file" \
        | head -n1
}

# Extract version from the base branch copy of a file
extract_wit_version_base() {
    local file="$1"
    git show "origin/${BASE_BRANCH}:${file}" 2>/dev/null \
        | sed -n 's/^[[:space:]]*package[[:space:]]\+[^@]*@\([0-9][0-9.]*[0-9]\)[[:space:]]*;.*/\1/p' \
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

if echo "$CHANGED_FILES" | grep -qx 'wit/tool\.wit'; then
    WIT_TOOL_CHANGED=true
fi
if echo "$CHANGED_FILES" | grep -qx 'wit/channel\.wit'; then
    WIT_CHANNEL_CHANGED=true
fi

if $WIT_TOOL_CHANGED; then
    echo ""
    echo "=== wit/tool.wit changed ==="

    NEW_VER=$(extract_wit_version "wit/tool.wit")
    OLD_VER=$(extract_wit_version_base "wit/tool.wit")
    echo "  WIT package version: ${OLD_VER:-<none>} -> ${NEW_VER:-<missing>}"

    if ! version_was_bumped "${NEW_VER}" "${OLD_VER}"; then
        echo "  ERROR: wit/tool.wit package version was not bumped (${OLD_VER} -> ${NEW_VER:-<missing>})."
        ERRORS=$((ERRORS + 1))
    else
        echo "  OK: WIT package version bumped."
    fi

    # Check WIT_TOOL_VERSION constant matches
    CONST_VER=$(extract_rust_const "src/tools/wasm/mod.rs" "WIT_TOOL_VERSION")
    if [[ -n "$NEW_VER" && "$CONST_VER" != "$NEW_VER" ]]; then
        echo "  ERROR: WIT_TOOL_VERSION in src/tools/wasm/mod.rs is '${CONST_VER}' but wit/tool.wit has '${NEW_VER}'. They must match."
        ERRORS=$((ERRORS + 1))
    elif [[ -n "$NEW_VER" ]]; then
        echo "  OK: WIT_TOOL_VERSION matches wit/tool.wit."
    fi
fi

if $WIT_CHANNEL_CHANGED; then
    echo ""
    echo "=== wit/channel.wit changed ==="

    NEW_VER=$(extract_wit_version "wit/channel.wit")
    OLD_VER=$(extract_wit_version_base "wit/channel.wit")
    echo "  WIT package version: ${OLD_VER:-<none>} -> ${NEW_VER:-<missing>}"

    if ! version_was_bumped "${NEW_VER}" "${OLD_VER}"; then
        echo "  ERROR: wit/channel.wit package version was not bumped (${OLD_VER} -> ${NEW_VER:-<missing>})."
        ERRORS=$((ERRORS + 1))
    else
        echo "  OK: WIT package version bumped."
    fi

    # Check WIT_CHANNEL_VERSION constant matches
    CONST_VER=$(extract_rust_const "src/tools/wasm/mod.rs" "WIT_CHANNEL_VERSION")
    if [[ -n "$NEW_VER" && "$CONST_VER" != "$NEW_VER" ]]; then
        echo "  ERROR: WIT_CHANNEL_VERSION in src/tools/wasm/mod.rs is '${CONST_VER}' but wit/channel.wit has '${NEW_VER}'. They must match."
        ERRORS=$((ERRORS + 1))
    elif [[ -n "$NEW_VER" ]]; then
        echo "  OK: WIT_CHANNEL_VERSION matches wit/channel.wit."
    fi
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
