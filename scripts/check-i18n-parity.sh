#!/usr/bin/env bash
#
# Validate that all i18n language packs under crates/ironclaw_gateway/static/i18n/
# share the same key set with no duplicates and matching placeholder tokens.
#
# When a new translation key is added to en.js, it must also be added to
# every other language file in lockstep — otherwise users of that language
# will see the raw key string at runtime.
#
# Usage:
#   ./scripts/check-i18n-parity.sh           # check all language files
#   Run by pre-commit hook automatically when i18n files are staged.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
I18N_DIR="$REPO_ROOT/crates/ironclaw_gateway/static/i18n"
BASE_LANG="en.js"
OTHER_LANGS=("zh-CN.js" "ko.js")
EXIT=0

if [ ! -d "$I18N_DIR" ]; then
    echo "i18n parity: directory not found: $I18N_DIR" >&2
    exit 1
fi

extract_keys() {
    grep -oE "^  '[^']+':" "$1" | sed -E "s/^  '([^']+)':/\1/"
}

# Extract "key=placeholders" pairs (placeholders sorted, comma-joined).
# Used to verify that {name}-style tokens are consistent across languages.
# Pure-POSIX: works with macOS awk (no gawk extensions).
extract_key_placeholders() {
    # Step 1: pull key + value into "key|value" lines
    sed -nE "s/^  '([^']+)': '(.*)',?[[:space:]]*\$/\1|\2/p" "$1" |
    # Step 2: for each line, extract sorted unique placeholders, emit "key=ph1,ph2,..."
    while IFS='|' read -r key val; do
        ph=$(printf '%s\n' "$val" | grep -oE '\{[a-zA-Z_][a-zA-Z0-9_]*\}' 2>/dev/null | tr -d '{}' | sort -u | paste -sd, - || true)
        printf '%s=%s\n' "$key" "$ph"
    done
}

fail() {
    echo "ERROR: $*" >&2
    EXIT=1
}

# --- Check 1: No duplicate keys within any single file ---
for f in "$BASE_LANG" "${OTHER_LANGS[@]}"; do
    path="$I18N_DIR/$f"
    if [ ! -f "$path" ]; then
        fail "missing language file: $path"
        continue
    fi
    dups=$(extract_keys "$path" | sort | uniq -d || true)
    if [ -n "$dups" ]; then
        fail "duplicate keys in $f:"
        printf '  %s\n' $dups >&2
    fi
done

# --- Check 2: Every language has identical key set to en.js ---
en_path="$I18N_DIR/$BASE_LANG"
# Portable mktemp helper: BSD/macOS `mktemp` requires an explicit template
# with at least 6 trailing X's, while GNU `mktemp` accepts a bare invocation.
# Always pass a template so the script works on every platform.
mktemp_file() {
    mktemp "${TMPDIR:-/tmp}/check-i18n-parity.XXXXXX"
}

if [ -f "$en_path" ]; then
    en_keys_file=$(mktemp_file)
    extract_keys "$en_path" | sort -u > "$en_keys_file"

    for f in "${OTHER_LANGS[@]}"; do
        path="$I18N_DIR/$f"
        [ -f "$path" ] || continue

        lang_keys_file=$(mktemp_file)
        extract_keys "$path" | sort -u > "$lang_keys_file"

        missing=$(comm -23 "$en_keys_file" "$lang_keys_file")
        extra=$(comm -13 "$en_keys_file" "$lang_keys_file")

        if [ -n "$missing" ]; then
            fail "$f is missing keys present in $BASE_LANG:"
            printf '  %s\n' $missing >&2
        fi
        if [ -n "$extra" ]; then
            fail "$f has keys not present in $BASE_LANG:"
            printf '  %s\n' $extra >&2
        fi

        rm -f "$lang_keys_file"
    done
    rm -f "$en_keys_file"
fi

# --- Check 3: Placeholder tokens ({name}, {count}, ...) match across files ---
if [ -f "$en_path" ]; then
    en_ph_file=$(mktemp_file)
    extract_key_placeholders "$en_path" | sort > "$en_ph_file"

    for f in "${OTHER_LANGS[@]}"; do
        path="$I18N_DIR/$f"
        [ -f "$path" ] || continue

        lang_ph_file=$(mktemp_file)
        extract_key_placeholders "$path" | sort > "$lang_ph_file"

        # Use mktemp instead of /tmp/<name>.$$ to avoid symlink races and
        # collisions: the predictable PID-suffix path is vulnerable to
        # symlink attacks in shared /tmp.
        mismatch_file=$(mktemp_file)
        join -t '=' -j 1 \
            <(awk -F= '{print $1 "=" $2}' "$en_ph_file") \
            <(awk -F= '{print $1 "=" $2}' "$lang_ph_file") \
            | awk -F= '$2 != $3 {print "  " $1 ": en=[" $2 "] '"$f"'=[" $3 "]"}' \
            > "$mismatch_file"

        if [ -s "$mismatch_file" ]; then
            fail "$f has placeholder mismatches with $BASE_LANG:"
            cat "$mismatch_file" >&2
        fi
        rm -f "$lang_ph_file" "$mismatch_file"
    done
    rm -f "$en_ph_file"
fi

if [ $EXIT -eq 0 ]; then
    count=$(extract_keys "$en_path" | wc -l | tr -d ' ')
    langs=$((${#OTHER_LANGS[@]} + 1))
    echo "i18n parity: OK ($count keys × $langs languages)"
fi

exit $EXIT
