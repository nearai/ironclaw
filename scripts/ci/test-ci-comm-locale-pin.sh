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
# LC_ALL=C so its collation matches the LC_ALL=C-sorted inputs. Backslash-
# newline continuations are joined first so multiline invocations — both the
# offending `comm \` form and a valid `LC_ALL=C \` + `comm` form — are
# inspected as whole commands; comment lines are then dropped. `find -L`
# follows the .githooks symlinks to their tracked targets; build caches are
# pruned. This file is excluded: it deliberately runs unpinned comm to prove
# the failure mode.
unpinned=""
while IFS= read -r -d '' file; do
    hits="$(sed -e ':a' -e '/\\$/{N; s/\\\n[[:space:]]*/ /; ba' -e '}' "$file" \
        | grep -av '^[[:space:]]*#' \
        | grep -anE '(^|[^-=[:alnum:]_.])comm([[:space:]]|$)' \
        | grep -avE 'LC_ALL=C[[:space:]]+comm([[:space:]]|$)' \
        || true)"
    if [ -n "$hits" ]; then
        unpinned="${unpinned}${file}: ${hits}"$'\n'
    fi
done < <(find -L "$ROOT_DIR/scripts/ci" "$ROOT_DIR/.githooks" \
    -name __pycache__ -prune -o -type f \
    ! -name "$(basename "${BASH_SOURCE[0]}")" -print0)
if [ -z "$unpinned" ]; then
    report 1 "all comm invocations are LC_ALL=C-pinned"
else
    report 0 "all comm invocations are LC_ALL=C-pinned" "$unpinned"
fi

# Case 2: the fixture pair that broke the ratchet. In C collation
# "ironclaw_event_streams" < "ironclaw_events" ('_' 0x5f < 's' 0x73). The
# sentinel second file ("zzz" sorts last in any collation) forces comm to
# advance through file 1 and check its order — with identical files the lines
# compare equal and comm never notices the disorder. Select a locale by
# proving the mismatch — unpinned comm must reject the C-sorted fixture under
# it (this skips C-compatible collations such as C.UTF-8, where the case would
# prove nothing) — then assert the pinned form succeeds in that same
# environment. If no installed locale disagrees with C on the fixture, the
# mismatch is not reproducible on this machine; say so explicitly instead of
# silently passing under C.
fixture="$(mktemp "${TMPDIR:-/tmp}/comm-locale.XXXXXX")"
sentinel="$(mktemp "${TMPDIR:-/tmp}/comm-locale.XXXXXX")"
printf 'ironclaw_event_streams\nironclaw_events\n' > "$fixture"
printf 'zzz\n' > "$sentinel"

mismatch_locale=""
while IFS= read -r candidate; do
    if ! LC_ALL="$candidate" comm -12 "$fixture" "$sentinel" > /dev/null 2>&1; then
        mismatch_locale="$candidate"
        break
    fi
done < <(locale -a 2>/dev/null | grep -aiE 'utf-?8$')

if [ -z "$mismatch_locale" ]; then
    echo "SKIP: no installed UTF-8 locale rejects the C-sorted fixture; collation mismatch not reproducible here"
elif LC_ALL="$mismatch_locale" sh -c 'LC_ALL=C comm -12 "$1" "$2"' _ "$fixture" "$sentinel" > /dev/null 2>&1; then
    report 1 "LC_ALL=C comm accepts C-sorted input under mismatching locale $mismatch_locale"
else
    report 0 "LC_ALL=C comm accepts C-sorted input under mismatching locale $mismatch_locale"
fi
rm -f "$fixture" "$sentinel"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
