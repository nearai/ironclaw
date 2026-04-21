#!/usr/bin/env python3
"""Compare Engine V2 Milestone 0 behavior against a baseline staging ref.

The comparison runs the shared replay suite in report mode on:
  1. the current checkout (expected: firat/engine-v2-m0)
  2. a temporary baseline worktree (default: origin/staging)

The baseline worktree is overlaid with the current M0 replay suite + fixtures so
both runs execute the exact same evaluation harness while exercising different
runtime code.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
M0_TEST = ROOT / "tests/e2e_engine_v2_milestone0.rs"
M0_FIXTURE_DIR = ROOT / "tests/fixtures/llm_traces/engine_v2"
REPORT_ENV = "IRONCLAW_M0_REPORT_JSONL"
DEFAULT_TIMEOUT_SECONDS = 1200


def run(cmd: list[str], *, cwd: Path, env: dict[str, str] | None = None, capture: bool = False) -> subprocess.CompletedProcess[str]:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    return subprocess.run(
        cmd,
        cwd=cwd,
        env=merged_env,
        text=True,
        check=False,
        capture_output=capture,
    )


def must_run(cmd: list[str], *, cwd: Path, env: dict[str, str] | None = None, capture: bool = False) -> subprocess.CompletedProcess[str]:
    result = run(cmd, cwd=cwd, env=env, capture=capture)
    if result.returncode != 0:
        if capture:
            sys.stderr.write(result.stdout)
            sys.stderr.write(result.stderr)
        raise SystemExit(f"command failed ({result.returncode}): {' '.join(cmd)}")
    return result


def copy_suite_into(worktree: Path) -> None:
    target_test = worktree / "tests/e2e_engine_v2_milestone0.rs"
    target_test.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(M0_TEST, target_test)

    target_fixture_dir = worktree / "tests/fixtures/llm_traces/engine_v2"
    target_fixture_dir.mkdir(parents=True, exist_ok=True)
    for fixture in M0_FIXTURE_DIR.glob("*.json"):
        shutil.copy2(fixture, target_fixture_dir / fixture.name)


def load_reports(path: Path) -> dict[str, Any]:
    reports: list[dict[str, Any]] = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line:
            reports.append(json.loads(line))
    return {report["scenario_id"]: report for report in reports}


def format_bool(value: bool) -> str:
    return "yes" if value else "no"


def format_missing(report: dict[str, Any]) -> str:
    check = report["expectation_check"]
    missing = check.get("missing_request_substrings", [])
    unexpected = check.get("unexpected_request_substrings", [])
    parts: list[str] = []
    if missing:
        parts.append("missing: " + "; ".join(missing))
    if unexpected:
        parts.append("unexpected: " + "; ".join(unexpected))
    return " | ".join(parts) if parts else "—"


def scenario_rows(baseline: dict[str, Any], current: dict[str, Any]) -> list[dict[str, Any]]:
    scenario_ids = sorted(set(baseline) | set(current))
    rows: list[dict[str, Any]] = []
    for scenario_id in scenario_ids:
        base = baseline.get(scenario_id)
        cur = current.get(scenario_id)
        row = {
            "scenario_id": scenario_id,
            "baseline_passed": bool(base and base["passed"]),
            "current_passed": bool(cur and cur["passed"]),
            "baseline_llm_calls": base["outcome"]["llm_calls"] if base else None,
            "current_llm_calls": cur["outcome"]["llm_calls"] if cur else None,
            "baseline_tool_calls": base["outcome"]["tool_calls_total"] if base else None,
            "current_tool_calls": cur["outcome"]["tool_calls_total"] if cur else None,
            "baseline_duplicates": base["outcome"]["duplicate_tool_calls"] if base else None,
            "current_duplicates": cur["outcome"]["duplicate_tool_calls"] if cur else None,
            "baseline_missing": format_missing(base) if base else "missing report",
            "current_missing": format_missing(cur) if cur else "missing report",
            "baseline_markers": base["outcome"] if base else None,
            "current_markers": cur["outcome"] if cur else None,
        }
        rows.append(row)
    return rows


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    improved = [r["scenario_id"] for r in rows if not r["baseline_passed"] and r["current_passed"]]
    regressed = [r["scenario_id"] for r in rows if r["baseline_passed"] and not r["current_passed"]]
    same_fail = [r["scenario_id"] for r in rows if not r["baseline_passed"] and not r["current_passed"]]
    same_pass = [r["scenario_id"] for r in rows if r["baseline_passed"] and r["current_passed"]]
    return {
        "scenario_count": len(rows),
        "baseline_passed": sum(1 for r in rows if r["baseline_passed"]),
        "current_passed": sum(1 for r in rows if r["current_passed"]),
        "improved": improved,
        "regressed": regressed,
        "same_fail": same_fail,
        "same_pass": same_pass,
    }


def render_markdown(summary: dict[str, Any], rows: list[dict[str, Any]], baseline_ref: str, current_ref: str) -> str:
    lines: list[str] = []
    lines.append("# Engine V2 Milestone 0 comparison")
    lines.append("")
    lines.append(f"- Baseline ref: `{baseline_ref}`")
    lines.append(f"- Current ref: `{current_ref}`")
    lines.append(f"- Scenarios: **{summary['scenario_count']}**")
    lines.append(f"- Baseline expectation pass count: **{summary['baseline_passed']}/{summary['scenario_count']}**")
    lines.append(f"- Current expectation pass count: **{summary['current_passed']}/{summary['scenario_count']}**")
    lines.append("")

    if summary["improved"]:
        lines.append("## Improved on current branch")
        for scenario_id in summary["improved"]:
            lines.append(f"- `{scenario_id}`")
        lines.append("")

    if summary["regressed"]:
        lines.append("## Regressed on current branch")
        for scenario_id in summary["regressed"]:
            lines.append(f"- `{scenario_id}`")
        lines.append("")

    if summary["same_fail"]:
        lines.append("## Still failing expectations on both refs")
        for scenario_id in summary["same_fail"]:
            lines.append(f"- `{scenario_id}`")
        lines.append("")

    lines.append("## Scenario table")
    lines.append("")
    lines.append("| Scenario | Baseline pass | Current pass | Baseline missing markers | Current missing markers | LLM calls (base→current) | Tool calls (base→current) | Duplicate tools (base→current) |")
    lines.append("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for row in rows:
        lines.append(
            "| {scenario} | {bpass} | {cpass} | {bmiss} | {cmiss} | {bllm}→{cllm} | {btools}→{ctools} | {bdup}→{cdup} |".format(
                scenario=row["scenario_id"],
                bpass=format_bool(row["baseline_passed"]),
                cpass=format_bool(row["current_passed"]),
                bmiss=row["baseline_missing"].replace("|", "\\|"),
                cmiss=row["current_missing"].replace("|", "\\|"),
                bllm=row["baseline_llm_calls"],
                cllm=row["current_llm_calls"],
                btools=row["baseline_tool_calls"],
                ctools=row["current_tool_calls"],
                bdup=row["baseline_duplicates"],
                cdup=row["current_duplicates"],
            )
        )
    lines.append("")
    return "\n".join(lines)


def branch_label(cwd: Path) -> str:
    result = must_run(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=cwd, capture=True)
    return result.stdout.strip()


def commit_label(cwd: Path) -> str:
    result = must_run(["git", "rev-parse", "--short", "HEAD"], cwd=cwd, capture=True)
    return result.stdout.strip()


def run_suite(worktree: Path, report_path: Path, timeout_seconds: int) -> None:
    if report_path.exists():
        report_path.unlink()
    env = {REPORT_ENV: str(report_path), "CARGO_TERM_COLOR": "always"}
    cmd = [
        "cargo",
        "test",
        "--test",
        "e2e_engine_v2_milestone0",
        "--features",
        "libsql",
        "--",
        "--nocapture",
        "--test-threads=1",
    ]
    result = run(cmd, cwd=worktree, env=env, capture=True)
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    if result.returncode != 0:
        raise SystemExit(
            f"suite failed in {worktree} with exit code {result.returncode}; partial report at {report_path}"
        )
    if not report_path.exists():
        raise SystemExit(f"suite completed but did not write report file: {report_path}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline-ref", default="origin/staging")
    parser.add_argument(
        "--output-dir",
        default=str(ROOT / "target/engine_v2_m0_compare"),
        help="Directory for markdown/json outputs",
    )
    parser.add_argument("--keep-baseline-worktree", action="store_true")
    parser.add_argument("--timeout-seconds", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    args = parser.parse_args()

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    timestamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")

    current_report_path = output_dir / f"current-{timestamp}.jsonl"
    baseline_report_path = output_dir / f"baseline-{timestamp}.jsonl"
    markdown_path = output_dir / f"compare-{timestamp}.md"
    json_path = output_dir / f"compare-{timestamp}.json"

    baseline_dir = Path(tempfile.mkdtemp(prefix="ironclaw-m0-baseline-"))
    try:
        must_run(["git", "worktree", "add", "--detach", str(baseline_dir), args.baseline_ref], cwd=ROOT)
        copy_suite_into(baseline_dir)

        current_ref = f"{branch_label(ROOT)}@{commit_label(ROOT)}"
        baseline_ref = f"{args.baseline_ref}@{commit_label(baseline_dir)}"

        print(f"==> Running current suite in {ROOT}")
        run_suite(ROOT, current_report_path, args.timeout_seconds)
        print(f"==> Running baseline suite in {baseline_dir}")
        run_suite(baseline_dir, baseline_report_path, args.timeout_seconds)

        current_reports = load_reports(current_report_path)
        baseline_reports = load_reports(baseline_report_path)
        rows = scenario_rows(baseline_reports, current_reports)
        summary = summarize(rows)

        markdown = render_markdown(summary, rows, baseline_ref, current_ref)
        markdown_path.write_text(markdown)
        json_path.write_text(
            json.dumps(
                {
                    "summary": summary,
                    "rows": rows,
                    "baseline_ref": baseline_ref,
                    "current_ref": current_ref,
                    "baseline_report_path": str(baseline_report_path),
                    "current_report_path": str(current_report_path),
                },
                indent=2,
            )
        )

        print(f"Markdown report: {markdown_path}")
        print(f"JSON report: {json_path}")
        print(markdown)
    finally:
        if args.keep_baseline_worktree:
            print(f"Kept baseline worktree: {baseline_dir}")
        else:
            run(["git", "worktree", "remove", "--force", str(baseline_dir)], cwd=ROOT)
            shutil.rmtree(baseline_dir, ignore_errors=True)


if __name__ == "__main__":
    main()
