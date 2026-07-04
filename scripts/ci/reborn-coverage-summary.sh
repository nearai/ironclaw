#!/usr/bin/env bash
#
# Render a Reborn-scoped coverage summary from a merged, crate-filtered lcov
# tracefile (see scripts/ci/reborn-coverage-merge-lcov.sh, which merges the 5
# per-lane `cargo llvm-cov report --lcov` outputs from the
# `reborn-integration-coverage` matrix in .github/workflows/reborn-tests.yml
# and filters to crates/ironclaw_* source files).
#
# The lcov input already covers every crate the int-tier suites link (all
# workspace `ironclaw_*` crates, not a Reborn-only allowlist) — this script's
# job is purely per-crate aggregation, exemption handling, and rendering.
#
# Usage:
#   reborn-coverage-summary.sh <lcov-path> <exemptions-toml-path>
#     Writes a GitHub-flavored Markdown report to stdout. The caller redirects
#     it into "$GITHUB_STEP_SUMMARY" (or anywhere else).
#   reborn-coverage-summary.sh --zero-crates <lcov-path> <exemptions-toml-path>
#     Writes, one per line, the Reborn crates that have instrumented lines but
#     zero of them covered (the "breadth" holes). Same crate aggregation as
#     the report — this script is the single owner of that computation so
#     reborn-coverage-comment.sh never has to recompute it.
#
# Exemptions (tests/integration/coverage-exemptions.toml, schema documented
# there): each [[exemption]] `module` path is dropped from the per-crate
# covered/total accounting entirely (it neither helps nor hurts a crate's
# percentage) and listed, with its reason + issue, in its own report section.
# An exemptions file with zero entries is valid (nothing is exempted yet).

set -euo pipefail

mode="report"
if [ "${1:-}" = "--zero-crates" ]; then
  mode="zero-crates"
  shift
fi

lcov_path="${1:?usage: reborn-coverage-summary.sh [--zero-crates] <lcov-path> <exemptions-toml-path>}"
exemptions_path="${2:?usage: reborn-coverage-summary.sh [--zero-crates] <lcov-path> <exemptions-toml-path>}"

if [ ! -f "${lcov_path}" ]; then
  echo "coverage lcov file not found: ${lcov_path}" >&2
  exit 1
fi

if [ ! -f "${exemptions_path}" ]; then
  echo "coverage exemptions manifest not found: ${exemptions_path}" >&2
  exit 1
fi

python3 - "${mode}" "${lcov_path}" "${exemptions_path}" <<'PY'
import re
import sys
import tomllib

mode, lcov_path, exemptions_path = sys.argv[1], sys.argv[2], sys.argv[3]

crate_re = re.compile(r"(?:^|/)crates/(ironclaw_[A-Za-z0-9_]+)/")


def round2(value: float) -> str:
    rounded = round(value, 2)
    # Match jq's `round` behavior for whole numbers (no trailing ".0").
    if rounded == int(rounded):
        return str(int(rounded))
    return str(rounded)


# ---------------------------------------------------------------------------
# Parse the exemptions manifest.
# ---------------------------------------------------------------------------

with open(exemptions_path, "rb") as fh:
    manifest = tomllib.load(fh)

exemptions = manifest.get("exemption", [])
exempt_modules: set[str] = set()
for entry in exemptions:
    module = entry.get("module")
    if not module:
        print(f"malformed exemption entry (missing 'module'): {entry}", file=sys.stderr)
        sys.exit(1)
    if not entry.get("reason"):
        print(f"exemption for '{module}' is missing 'reason'", file=sys.stderr)
        sys.exit(1)
    if not entry.get("issue"):
        print(f"exemption for '{module}' is missing 'issue'", file=sys.stderr)
        sys.exit(1)
    if not module.startswith("crates/"):
        print(f"exemption module path '{module}' must be repo-relative and start with 'crates/'", file=sys.stderr)
        sys.exit(1)
    exempt_modules.add(module)

# ---------------------------------------------------------------------------
# Parse the lcov tracefile: per-file (covered, total) from LH:/LF:, per crate.
# ---------------------------------------------------------------------------

by_crate: dict[str, dict[str, int]] = {}
total = 0
hit = 0

current_file = None
current_covered = None
current_count = None

with open(lcov_path, "r", encoding="utf-8") as fh:
    for raw_line in fh:
        line = raw_line.rstrip("\n")
        if line.startswith("SF:"):
            current_file = line[len("SF:"):]
            current_covered = None
            current_count = None
        elif line.startswith("LF:"):
            current_count = int(line[len("LF:"):])
        elif line.startswith("LH:"):
            current_covered = int(line[len("LH:"):])
        elif line == "end_of_record":
            if current_file is not None and current_covered is not None and current_count is not None:
                # Exempted files are skipped entirely: they never enter the
                # per-crate or aggregate accounting (neither help nor hurt).
                is_exempt = any(current_file.endswith("/" + m) or current_file == m for m in exempt_modules)
                if not is_exempt:
                    match = crate_re.search(current_file)
                    if match:
                        crate = match.group(1)
                        bucket = by_crate.setdefault(crate, {"covered": 0, "count": 0})
                        bucket["covered"] += current_covered
                        bucket["count"] += current_count
                        total += current_count
                        hit += current_covered
            current_file = None
            current_covered = None
            current_count = None

pct = (hit / total * 100) if total > 0 else 0.0

rows = []
for crate, counts in by_crate.items():
    count = counts["count"]
    covered = counts["covered"]
    crate_pct = (covered / count * 100) if count > 0 else 0.0
    rows.append({"crate": crate, "covered": covered, "count": count, "pct": crate_pct})
rows.sort(key=lambda r: r["pct"])

if mode == "zero-crates":
    for row in rows:
        if row["count"] > 0 and row["covered"] == 0:
            print(row["crate"])
    sys.exit(0)

# --------------------------- report mode -----------------------------------

lines = ["## Reborn integration-tier coverage", ""]

if total > 0:
    lines.append(f"**Line coverage (Reborn crates): {round2(pct)}%** — {hit} / {total} lines")
else:
    lines.append("**No Reborn crate coverage data found** (0 instrumented lines matched the Reborn crate filter).")
lines.append("")

if total > 0:
    lines.append(f"<details><summary>Per-crate breakdown ({len(rows)} crates, lowest-covered first)</summary>")
    lines.append("")
    lines.append("| Crate | Line % | Covered / Total |")
    lines.append("|---|---:|---:|")
    for row in rows:
        lines.append(f"| `{row['crate']}` | {round2(row['pct'])}% | {row['covered']} / {row['count']} |")
    lines.append("")
    lines.append("</details>")
    lines.append("")
    lines.append(
        "_This signal is informational: coverage never gates the PR — not the percentage, "
        "not the per-crate holes, not the 0-coverage callout._"
    )

lines.append("")
lines.append(f"<details><summary>Exemptions ({len(exemptions)} file(s) excluded from the accounting above)</summary>")
lines.append("")
if exemptions:
    lines.append("| Module | Reason | Issue |")
    lines.append("|---|---|---|")
    for entry in sorted(exemptions, key=lambda e: e["module"]):
        lines.append(f"| `{entry['module']}` | {entry['reason']} | {entry['issue']} |")
else:
    lines.append("_No exemptions configured._")
lines.append("")
lines.append("</details>")

print("\n".join(lines))
PY
