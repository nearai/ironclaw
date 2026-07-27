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
#   MUT_OUT=.           Directory to create mutants.out in (default: repo root)
#   MUT_ITERATE=0       Set to 1 to skip mutants caught in a previous run
#
# Output: $MUT_OUT/mutants.out/triage-queue.md — one entry per survivor with its
# sabotage diff and the enclosing source, so a reviewer never has to go hunting.
# cargo-mutants always creates a directory literally named mutants.out inside
# --output, so MUT_OUT names that directory's parent, not the report itself.
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
MUT_OUT="${MUT_OUT:-.}"
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

# Checked after argument validation on purpose: a usage error must report the
# usage error, not a missing tool. Ordering these the other way round made the
# unscoped-run guard unreachable on any machine without cargo-mutants installed
# — and made this script's self-tests pass only because the author had it.
if ! command -v cargo-mutants >/dev/null 2>&1; then
  echo "error: cargo-mutants not installed." >&2
  echo "       cargo install cargo-mutants --locked" >&2
  exit 1
fi

# A path that no longer exists is the likeliest way to audit nothing: files get
# moved between crates, an old command line keeps working, cargo-mutants filters
# down to zero mutants, and the run reports success over a file it never read.
args=(--package "$package" --timeout "$MUT_TIMEOUT" --jobs "$MUT_JOBS" --output "$MUT_OUT")
for file in "${files[@]:-}"; do
  [ -z "$file" ] && continue
  if [ ! -f "$repo_root/$file" ] && [ ! -f "$file" ]; then
    echo "error: no such file to audit: $file" >&2
    echo "       it may have moved — an audit of a stale path finds zero" >&2
    echo "       mutants and reports a clean run over code it never read." >&2
    exit 1
  fi
  args+=(-f "$file")
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

# cargo-mutants creates mutants.out *inside* --output. Reading the results from
# $MUT_OUT itself finds nothing, which failed every audit at this final step —
# see scripts/test-mutation-audit.sh section G.
report_dir="$MUT_OUT/mutants.out"

# Zero mutants is never a pass. cargo-mutants only WARNs ("No mutants found
# under the active filters") and still exits 0, so without this the script
# prints an empty triage queue and reads as "nothing to fix" — the same clean
# bill of health the --package guard above exists to prevent. Caught here as
# well as at the filter, because filters are not the only way to reach zero.
total=0
for outcome in caught missed unviable timeout; do
  file="$report_dir/$outcome.txt"
  [ -f "$file" ] && total=$((total + $(grep -c . "$file" || true)))
done
if [ "$total" -eq 0 ]; then
  echo "error: the run produced zero mutants, so nothing was tested." >&2
  echo "       an empty result is not a passing audit — check that the" >&2
  echo "       package and file filters still match real code." >&2
  exit 1
fi

python3 "$repo_root/scripts/ci/mutation_triage_queue.py" \
  --report-dir "$report_dir" \
  --output "$report_dir/triage-queue.md"

echo
echo "▶ triage queue: $report_dir/triage-queue.md"
echo "  Assign each survivor one verdict (see docs/internal/mutation-audit.md):"
echo "    real-gap · equivalent-mutant · needs-product-decision"
echo "  A fix for a real-gap survivor is only accepted once"
echo "  scripts/mutation-verify-fix.sh proves it moves MISSED -> caught."
