#!/usr/bin/env bash
# Regression tests for the comm/sort collation mismatch fixed in
# discover-reborn-package-crates.sh: inputs were sorted with LC_ALL=C but comm
# ran in the ambient locale, so under a UTF-8 collation (which orders
# "ironclaw_events" before "ironclaw_event_streams", unlike C) the pre-push
# coverage ratchet died with "comm: input is not in sorted order".

set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

PASS=0
FAIL=0

report() {
    local ok="$1" label="$2" detail="${3:-}"
    if [ "$ok" -eq 1 ]; then
        echo "PASS: $label"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $label${detail:+ — $detail}"
        FAIL=$((FAIL + 1))
    fi
}

# Case 1: every comm invocation in CI scripts and tracked hooks must pin
# LC_ALL=C so its collation matches the LC_ALL=C-sorted inputs.
unpinned="$(grep -rnE '(^|[^=[:alnum:]_])comm[[:space:]]+-' \
    "$ROOT_DIR/scripts/ci" "$ROOT_DIR/.githooks" \
    | grep -v 'LC_ALL=C comm' \
    | grep -v 'test-ci-comm-locale-pin' \
    || true)"
if [ -z "$unpinned" ]; then
    report 1 "all comm invocations are LC_ALL=C-pinned"
else
    report 0 "all comm invocations are LC_ALL=C-pinned" "$unpinned"
fi

# Case 2: the fixture pair that broke the ratchet. In C collation
# "ironclaw_event_streams" < "ironclaw_events" ('_' 0x5f < 's' 0x73); UTF-8
# collations disagree, so an unpinned comm rejects the C-sorted file. The
# pinned form must accept it regardless of the ambient locale.
utf8_locale=""
for candidate in C.UTF-8 C.utf8 en_US.UTF-8 en_US.utf8; do
    if locale -a 2>/dev/null | grep -qx "$candidate"; then
        utf8_locale="$candidate"
        break
    fi
done

fixture="$(mktemp "${TMPDIR:-/tmp}/comm-locale.XXXXXX")"
printf 'ironclaw_event_streams\nironclaw_events\n' > "$fixture"

if LC_ALL="${utf8_locale:-C}" sh -c 'LC_ALL=C comm -12 "$1" "$1"' _ "$fixture" > /dev/null 2>&1; then
    report 1 "LC_ALL=C comm accepts C-sorted input under ${utf8_locale:-C} locale"
else
    report 0 "LC_ALL=C comm accepts C-sorted input under ${utf8_locale:-C} locale"
fi
rm -f "$fixture"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
