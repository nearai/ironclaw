#!/usr/bin/env bash
# Run a targeted mutation audit and emit a triage-ready queue of survivors.
#
# Mutation testing sabotages one piece of production code at a time and re-runs
# the tests. A sabotage the suite still passes ("MISSED") is code with no
# assertion behind it. Line coverage cannot find these: it proves a line ran,
# not that anything checked the result.
#
# Usage:
#   ./scripts/mutation-audit.sh -p ironclaw_event_projections \
#       crates/ironclaw_event_projections/src/runtime_projection.rs
#   ./scripts/mutation-audit.sh -p ironclaw_dispatcher            # whole package
#
# Options (env vars):
#   MUT_JOBS=3          Parallel mutants (default: 3)
#   MUT_TIMEOUT=300     Per-mutant timeout in seconds (default: 300)
#   MUT_OUT=mutants.out Report directory (default: mutants.out)
#   MUT_ITERATE=0       Set to 1 to skip mutants caught in a previous run
#
# Output: $MUT_OUT/triage-queue.md — one entry per survivor with its sabotage
# diff and the enclosing source, so a reviewer never has to go hunting.
#
# Requires: cargo-mutants (install: cargo install cargo-mutants --locked)
#
# Deliberately NOT a CI gate. See docs/internal/mutation-audit.md — roughly a
# third of survivors are "equivalent mutants" that no test can catch, so a
# mutation score would be a flake generator. This is a periodic audit whose
# output is a work queue.

set -euo pipefail

MUT_JOBS="${MUT_JOBS:-3}"
MUT_TIMEOUT="${MUT_TIMEOUT:-300}"
MUT_OUT="${MUT_OUT:-mutants.out}"
MUT_ITERATE="${MUT_ITERATE:-0}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# See scripts/mutation-verify-fix.sh for the full rationale: a shared
# CARGO_TARGET_DIR lets parallel mutant builds clobber each other's artifacts,
# so a job can test a binary built from a different mutant's source. Verdicts
# then come out wrong in both directions. Never inherit it.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  echo "note: ignoring CARGO_TARGET_DIR=$CARGO_TARGET_DIR — a shared target" >&2
  echo "      directory produces wrong mutation verdicts." >&2
  unset CARGO_TARGET_DIR
fi

if ! command -v cargo-mutants >/dev/null 2>&1; then
  echo "error: cargo-mutants not installed." >&2
  echo "       cargo install cargo-mutants --locked" >&2
  exit 1
fi

package=""
files=()
while [ $# -gt 0 ]; do
  case "$1" in
    -p | --package)
      package="$2"
      shift 2
      ;;
    -h | --help)
      sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's|^# \{0,1\}||'
      exit 0
      ;;
    *)
      files+=("$1")
      shift
      ;;
  esac
done

if [ -z "$package" ]; then
  # Without --package, cargo-mutants scopes to the workspace-root package,
  # which has no lib or bin of its own and therefore yields zero mutants —
  # a silent no-op that reads like "nothing to fix".
  echo "error: -p/--package is required (the workspace root has no lib/bin," >&2
  echo "       so an unscoped run silently finds zero mutants)." >&2
  exit 1
fi

args=(--package "$package" --timeout "$MUT_TIMEOUT" --jobs "$MUT_JOBS" --output "$MUT_OUT")
for file in "${files[@]:-}"; do
  [ -n "$file" ] && args+=(-f "$file")
done
[ "$MUT_ITERATE" = "1" ] && args+=(--iterate)

echo "▶ mutation audit: package=$package files=${files[*]:-<all>}"
set +e
cargo mutants "${args[@]}"
mutants_status=$?
set -e

# Exit 2 means surviving mutants were found, which is the expected outcome of an
# audit, not a failure. Anything else is a real error.
if [ "$mutants_status" -ne 0 ] && [ "$mutants_status" -ne 2 ]; then
  echo "error: cargo mutants failed with status $mutants_status" >&2
  exit "$mutants_status"
fi

python3 "$repo_root/scripts/ci/mutation_triage_queue.py" \
  --report-dir "$MUT_OUT" \
  --output "$MUT_OUT/triage-queue.md"

echo
echo "▶ triage queue: $MUT_OUT/triage-queue.md"
echo "  Assign each survivor one verdict (see docs/internal/mutation-audit.md):"
echo "    real-gap · equivalent-mutant · needs-product-decision"
echo "  A fix for a real-gap survivor is only accepted once"
echo "  scripts/mutation-verify-fix.sh proves it moves MISSED -> caught."
