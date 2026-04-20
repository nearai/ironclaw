#!/usr/bin/env python3
"""Compare Engine V2 Milestone 0 black-box behavior against base staging.

This runs the outcome-first black-box replay suite on:
  1. the current checkout (expected: firat/engine-v2-m0)
  2. a temporary detached worktree at the chosen baseline ref

The baseline worktree is overlaid with the current black-box suite + fixtures
so both refs execute the same evaluation harness while different runtime code
produces the observable outcomes.
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
BLACKBOX_TEST = ROOT / "tests/e2e_engine_v2_milestone0_blackbox.rs"
BLACKBOX_FIXTURE_DIR = ROOT / "tests/fixtures/llm_traces/engine_v2_blackbox"
REPORT_ENV = "IRONCLAW_M0_BLACKBOX_REPORT_JSONL"
TEST_TARGET = "e2e_engine_v2_milestone0_blackbox"


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
    target_test = worktree / "tests/e2e_engine_v2_milestone0_blackbox.rs"
    target_test.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(BLACKBOX_TEST, target_test)

    target_fixture_dir = worktree / "tests/fixtures/llm_traces/engine_v2_blackbox"
    target_fixture_dir.mkdir(parents=True, exist_ok=True)
    for fixture in BLACKBOX_FIXTURE_DIR.glob("*.json"):
        shutil.copy2(fixture, target_fixture_dir / fixture.name)


def load_reports(path: Path) -> dict[str, Any]:
    reports: list[dict[str, Any]] = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line:
            reports.append(json.loads(line))
    return {report["scenario_id"]: report for report in reports}


def branch_label(cwd: Path) -> str:
    result = must_run(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=cwd, capture=True)
    return result.stdout.strip()


def commit_label(cwd: Path) -> str:
    result = must_run(["git", "rev-parse", "--short", "HEAD"], cwd=cwd, capture=True)
    return result.stdout.strip()


def run_suite(worktree: Path, report_path: Path) -> None:
    if report_path.exists():
        report_path.unlink()
    env = {REPORT_ENV: str(report_path), "CARGO_TERM_COLOR": "always"}
    cmd = [
        "cargo",
        "test",
        "--test",
        TEST_TARGET,
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


def summarize(baseline: dict[str, Any], current: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
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
            "baseline_duplicate_tool_calls": base["outcome"]["duplicate_tool_calls"] if base else None,
            "current_duplicate_tool_calls": cur["outcome"]["duplicate_tool_calls"] if cur else None,
            "baseline_failed_tool_streak": base["outcome"]["max_failed_tool_streak"] if base else None,
            "current_failed_tool_streak": cur["outcome"]["max_failed_tool_streak"] if cur else None,
            "baseline_final_state": base["outcome"]["final_state"] if base else None,
            "current_final_state": cur["outcome"]["final_state"] if cur else None,
            "baseline_failures": base["check"] if base else None,
            "current_failures": cur["check"] if cur else None,
            "baseline_response_preview": base.get("final_response_preview") if base else None,
            "current_response_preview": cur.get("final_response_preview") if cur else None,
        }
        rows.append(row)

    summary = {
        "scenario_count": len(rows),
        "baseline_passed": sum(1 for row in rows if row["baseline_passed"]),
        "current_passed": sum(1 for row in rows if row["current_passed"]),
        "improved": [row["scenario_id"] for row in rows if not row["baseline_passed"] and row["current_passed"]],
        "regressed": [row["scenario_id"] for row in rows if row["baseline_passed"] and not row["current_passed"]],
        "same_fail": [row["scenario_id"] for row in rows if not row["baseline_passed"] and not row["current_passed"]],
        "same_pass": [row["scenario_id"] for row in rows if row["baseline_passed"] and row["current_passed"]],
        "baseline_total_llm_calls": sum(row["baseline_llm_calls"] or 0 for row in rows),
        "current_total_llm_calls": sum(row["current_llm_calls"] or 0 for row in rows),
        "baseline_total_tool_calls": sum(row["baseline_tool_calls"] or 0 for row in rows),
        "current_total_tool_calls": sum(row["current_tool_calls"] or 0 for row in rows),
        "baseline_total_duplicate_tool_calls": sum(row["baseline_duplicate_tool_calls"] or 0 for row in rows),
        "current_total_duplicate_tool_calls": sum(row["current_duplicate_tool_calls"] or 0 for row in rows),
        "baseline_total_failed_tool_streak": sum(row["baseline_failed_tool_streak"] or 0 for row in rows),
        "current_total_failed_tool_streak": sum(row["current_failed_tool_streak"] or 0 for row in rows),
    }
    return summary, rows


def render_failures(check: dict[str, Any] | None) -> str:
    if not check:
        return "missing report"
    parts: list[str] = []
    if check.get("missing_response_substrings"):
        parts.append("missing response: " + "; ".join(check["missing_response_substrings"]))
    if check.get("unexpected_response_substrings"):
        parts.append("unexpected response: " + "; ".join(check["unexpected_response_substrings"]))
    if check.get("final_response_missing"):
        parts.append("no final response")
    if check.get("llm_calls_exceeded_by") is not None:
        parts.append(f"llm_calls +{check['llm_calls_exceeded_by']}")
    if check.get("tool_calls_exceeded_by") is not None:
        parts.append(f"tool_calls +{check['tool_calls_exceeded_by']}")
    if check.get("duplicate_tool_calls_exceeded_by") is not None:
        parts.append(f"duplicate_tools +{check['duplicate_tool_calls_exceeded_by']}")
    if check.get("failed_tool_streak_exceeded_by") is not None:
        parts.append(f"failed_streak +{check['failed_tool_streak_exceeded_by']}")
    if check.get("unexpected_final_state"):
        parts.append(f"final_state={check['unexpected_final_state']}")
    return " | ".join(parts) if parts else "—"


def render_markdown(summary: dict[str, Any], rows: list[dict[str, Any]], baseline_ref: str, current_ref: str) -> str:
    lines: list[str] = []
    lines.append("# Engine V2 Milestone 0 black-box comparison")
    lines.append("")
    lines.append(f"- Baseline ref: `{baseline_ref}`")
    lines.append(f"- Current ref: `{current_ref}`")
    lines.append(f"- Scenarios: **{summary['scenario_count']}**")
    lines.append(f"- Baseline pass count: **{summary['baseline_passed']}/{summary['scenario_count']}**")
    lines.append(f"- Current pass count: **{summary['current_passed']}/{summary['scenario_count']}**")
    lines.append(f"- Total LLM calls: **{summary['baseline_total_llm_calls']} → {summary['current_total_llm_calls']}**")
    lines.append(f"- Total tool calls: **{summary['baseline_total_tool_calls']} → {summary['current_total_tool_calls']}**")
    lines.append(f"- Total duplicate tool calls: **{summary['baseline_total_duplicate_tool_calls']} → {summary['current_total_duplicate_tool_calls']}**")
    lines.append(f"- Total failed-tool streak sum: **{summary['baseline_total_failed_tool_streak']} → {summary['current_total_failed_tool_streak']}**")
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

    if summary["same_pass"]:
        lines.append("## Stable passes on both refs")
        for scenario_id in summary["same_pass"]:
            lines.append(f"- `{scenario_id}`")
        lines.append("")

    if summary["same_fail"]:
        lines.append("## Still failing on both refs")
        for scenario_id in summary["same_fail"]:
            lines.append(f"- `{scenario_id}`")
        lines.append("")

    lines.append("## Scenario table")
    lines.append("")
    lines.append("| Scenario | Baseline pass | Current pass | LLM calls (base→current) | Tool calls (base→current) | Duplicate tools (base→current) | Failed streak (base→current) | Baseline failures | Current failures |")
    lines.append("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    for row in rows:
        lines.append(
            "| {scenario} | {bpass} | {cpass} | {bllm}→{cllm} | {btools}→{ctools} | {bdup}→{cdup} | {bfail}→{cfail} | {bissues} | {cissues} |".format(
                scenario=row["scenario_id"],
                bpass="yes" if row["baseline_passed"] else "no",
                cpass="yes" if row["current_passed"] else "no",
                bllm=row["baseline_llm_calls"],
                cllm=row["current_llm_calls"],
                btools=row["baseline_tool_calls"],
                ctools=row["current_tool_calls"],
                bdup=row["baseline_duplicate_tool_calls"],
                cdup=row["current_duplicate_tool_calls"],
                bfail=row["baseline_failed_tool_streak"],
                cfail=row["current_failed_tool_streak"],
                bissues=render_failures(row["baseline_failures"]).replace("|", "\\|"),
                cissues=render_failures(row["current_failures"]).replace("|", "\\|"),
            )
        )
    lines.append("")
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline-ref", default="origin/staging")
    parser.add_argument(
        "--output-dir",
        default=str(ROOT / "target/engine_v2_m0_blackbox_compare"),
        help="Directory for markdown/json outputs",
    )
    parser.add_argument("--keep-baseline-worktree", action="store_true")
    args = parser.parse_args()

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    timestamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")

    current_report_path = output_dir / f"current-{timestamp}.jsonl"
    baseline_report_path = output_dir / f"baseline-{timestamp}.jsonl"
    markdown_path = output_dir / f"compare-{timestamp}.md"
    json_path = output_dir / f"compare-{timestamp}.json"

    baseline_dir = Path(tempfile.mkdtemp(prefix="ironclaw-m0-blackbox-baseline-"))
    try:
        must_run(["git", "worktree", "add", "--detach", str(baseline_dir), args.baseline_ref], cwd=ROOT)
        copy_suite_into(baseline_dir)

        current_ref = f"{branch_label(ROOT)}@{commit_label(ROOT)}"
        baseline_ref = f"{args.baseline_ref}@{commit_label(baseline_dir)}"

        print(f"==> Running current black-box suite in {ROOT}")
        run_suite(ROOT, current_report_path)
        print(f"==> Running baseline black-box suite in {baseline_dir}")
        run_suite(baseline_dir, baseline_report_path)

        current_reports = load_reports(current_report_path)
        baseline_reports = load_reports(baseline_report_path)
        summary, rows = summarize(baseline_reports, current_reports)

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
