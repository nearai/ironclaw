#!/usr/bin/env bash
#
# Coverage ratchet gate: compares the SAME merged, exemption-filtered lcov
# aggregation reborn-coverage-summary.sh already computes (via
# scripts/ci/lib/reborn_coverage_lcov.py — reused here, not reimplemented)
# against a committed floor file (tests/integration/coverage-floor.toml).
# Pure post-processing over already-produced lcov + TOML — never re-runs
# tests, never triggers a second CI pass.
#
# Usage:
#   reborn-coverage-ratchet.sh <lcov-path> <exemptions-toml-path> <floor-toml-path>
#
# Floor file schema is documented in tests/integration/coverage-floor.toml's
# own header. Summary of gate behavior:
#   - [global].enforce = false: DRY-RUN. Ratchet/threshold violations are
#     printed (prefixed "[dry-run, would FAIL]") but never fail the script.
#   - [global].enforce = true: violations exit 1.
#   - Schema errors (missing required fields, a crate double-listed as both
#     floored and whole-crate-exempted, duplicate [[crate]] names, a missing
#     floor-toml file) always exit 1, regardless of `enforce` — these are
#     manifest bugs, not coverage regressions, and are never soaked silently.
#   - A crate entry may configure floor_percent and/or floor_covered_lines;
#     the crate fails if EITHER configured check is violated (both must
#     hold to pass — floor_covered_lines exists specifically to catch
#     denominator-dilution masking a real numerator regression).
#   - The denominator delta vs each entry's own captured_total_lines is
#     always printed (never itself gates) so a reviewer can eyeball whether
#     a violation is a legitimate #5656-style shift or a real regression.
#   - An unconditional "Ratchet mode: DRY-RUN/ENFORCING" banner is always
#     the first output line, so a soak-period all-green run still makes the
#     gate's advisory status visible without opening the floor file.

set -euo pipefail

lcov_path="${1:?usage: reborn-coverage-ratchet.sh <lcov-path> <exemptions-toml-path> <floor-toml-path>}"
exemptions_path="${2:?usage: reborn-coverage-ratchet.sh <lcov-path> <exemptions-toml-path> <floor-toml-path>}"
floor_path="${3:?usage: reborn-coverage-ratchet.sh <lcov-path> <exemptions-toml-path> <floor-toml-path>}"

if [ ! -f "${lcov_path}" ]; then
  echo "coverage lcov file not found: ${lcov_path}" >&2
  exit 1
fi

if [ ! -f "${exemptions_path}" ]; then
  echo "coverage exemptions manifest not found: ${exemptions_path}" >&2
  exit 1
fi

if [ ! -f "${floor_path}" ]; then
  echo "coverage floor manifest not found: ${floor_path}" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

PYTHONPATH="${script_dir}/lib${PYTHONPATH:+:${PYTHONPATH}}" python3 - "${lcov_path}" "${exemptions_path}" "${floor_path}" <<'PY'
import sys
import tomllib

import reborn_coverage_lcov as cov

lcov_path, exemptions_path, floor_path = sys.argv[1], sys.argv[2], sys.argv[3]


def round2(value: float) -> str:
    rounded = round(value, 2)
    if rounded == int(rounded):
        return str(int(rounded))
    return str(rounded)


def signed(value) -> str:
    return f"+{value}" if value >= 0 else str(value)


exempt_modules, exempt_crates, _exemptions = cov.load_exemptions(exemptions_path)
by_crate, total, hit = cov.aggregate(lcov_path, exempt_modules, exempt_crates)

with open(floor_path, "rb") as fh:
    floor = tomllib.load(fh)

# ---------------------------------------------------------------------------
# Schema validation. All errors below exit 1 unconditionally (schema bugs,
# not coverage regressions) BEFORE enforce is ever consulted for exit code.
# ---------------------------------------------------------------------------

global_cfg = floor.get("global")
if not isinstance(global_cfg, dict):
    print("coverage floor manifest missing required [global] section", file=sys.stderr)
    sys.exit(1)

if "floor_percent" not in global_cfg:
    print("coverage floor manifest [global] missing required 'floor_percent'", file=sys.stderr)
    sys.exit(1)

enforce_raw = global_cfg.get("enforce", False)
if not isinstance(enforce_raw, bool):
    print(
        f"coverage floor manifest [global].enforce must be a boolean, got {enforce_raw!r}",
        file=sys.stderr,
    )
    sys.exit(1)
enforce = enforce_raw
floor_percent_global = global_cfg["floor_percent"]
tolerance_percent_global = float(global_cfg.get("tolerance_percent", 0.5))
captured_total_lines_global = global_cfg.get("captured_total_lines")

crate_entries_raw = floor.get("crate", [])
if "crate" in floor and not isinstance(crate_entries_raw, list):
    print(
        "coverage floor manifest has a 'crate' key that is not an array of tables — "
        "per-crate floors must use [[crate]], not a single [crate] table",
        file=sys.stderr,
    )
    sys.exit(1)

seen_names: set[str] = set()
validated_crates = []
for entry in crate_entries_raw:
    name = entry.get("name")
    if not name:
        print("coverage floor manifest has a [[crate]] entry missing required 'name'", file=sys.stderr)
        sys.exit(1)
    if name in seen_names:
        print(f"coverage floor manifest has a duplicate [[crate]] entry for '{name}'", file=sys.stderr)
        sys.exit(1)
    seen_names.add(name)

    if name in exempt_crates:
        print(
            f"coverage floor manifest conflict for '{name}': crate is also whole-crate-exempted in "
            "coverage-exemptions.toml (zero accounted lines, cannot be floored)",
            file=sys.stderr,
        )
        sys.exit(1)

    has_pct = entry.get("floor_percent") is not None
    has_lines = entry.get("floor_covered_lines") is not None
    if not has_pct and not has_lines:
        print(
            f"coverage floor manifest entry for '{name}' is missing both 'floor_percent' and "
            "'floor_covered_lines' (at least one required)",
            file=sys.stderr,
        )
        sys.exit(1)

    validated_crates.append(entry)

# ---------------------------------------------------------------------------
# Build one evaluable entry per gated target: the [global] aggregate, plus
# each opted-in [[crate]].
# ---------------------------------------------------------------------------


def evaluate(name, covered, count, floor_percent, floor_covered_lines, tol_pct, tol_lines, captured_total):
    """Returns (failed_checks, body_lines) for one gated entry.

    Divide-by-zero guarded: a crate with count == 0 (e.g. renamed/removed,
    R12) is treated as 0% / 0 covered rather than crashing.
    """
    pct = (covered / count * 100) if count > 0 else 0.0
    failed = []
    body = [f"  observed: {round2(pct)}% ({covered} / {count} lines)"]

    if floor_percent is not None:
        eff_floor_pct = floor_percent - tol_pct
        body.append(
            f"  floor:    {round2(floor_percent)}% (tolerance {round2(tol_pct)}pp -> effective floor {round2(eff_floor_pct)}%)"
        )
        if pct < eff_floor_pct:
            failed.append("percent")

    if floor_covered_lines is not None:
        eff_floor_lines = floor_covered_lines - tol_lines
        body.append(
            f"  floor_covered_lines: {floor_covered_lines} (tolerance {tol_lines} lines -> effective floor {eff_floor_lines})"
        )
        if covered < eff_floor_lines:
            failed.append("lines")

    # Denominator delta vs this entry's own captured baseline — printed
    # unconditionally (pass or fail), never itself a gate. A captured_total
    # of None or <=0 means no baseline has been captured yet (e.g. the
    # placeholder landing value) — skip the note rather than print a
    # meaningless "vs 0 lines" delta.
    if captured_total is not None and captured_total > 0:
        delta = count - captured_total
        pct_delta = delta / captured_total * 100
        material = abs(pct_delta) > 5
        flag = "material change (>5%)" if material else "not a material change"
        pct_delta_str = round2(pct_delta)
        if pct_delta >= 0:
            pct_delta_str = f"+{pct_delta_str}"
        body.append(
            f"  denominator: {count} lines now vs {captured_total} at floor capture "
            f"({signed(delta)} lines, {pct_delta_str}%) — {flag}"
        )

    return failed, body


entries = []

entries.append(
    {
        "name": "global",
        "is_global": True,
        "covered": hit,
        "count": total,
        "floor_percent": floor_percent_global,
        "floor_covered_lines": None,
        "tol_pct": tolerance_percent_global,
        "tol_lines": None,
        "captured_total": captured_total_lines_global,
    }
)

for entry in validated_crates:
    name = entry["name"]
    counts = by_crate.get(name, {"covered": 0, "count": 0})
    entries.append(
        {
            "name": name,
            "is_global": False,
            "covered": counts["covered"],
            "count": counts["count"],
            "floor_percent": entry.get("floor_percent"),
            "floor_covered_lines": entry.get("floor_covered_lines"),
            "tol_pct": float(entry.get("tolerance_percent", tolerance_percent_global)),
            "tol_lines": int(entry.get("tolerance_lines", 20)),
            "captured_total": entry.get("captured_total_lines"),
        }
    )

results = []
any_violation = False
for e in entries:
    failed, body = evaluate(
        e["name"], e["covered"], e["count"], e["floor_percent"], e["floor_covered_lines"],
        e["tol_pct"], e["tol_lines"], e["captured_total"],
    )
    failing = len(failed) > 0
    any_violation = any_violation or failing
    results.append((e, failing, body))

# ---------------------------------------------------------------------------
# Output. Mode banner is unconditional — printed on every run, pass or fail,
# so a soak-period all-green dry-run run still shows the gate is advisory.
# ---------------------------------------------------------------------------

n_crate_floors = len(validated_crates)
n_would_fail = sum(1 for _e, failing, _body in results if failing)

if enforce:
    print("Ratchet mode: ENFORCING")
else:
    print(f"Ratchet mode: DRY-RUN (enforce=false) — {n_crate_floors} crate floor(s) configured, {n_would_fail} would-fail")
print()

FIX_COMMON = [
    "  To fix:",
    "    - If this is a real coverage regression: add tests, don't touch the floor file.",
    "    - If this is a legitimate denominator/numerator shift EITHER direction —",
    "      growth (new/renamed module entering instrumentation, e.g. #5656) OR",
    "      shrinkage (a code+test deletion legitimately lowering covered lines,",
    "      even when the denominator barely moves): update",
]

for e, failing, body in results:
    if failing:
        header = f"RATCHET FAIL: {e['name']}" if enforce else f"RATCHET FAIL [dry-run, would FAIL]: {e['name']}"
    else:
        header = f"RATCHET PASS: {e['name']}"
    print(header)
    for line in body:
        print(line)
    if failing:
        for line in FIX_COMMON:
            print(line)
        if e["is_global"]:
            print("      tests/integration/coverage-floor.toml's [global] section IN THIS PR —")
            print("      bump (or lower) captured_total_lines and floor_percent to the new")
            print("      observed numbers, set captured_date, and add a one-line rationale.")
        else:
            print("      tests/integration/coverage-floor.toml's [[crate]] entry for")
            print(f"      {e['name']} IN THIS PR — bump (or lower) captured_total_lines and")
            print("      floor_percent/floor_covered_lines to the new observed numbers, set")
            print("      captured_date, and add a one-line rationale + issue link.")
        print("      See the file's own header for the schema.")
    print()

if enforce and any_violation:
    sys.exit(1)
sys.exit(0)
PY
