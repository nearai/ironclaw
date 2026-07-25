#!/usr/bin/env bash
# Prove that a new test actually kills the mutant it claims to kill.
#
# This is the acceptance gate for mutation-driven test work, and the reason the
# triage queue can be worked at scale without trusting anyone's judgement: the
# criterion is mechanical, not a review opinion.
#
# A fix is accepted only when BOTH hold:
#   1. the suite passes on unmodified code   (the test is not just broken)
#   2. the suite fails with the sabotage applied  (the test actually checks it)
#
# Condition 2 is what a decorative test cannot satisfy, and condition 1 is what
# a test reshaped to match current behaviour cannot satisfy. Together they
# reject both failure modes without a human reading the diff.
#
# Usage:
#   ./scripts/mutation-verify-fix.sh -p <package> \
#       'crates/foo/src/bar.rs:80:5: replace baz with ()'
#
# The mutant string is copied verbatim from mutants.out/missed.txt or the
# triage queue.
#
# Options (env vars):
#   MUT_TIMEOUT=300   Per-mutant timeout in seconds (default: 300)
#
# Exit codes: 0 accepted · 1 rejected · 2 usage/tooling error

set -euo pipefail

MUT_TIMEOUT="${MUT_TIMEOUT:-300}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# A shared CARGO_TARGET_DIR silently corrupts mutation results and must never
# be inherited here. cargo-mutants copies the source tree per job; if every
# copy is redirected at one absolute target directory, parallel jobs clobber
# each other's compiled artifacts and a job can run a test binary built from a
# different job's mutated source. The verdicts then come out wrong in BOTH
# directions — a killed mutant reported as surviving, and worse, a surviving
# mutant reported as caught. This was observed, not theorised: the same mutant
# reported MISSED with a shared dir and caught without one, on identical source.
# Correctness beats the warm cache for a tool whose only job is to be trusted.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  echo "note: ignoring CARGO_TARGET_DIR=$CARGO_TARGET_DIR — a shared target" >&2
  echo "      directory produces wrong mutation verdicts (see comment above)." >&2
  unset CARGO_TARGET_DIR
fi

package=""
mutant=""
while [ $# -gt 0 ]; do
  case "$1" in
    -p | --package)
      package="$2"
      shift 2
      ;;
    -h | --help)
      sed -n '2,26p' "${BASH_SOURCE[0]}" | sed 's|^# \{0,1\}||'
      exit 0
      ;;
    *)
      mutant="$1"
      shift
      ;;
  esac
done

if [ -z "$package" ] || [ -z "$mutant" ]; then
  echo "usage: $0 -p <package> '<mutant line from missed.txt>'" >&2
  exit 2
fi

if ! command -v cargo-mutants >/dev/null 2>&1; then
  echo "error: cargo-mutants not installed (cargo install cargo-mutants --locked)" >&2
  exit 2
fi

# The mutant line is "<file>:<line>:<col>: <description>". cargo-mutants
# --file wants just the path, and re-running the whole file is what lets us
# assert this specific mutant flipped to caught.
file="${mutant%%:*}"
if [ ! -f "$file" ]; then
  echo "error: mutant does not name an existing file: $file" >&2
  exit 2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-mutation-verify.XXXXXX")"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

echo "▶ verifying fix for:"
echo "    $mutant"
echo

set +e
cargo mutants --package "$package" -f "$file" \
  --timeout "$MUT_TIMEOUT" --jobs 3 --output "$work" >"$work/run.log" 2>&1
status=$?
set -e

if [ ! -f "$work/mutants.out/missed.txt" ]; then
  echo "REJECTED: cargo mutants produced no report (status $status)." >&2
  tail -20 "$work/run.log" >&2
  exit 2
fi

# Baseline failure means the suite does not pass on unmodified code, so nothing
# downstream is meaningful — this catches a test written to match a mutant
# rather than the intended behaviour.
if grep -qiE "^(FAILED|ERROR).*[Uu]nmutated baseline|baseline failed" "$work/run.log"; then
  echo "REJECTED: the test suite does not pass on unmodified code." >&2
  echo "          A regression test must pass before it can prove anything." >&2
  exit 1
fi

if grep -Fq "$mutant" "$work/mutants.out/missed.txt"; then
  echo "REJECTED: that sabotage still survives — the suite passes with it applied."
  echo "          The new test does not actually check this behaviour."
  echo
  echo "Still-surviving mutants in $file:"
  sed 's/^/  /' "$work/mutants.out/missed.txt"
  exit 1
fi

if ! grep -Fq "$mutant" "$work/mutants.out/caught.txt" 2>/dev/null; then
  echo "REJECTED: that mutant is neither missed nor caught in this run." >&2
  echo "          It is likely unviable now, or the string does not match any" >&2
  echo "          generated mutant — re-copy it from missed.txt verbatim." >&2
  exit 1
fi

echo "ACCEPTED: the sabotage is now caught."
echo "  suite passes on real code, and fails once this mutant is applied."
echo
echo "Remaining survivors in $file (for the triage queue):"
if [ -s "$work/mutants.out/missed.txt" ]; then
  sed 's/^/  /' "$work/mutants.out/missed.txt"
else
  echo "  none"
fi
