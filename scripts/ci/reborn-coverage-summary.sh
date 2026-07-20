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
# there): each [[exemption]] is a per-file `module` path or whole-crate
# `crate` name (exactly one), dropped from the accounting and listed separately.

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

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

PYTHONPATH="${script_dir}/lib${PYTHONPATH:+:${PYTHONPATH}}" python3 - "${mode}" "${lcov_path}" "${exemptions_path}" <<'PY'
import sys

import reborn_coverage_lcov as cov

mode, lcov_path, exemptions_path = sys.argv[1], sys.argv[2], sys.argv[3]


def round2(value: float) -> str:
    rounded = round(value, 2)
    # Match jq's `round` behavior for whole numbers (no trailing ".0").
    if rounded == int(rounded):
        return str(int(rounded))
    return str(rounded)


# Exemption parsing and lcov aggregation are shared with
# reborn-coverage-ratchet.sh via scripts/ci/lib/reborn_coverage_lcov.py — this
# script owns only the rendering below.
exempt_modules, exempt_crates, exemptions = cov.load_exemptions(exemptions_path)
by_crate, total, hit = cov.aggregate(lcov_path, exempt_modules, exempt_crates)

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
        "_This table itself is informational and never gates the PR on its own — not the "
        "percentage, not the per-crate holes, not the 0-coverage callout. A separate coverage "
        "ratchet (dry-run until enforce=true; see tests/integration/coverage-floor.toml) can "
        "fail the build on specific configured floors._"
    )

lines.append("")
lines.append(f"<details><summary>Exemptions ({len(exemptions)} entry/entries excluded from the accounting above)</summary>")
lines.append("")
if exemptions:
    lines.append("| Module / Crate | Reason | Issue |")
    lines.append("|---|---|---|")
    for entry in sorted(exemptions, key=lambda e: e["label"]):
        lines.append(f"| `{entry['label']}` | {entry['reason']} | {entry['issue']} |")
else:
    lines.append("_No exemptions configured._")
lines.append("")
lines.append("</details>")

print("\n".join(lines))
PY
