#!/usr/bin/env bash
# Self-tests for the mutation-audit harness.
#
# Guardrails are code: a checker that silently does nothing is worse than no
# checker, because it reads as a clean bill of health. These cases pin the
# failure modes that actually bit during development — each one produced a
# wrong, confident answer before it was fixed.
#
# Fast and hermetic: no cargo, no compilation. The expensive end-to-end
# behaviour (a mutant flipping MISSED -> caught) is proven by running
# scripts/mutation-verify-fix.sh against a real crate, not here.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
audit="$repo_root/scripts/mutation-audit.sh"
verify="$repo_root/scripts/mutation-verify-fix.sh"
queue="$repo_root/scripts/ci/mutation_triage_queue.py"

work="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-mutation-selftest.XXXXXX")"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

failures=0
check() {
  local label="$1"
  shift
  if "$@"; then
    echo "  ok   $label"
  else
    echo "  FAIL $label"
    failures=$((failures + 1))
  fi
}

echo "▶ A. an unscoped audit is refused, not silently empty"
# Without --package, cargo-mutants scopes to the workspace-root package, which
# has no lib/bin, and reports zero mutants. That reads as "nothing to fix".
check "audit without --package exits non-zero" \
  bash -c "! '$audit' 2>/dev/null"
check "audit without --package explains why" \
  bash -c "'$audit' 2>&1 | grep -q 'silently finds zero mutants'"

echo "▶ B. the verify gate refuses incomplete invocations"
check "verify with no args exits non-zero" \
  bash -c "! '$verify' 2>/dev/null"
check "verify without --package exits non-zero" \
  bash -c "! '$verify' 'crates/x/src/y.rs:1:1: replace f with ()' 2>/dev/null"
check "verify rejects a mutant naming a nonexistent file" \
  bash -c "! '$verify' -p some_pkg 'crates/nope/src/gone.rs:1:1: replace f with ()' 2>/dev/null"

echo "▶ C. both scripts refuse to inherit a shared CARGO_TARGET_DIR"
# This one is load-bearing: a shared target dir made the gate report a killed
# mutant as surviving, on identical source. Wrong in both directions.
check "audit warns and unsets CARGO_TARGET_DIR" \
  bash -c "CARGO_TARGET_DIR=/tmp/shared-target '$audit' 2>&1 | grep -q 'ignoring CARGO_TARGET_DIR'"
check "verify warns and unsets CARGO_TARGET_DIR" \
  bash -c "CARGO_TARGET_DIR=/tmp/shared-target '$verify' 2>&1 | grep -q 'ignoring CARGO_TARGET_DIR'"

echo "▶ D. the triage queue reports survivors and scores over viable mutants"
report="$work/report"
mkdir -p "$report"
cat >"$report/missed.txt" <<'EOF'
crates/demo/src/lib.rs:12:5: replace add with ()
EOF
cat >"$report/caught.txt" <<'EOF'
crates/demo/src/lib.rs:20:5: replace sub with ()
crates/demo/src/lib.rs:24:5: replace mul with ()
EOF
cat >"$report/unviable.txt" <<'EOF'
crates/demo/src/lib.rs:30:5: replace thing -> Self with Default::default()
crates/demo/src/lib.rs:34:5: replace other -> Self with Default::default()
crates/demo/src/lib.rs:38:5: replace more -> Self with Default::default()
EOF

python3 "$queue" --report-dir "$report" --output "$work/queue.md" >/dev/null

check "queue counts survivors" \
  grep -q '\*\*1 survivors\*\*' "$work/queue.md"
# The headline number must exclude unviable mutants: they failed to compile and
# say nothing about test strength. Scoring 2/6 instead of 2/3 would understate
# the suite and invite someone to 'fix' non-problems.
check "queue scores over viable mutants only (2/3, not 2/6)" \
  grep -q '\*\*2/3\*\*' "$work/queue.md"
check "queue lists the surviving mutant verbatim" \
  grep -q 'replace add with ()' "$work/queue.md"
check "queue offers the needs-product-decision verdict" \
  grep -q 'needs-product-decision' "$work/queue.md"
check "queue seeds every entry with an unset verdict" \
  grep -q 'verdict: .TODO.' "$work/queue.md"

echo "▶ E. an empty survivor list is reported as clean, not as an error"
empty="$work/empty"
mkdir -p "$empty"
: >"$empty/missed.txt"
: >"$empty/caught.txt"
python3 "$queue" --report-dir "$empty" --output "$work/empty.md" >/dev/null
check "queue states there is nothing to triage" \
  grep -q 'No surviving mutants' "$work/empty.md"

echo "▶ F. a missing report is a loud error, not an empty queue"
check "queue fails when missed.txt is absent" \
  bash -c "! python3 '$queue' --report-dir '$work/absent' --output '$work/x.md' 2>/dev/null"

echo
if [ "$failures" -eq 0 ]; then
  echo "all mutation-harness self-tests passed"
else
  echo "$failures self-test(s) failed" >&2
  exit 1
fi
