#!/usr/bin/env bash
#
# Merge N per-lane lcov tracefiles from the Reborn integration-tier coverage
# lanes (.github/workflows/reborn-tests.yml's `reborn-integration-coverage`
# matrix — 4 flat partitions + 1 group lane, each producing its lcov via one
# combined `cargo llvm-cov ... test --lcov` invocation) into one deterministic tracefile, filtered to `crates/ironclaw_*`
# source files.
#
# Every lane instruments the SAME workspace build, so the same source file can
# appear in more than one lane's tracefile with different per-line hit counts
# (each lane only exercised its own subset of test binaries against that
# shared file). A naive concatenation would leave duplicate `SF:` blocks for
# that file instead of one true picture of "how many lanes hit this line or
# branch" — so this script sums DA: hit counts per (file, line) and BRDA:
# counts per (file, line, block, branch) across all inputs. LF/LH and BRF/BRH
# are recomputed from the merged counts rather than trusting any one lane.
#
# Deliberately a small Python merger, not the `lcov` CLI (`lcov -a ... -o`):
# avoids an extra apt-get dependency in the coverage-report job (see
# scripts/ci/install-ci-apt-packages.sh for that pattern, used elsewhere) and
# keeps this fully testable locally without installing anything.
#
# Usage:
#   reborn-coverage-merge-lcov.sh <output.lcov> <input1.lcov> [input2.lcov ...]

set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: $0 <output.lcov> <input1.lcov> [input2.lcov ...]" >&2
  exit 2
fi

output_path="$1"
shift

for input_path in "$@"; do
  if [ ! -f "${input_path}" ]; then
    echo "input lcov file not found: ${input_path}" >&2
    exit 1
  fi
done

python3 - "${output_path}" "$@" <<'PY'
import re
import sys

output_path = sys.argv[1]
input_paths = sys.argv[2:]

# filename -> {"lines": {line_number: hit_count},
#              "branches": {(line, block, branch): hit_count}}
files: dict[str, dict[str, dict]] = {}

# Only source files under a crates/ironclaw_* directory are kept — this is
# the "all 71 crates" scope for the Reborn integration-tier coverage report,
# a superset of the historical Reborn-family-only allowlist (the int-tier
# suites now exercise the whole workspace closure, not just the Reborn crate
# families).
crate_re = re.compile(r"(?:^|/)crates/(ironclaw_[A-Za-z0-9_]+)/")

for input_path in input_paths:
    current_file = None
    keep_current = False
    with open(input_path, "r", encoding="utf-8") as fh:
        for raw_line in fh:
            line = raw_line.rstrip("\n")
            if line.startswith("SF:"):
                current_file = line[len("SF:"):]
                keep_current = bool(crate_re.search(current_file))
                if keep_current:
                    files.setdefault(current_file, {"lines": {}, "branches": {}})
            elif line.startswith("DA:") and keep_current and current_file is not None:
                rest = line[len("DA:"):]
                parts = rest.split(",")
                line_no = int(parts[0])
                hit_count = int(parts[1])
                bucket = files[current_file]["lines"]
                bucket[line_no] = bucket.get(line_no, 0) + hit_count
            elif line.startswith("BRDA:") and keep_current and current_file is not None:
                line_no, block, branch, taken = line[len("BRDA:"):].split(",", 3)
                key = (int(line_no), block, branch)
                hit_count = 0 if taken == "-" else int(taken)
                bucket = files[current_file]["branches"]
                bucket[key] = bucket.get(key, 0) + hit_count
            elif line == "end_of_record":
                current_file = None
                keep_current = False
            # LF:/LH:/BRF:/BRH:/function records are ignored on input because
            # their summaries are recomputed below.

if not files:
    # A run with zero matching files is a legitimate (if unlikely) outcome —
    # write an empty tracefile rather than erroring, mirroring how the
    # downstream summary renderer treats "no data" as its own reportable case
    # rather than a script failure.
    open(output_path, "w", encoding="utf-8").close()
    sys.exit(0)

with open(output_path, "w", encoding="utf-8") as out:
    for filename in sorted(files):
        lines = files[filename]["lines"]
        branches = files[filename]["branches"]
        out.write(f"SF:{filename}\n")
        for line_no in sorted(lines):
            out.write(f"DA:{line_no},{lines[line_no]}\n")
        lines_found = len(lines)
        lines_hit = sum(1 for count in lines.values() if count > 0)
        out.write(f"LF:{lines_found}\n")
        out.write(f"LH:{lines_hit}\n")
        for (line_no, block, branch), count in sorted(branches.items()):
            out.write(f"BRDA:{line_no},{block},{branch},{count}\n")
        out.write(f"BRF:{len(branches)}\n")
        out.write(f"BRH:{sum(1 for count in branches.values() if count > 0)}\n")
        out.write("end_of_record\n")
PY
