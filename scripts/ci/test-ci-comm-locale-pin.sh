#!/usr/bin/env bash
# Regression tests for the comm/sort collation mismatch fixed in
# discover-reborn-package-crates.sh: inputs were sorted with LC_ALL=C but comm
# ran in the ambient locale, so under a UTF-8 collation (which orders
# "ironclaw_event_log" before "ironclaw_event_streams", unlike C) the pre-push
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

assert_success() {
    local label="$1"
    shift
    if "$@" > /dev/null 2>&1; then
        report 1 "$label"
    else
        report 0 "$label"
    fi
}

assert_failure() {
    local label="$1"
    shift
    if "$@" > /dev/null 2>&1; then
        report 0 "$label"
    else
        report 1 "$label"
    fi
}

# Case 1: every comm invocation in CI scripts and tracked hooks must pin
# LC_ALL=C so its collation matches the LC_ALL=C-sorted inputs. Backslash-
# newline continuations are joined first so multiline invocations — both the
# offending `comm \` form and a valid `LC_ALL=C \` + `comm` form — are
# inspected as whole commands; comment lines are then dropped, and compound
# commands are split at `;`, `&`, and `|` so each invocation is inspected
# independently (a pinned comm must not excuse an unpinned one on the same
# logical line). `find -L` follows the .githooks symlinks to their tracked
# targets; build caches are pruned. This file is excluded: it deliberately
# runs unpinned comm to prove the failure mode.
scan_unpinned_comm() {
    sed -e ':a' -e '/\\$/{N; s/\\\n[[:space:]]*/ /; ba' -e '}' "$1" \
        | grep -av '^[[:space:]]*#' \
        | tr ';&|' '\n\n\n' \
        | grep -anE '(^|[^-=[:alnum:]_.])comm([[:space:]]|$)' \
        | grep -avE 'LC_ALL=C[[:space:]]+comm([[:space:]]|$)' \
        || true
}

# Scanner self-checks: each invocation in a compound command is judged on its
# own, and the pinned forms (single, compound, multiline continuation) stay
# clean.
scanner_fixture="$(mktemp "${TMPDIR:-/tmp}/comm-locale.XXXXXX")"
printf 'LC_ALL=C comm -12 a b; comm -12 c d\n' > "$scanner_fixture"
assert_failure "scanner flags an unpinned comm after a pinned one in a compound command" \
    test -z "$(scan_unpinned_comm "$scanner_fixture")"
printf 'comm \\\n  -12 a b\n' > "$scanner_fixture"
assert_failure "scanner flags an unpinned multiline comm continuation" \
    test -z "$(scan_unpinned_comm "$scanner_fixture")"
printf 'LC_ALL=C comm -12 a b && LC_ALL=C \\\n  comm -12 c d\n' > "$scanner_fixture"
assert_success "scanner accepts pinned comm in compound and multiline forms" \
    test -z "$(scan_unpinned_comm "$scanner_fixture")"
rm -f "$scanner_fixture"

unpinned=""
while IFS= read -r -d '' file; do
    hits="$(scan_unpinned_comm "$file")"
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
# "ironclaw_event_streams" < "ironclaw_event_log" ('_' 0x5f < 's' 0x73). The
# sentinel second file ("zzz") forces comm to advance through file 1 and
# check its order — with identical files the lines compare equal and comm
# never notices the disorder. Select a locale by proving the mismatch:
# the sentinel must still sort after both fixture entries under it (or the
# fixture-order path is never exercised) and unpinned comm must reject the
# C-sorted fixture under it (this skips C-compatible collations such as
# C.UTF-8, where the case would prove nothing). Then assert both sides in
# that same environment: unpinned comm rejects the fixture and the pinned
# form accepts it. If no installed locale qualifies, the mismatch is not
# reproducible on this machine; say so explicitly instead of silently
# passing under C.
fixture="$(mktemp "${TMPDIR:-/tmp}/comm-locale.XXXXXX")"
sentinel="$(mktemp "${TMPDIR:-/tmp}/comm-locale.XXXXXX")"
printf 'ironclaw_event_streams\nironclaw_events\n' > "$fixture"
printf 'zzz\n' > "$sentinel"

mismatch_locale=""
while IFS= read -r candidate; do
    last="$(cat "$fixture" "$sentinel" | LC_ALL="$candidate" sort | tail -n 1)"
    if [ "$last" != "zzz" ]; then
        continue
    fi
    if ! LC_ALL="$candidate" comm -12 "$fixture" "$sentinel" > /dev/null 2>&1; then
        mismatch_locale="$candidate"
        break
    fi
done < <(locale -a 2>/dev/null | grep -aiE 'utf-?8$')

if [ -z "$mismatch_locale" ]; then
    echo "SKIP: no installed UTF-8 locale rejects the C-sorted fixture; collation mismatch not reproducible here"
else
    assert_failure "unpinned comm rejects C-sorted input under mismatching locale $mismatch_locale" \
        env LC_ALL="$mismatch_locale" comm -12 "$fixture" "$sentinel"
    assert_success "LC_ALL=C comm accepts C-sorted input under mismatching locale $mismatch_locale" \
        env LC_ALL="$mismatch_locale" sh -c 'LC_ALL=C comm -12 "$1" "$2"' _ "$fixture" "$sentinel"
fi
rm -f "$fixture" "$sentinel"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
