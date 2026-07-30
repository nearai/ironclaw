#!/usr/bin/env python3
"""Enforce coverage for instrumentable production lines changed by a PR."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections.abc import Iterable
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

HUNK_RE = re.compile(r"^@@ .* \+(?P<start>\d+)(?:,(?P<count>\d+))? @@")
PRODUCT_SOURCE_RE = re.compile(r"^crates/[^/]+/src/.+\.rs$")


def _normalize_source_path(path: str) -> str:
    normalized = path.replace("\\", "/")
    marker = "crates/"
    marker_index = normalized.find(marker)
    return normalized[marker_index:] if marker_index >= 0 else normalized


def _changed_lines(diff_text: str) -> dict[str, set[int]]:
    changed: dict[str, set[int]] = {}
    current_path: str | None = None
    for line in diff_text.splitlines():
        if line.startswith("+++ "):
            value = line[4:]
            current_path = None if value == "/dev/null" else value.removeprefix("b/")
            continue
        match = HUNK_RE.match(line)
        if current_path is None or match is None:
            continue
        start = int(match.group("start"))
        count = int(match.group("count") or "1")
        changed.setdefault(current_path, set()).update(range(start, start + count))
    return changed


def _lcov_lines(lcov_text: str) -> dict[str, dict[int, int]]:
    files: dict[str, dict[int, int]] = {}
    current: dict[int, int] | None = None
    for line in lcov_text.splitlines():
        if line.startswith("SF:"):
            path = _normalize_source_path(line[3:])
            current = files.setdefault(path, {})
        elif line.startswith("DA:") and current is not None:
            fields = line[3:].split(",", 2)
            line_number = int(fields[0])
            hits = int(fields[1])
            current[line_number] = max(current.get(line_number, 0), hits)
        elif line == "end_of_record":
            current = None
    return files


def _is_exempt(
    path: str,
    exempt_modules: set[str],
    exempt_crates: set[str],
) -> bool:
    if path in exempt_modules:
        return True
    parts = path.split("/")
    return len(parts) > 1 and parts[1] in exempt_crates


def evaluate(
    diff_text: str,
    lcov_text: str,
    threshold: float,
    *,
    exempt_modules: Iterable[str] = (),
    exempt_crates: Iterable[str] = (),
) -> dict[str, object]:
    """Return a deterministic changed-line coverage decision."""
    if not diff_text.strip():
        raise ValueError("git diff input is empty")
    if not 0.0 <= threshold <= 100.0:
        raise ValueError("coverage threshold must be between 0 and 100")

    module_exemptions = set(exempt_modules)
    crate_exemptions = set(exempt_crates)
    changed = {
        path: lines
        for path, lines in _changed_lines(diff_text).items()
        if PRODUCT_SOURCE_RE.fullmatch(path)
        and not _is_exempt(path, module_exemptions, crate_exemptions)
    }
    coverage = _lcov_lines(lcov_text)
    missing_files = sorted(path for path in changed if path not in coverage)
    instrumented: list[tuple[str, int, int]] = []
    for path, lines in sorted(changed.items()):
        for line_number in sorted(lines):
            if line_number in coverage.get(path, {}):
                instrumented.append((path, line_number, coverage[path][line_number]))

    covered = sum(hits > 0 for _, _, hits in instrumented)
    total = len(instrumented)
    percent = round(covered / total * 100.0, 2) if total else 100.0
    uncovered = [
        f"{path}:{line_number}" for path, line_number, hits in instrumented if hits == 0
    ]
    passed = not missing_files and percent >= threshold
    return {
        "schema_version": 1,
        "threshold_percent": threshold,
        "coverage_percent": percent,
        "covered_lines": covered,
        "instrumented_lines": total,
        "changed_product_files": sorted(changed),
        "missing_files": missing_files,
        "uncovered": uncovered,
        "passed": passed,
    }


def _git_diff(base: str, head: str) -> str:
    return subprocess.run(
        [
            "git",
            "diff",
            "--unified=0",
            "--no-ext-diff",
            "--no-renames",
            "--diff-filter=AM",
            f"{base}...{head}",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def _print_report(report: dict[str, object]) -> None:
    print("### Changed production-line coverage")
    print()
    print(
        f"- Result: {'PASS' if report['passed'] else 'FAIL'}\n"
        f"- Coverage: {report['coverage_percent']}% "
        f"({report['covered_lines']} / {report['instrumented_lines']})\n"
        f"- Required: {report['threshold_percent']}%"
    )
    missing_files = list(report["missing_files"])
    if missing_files:
        print("- Changed production files absent from LCOV:")
        for path in missing_files:
            print(f"  - `{path}`")
    uncovered = list(report["uncovered"])
    if uncovered:
        print("- Uncovered changed instrumentable lines:")
        for location in uncovered:
            print(f"  - `{location}`")


def main() -> int:
    sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
    from reborn_coverage_lcov import load_exemptions

    parser = argparse.ArgumentParser()
    parser.add_argument("--lcov", type=Path, required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--threshold", type=float, default=90.0)
    parser.add_argument(
        "--exemptions",
        type=Path,
        default=ROOT / "tests/integration/coverage-exemptions.toml",
    )
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()

    exempt_modules, exempt_crates, _ = load_exemptions(args.exemptions)
    report = evaluate(
        _git_diff(args.base, args.head),
        args.lcov.read_text(encoding="utf-8"),
        args.threshold,
        exempt_modules=exempt_modules,
        exempt_crates=exempt_crates,
    )
    if args.json is not None:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    _print_report(report)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
