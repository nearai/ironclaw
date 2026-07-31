#!/usr/bin/env bash
#
# Merge N per-lane lcov tracefiles from the Reborn integration-tier coverage
# lanes (.github/workflows/reborn-tests.yml's `reborn-integration-coverage`
# matrix — 4 flat partitions + 1 group lane, each producing its lcov via one
# combined `cargo llvm-cov ... test --lcov` invocation) into one deterministic tracefile, filtered to
# workspace crate source files.
#
# Every lane instruments the SAME workspace build, so the same source file can
# appear in more than one lane's tracefile with different per-line hit counts
# (each lane only exercised its own subset of test binaries against that
# shared file). A naive concatenation would leave duplicate `SF:` blocks for
# that file instead of one true picture of "how many of the 5 lanes hit this
# line" — so this script sums DA: hit counts per (file, line) across all
# inputs and recomputes LF/LH from the merged counts, rather than trusting any
# single lane's LF/LH.
#
# Deliberately a small Python merger, not the `lcov` CLI (`lcov -a ... -o`):
# avoids an extra apt-get dependency in the coverage-report job (see
# scripts/ci/install-ci-apt-packages.sh for that pattern, used elsewhere) and
# keeps this fully testable locally without installing anything.
#
# Which source files are in scope is resolved from the crate tree itself
# (scripts/ci/lib/crate_tree.py), not from a `crates/ironclaw_*` path pattern:
# a pattern keyed to today's flat layout stops matching the moment crates move
# into family directories, and this script would then merge zero files, write
# an empty tracefile, and exit 0 — coverage dark, CI green. See
# docs/reborn/target-architecture/CHECKLIST.md WS10.
#
# Test/override env vars (used by test-reborn-coverage.sh; unset in prod):
#   IRONCLAW_REPO_ROOT  repository root whose crate tree defines the filter
#                       (default: this script's ../..)
#
# Usage:
#   reborn-coverage-merge-lcov.sh <output.lcov> <input1.lcov> [input2.lcov ...]

set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: $0 <output.lcov> <input1.lcov> [input2.lcov ...]" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="${IRONCLAW_REPO_ROOT:-$(cd "${script_dir}/../.." && pwd)}"

output_path="$1"
shift

for input_path in "$@"; do
  if [ ! -f "${input_path}" ]; then
    echo "input lcov file not found: ${input_path}" >&2
    exit 1
  fi
done

IRONCLAW_CRATE_TREE_LIB="${script_dir}/lib" \
python3 - "${repo_root}" "${output_path}" "$@" <<'PY'
import os
import re
import sys

sys.path.insert(0, os.environ["IRONCLAW_CRATE_TREE_LIB"])
from crate_tree import CrateTreeError, crate_directories  # noqa: E402

repo_root = sys.argv[1]
output_path = sys.argv[2]
input_paths = sys.argv[3:]

# filename -> { line_number: hit_count }
files: dict[str, dict[int, int]] = {}

# Scope = every source file that lives inside a workspace crate directory,
# wherever that directory sits under `crates/`. This is the "all crates" scope
# for the Reborn integration-tier coverage report, a superset of the historical
# Reborn-family-only allowlist (the int-tier suites now exercise the whole
# workspace closure, not just the Reborn crate families).
#
# `SF:` records carry absolute checkout paths, so the match is anchored on a
# path-segment boundary rather than the start of the string. Anchoring on the
# discovered directory list (rather than on `crates/<anything>/`) also keeps
# third-party sources out: several vendored crates ship their own `crates/`
# subdirectory (`.../wasmtime-46.0.1/crates/wasmtime/src/lib.rs`), and those
# must stay out of the Reborn coverage denominator.
try:
    crate_dirs = crate_directories(repo_root)
except CrateTreeError as error:
    print(f"reborn-coverage-merge-lcov: {error}", file=sys.stderr)
    sys.exit(1)

crate_re = re.compile(
    "(?:^|/)(?:" + "|".join(re.escape(directory) for directory in crate_dirs) + ")/"
)

kept_per_input: list[tuple[str, int]] = []

for input_path in input_paths:
    kept_here: set[str] = set()
    current_file = None
    keep_current = False
    with open(input_path, "r", encoding="utf-8") as fh:
        for raw_line in fh:
            line = raw_line.rstrip("\n")
            if line.startswith("SF:"):
                current_file = line[len("SF:"):]
                keep_current = bool(crate_re.search(current_file))
                if keep_current:
                    files.setdefault(current_file, {})
                    kept_here.add(current_file)
            elif line.startswith("DA:") and keep_current and current_file is not None:
                rest = line[len("DA:"):]
                parts = rest.split(",")
                line_no = int(parts[0])
                hit_count = int(parts[1])
                bucket = files[current_file]
                bucket[line_no] = bucket.get(line_no, 0) + hit_count
            elif line == "end_of_record":
                current_file = None
                keep_current = False
            # LF:/LH:/other record kinds (FN, BRDA, ...) are ignored on input —
            # this report only needs line coverage, and LF/LH are recomputed
            # from the merged DA: counts below so a single lane's summary
            # never gets trusted as the merged truth.
    kept_per_input.append((input_path, len(kept_here)))

# Per-input contribution, on stderr so the merged tracefile on stdout-adjacent
# paths stays untouched. A lane that suddenly contributes far fewer files than
# its siblings is visible here before it is visible in a coverage number.
for input_path, kept in kept_per_input:
    print(
        f"reborn-coverage-merge-lcov: {input_path}: {kept} crate source file(s)",
        file=sys.stderr,
    )

if not files:
    # Fail closed. Historically this wrote an empty tracefile and exited 0,
    # which is indistinguishable from "the lanes ran and found nothing" — and
    # is exactly what a path-shape change produces: every `SF:` record filtered
    # out, an empty report, a green pipeline, and no coverage signal at all.
    # There is no healthy configuration in which N lane tracefiles over an
    # instrumented workspace build contain zero workspace crate sources.
    print(
        "reborn-coverage-merge-lcov: no workspace crate source files matched across "
        f"{len(input_paths)} input tracefile(s).\n"
        f"  crate directories discovered under {repo_root}: {len(crate_dirs)}\n"
        "  Either the lane tracefiles are empty/for the wrong build, or the crate tree "
        "moved and the paths inside them no longer resolve. Refusing to write an empty "
        "coverage report (docs/reborn/target-architecture/CHECKLIST.md WS10).",
        file=sys.stderr,
    )
    sys.exit(1)

with open(output_path, "w", encoding="utf-8") as out:
    for filename in sorted(files):
        lines = files[filename]
        out.write(f"SF:{filename}\n")
        for line_no in sorted(lines):
            out.write(f"DA:{line_no},{lines[line_no]}\n")
        lines_found = len(lines)
        lines_hit = sum(1 for count in lines.values() if count > 0)
        out.write(f"LF:{lines_found}\n")
        out.write(f"LH:{lines_hit}\n")
        out.write("end_of_record\n")
PY
