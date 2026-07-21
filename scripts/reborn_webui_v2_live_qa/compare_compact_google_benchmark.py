#!/usr/bin/env python3
"""Compare compact-Google live-canary control and treatment artifacts."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any

ARMS = ("disabled", "enabled")
PROVIDER_MARKER = "qa-context-compact-"


def _arm_for_path(path: Path) -> str | None:
    text = str(path)
    matches = [arm for arm in ARMS if f"{PROVIDER_MARKER}{arm}" in text]
    return matches[0] if len(matches) == 1 else None


def load_results(artifacts_dir: Path) -> dict[str, dict[str, dict[str, Any]]]:
    arms: dict[str, dict[str, dict[str, Any]]] = {arm: {} for arm in ARMS}
    for path in artifacts_dir.rglob("results.json"):
        arm = _arm_for_path(path)
        if arm is None:
            continue
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if not isinstance(payload, dict):
            continue
        for result in payload.get("results") or []:
            if not isinstance(result, dict):
                continue
            details = result.get("details") or {}
            case = str(details.get("case") or "") if isinstance(details, dict) else ""
            if case.startswith("benchmark_google_"):
                arms[arm][case] = result
    return arms


def _integer(value: Any) -> int:
    if isinstance(value, bool):
        return 0
    if isinstance(value, int):
        return value
    return int(value) if isinstance(value, float) and math.isfinite(value) else 0


def _metric(result: dict[str, Any], key: str) -> int:
    details = result.get("details") or {}
    metrics = details.get("run_metrics") or {} if isinstance(details, dict) else {}
    value = metrics.get(key, 0) if isinstance(metrics, dict) else 0
    return _integer(value)


def _tokens(result: dict[str, Any], key: str) -> int:
    details = result.get("details") or {}
    metrics = details.get("run_metrics") or {} if isinstance(details, dict) else {}
    usage = metrics.get("usage") or {} if isinstance(metrics, dict) else {}
    value = usage.get(key, 0) if isinstance(usage, dict) else 0
    return _integer(value)


def build_report(
    arms: dict[str, dict[str, dict[str, Any]]],
) -> tuple[dict[str, Any], str]:
    cases = sorted(set(arms["disabled"]) | set(arms["enabled"]))
    rows: list[dict[str, Any]] = []
    for case in cases:
        control = arms["disabled"].get(case, {})
        treatment = arms["enabled"].get(case, {})
        control_calls = _metric(control, "google_tool_call_count")
        treatment_calls = _metric(treatment, "google_tool_call_count")
        saved = control_calls - treatment_calls
        rows.append(
            {
                "case": case,
                "disabled_success": control.get("success") is True,
                "enabled_success": treatment.get("success") is True,
                "disabled_google_calls": control_calls,
                "enabled_google_calls": treatment_calls,
                "google_calls_saved": saved,
                "compact_calls": _metric(treatment, "compact_google_tool_call_count"),
                "discovery_call_delta": _metric(
                    treatment, "discovery_tool_call_count"
                )
                - _metric(control, "discovery_tool_call_count"),
                "input_token_delta": _tokens(treatment, "input_tokens")
                - _tokens(control, "input_tokens"),
                "output_token_delta": _tokens(treatment, "output_tokens")
                - _tokens(control, "output_tokens"),
                "latency_delta_ms": _integer(treatment.get("latency_ms"))
                - _integer(control.get("latency_ms")),
            }
        )

    comparable = [
        row
        for row in rows
        if row["disabled_success"] and row["enabled_success"]
    ]
    control_total = sum(row["disabled_google_calls"] for row in comparable)
    treatment_total = sum(row["enabled_google_calls"] for row in comparable)
    saved_total = control_total - treatment_total
    saved_percent = (
        round(saved_total * 100.0 / control_total, 1) if control_total else None
    )
    verdict = "INCONCLUSIVE"
    if len(comparable) == len(rows) == 5:
        verdict = "VERIFIED" if saved_total > 0 else "NOT VERIFIED"
    report = {
        "verdict": verdict,
        "comparable_cases": len(comparable),
        "expected_cases": 5,
        "disabled_google_calls": control_total,
        "enabled_google_calls": treatment_total,
        "google_calls_saved": saved_total,
        "google_calls_saved_percent": saved_percent,
        "rows": rows,
    }

    lines = [
        "## Compact Google capability benchmark",
        "",
        f"**{verdict}**: comparable cases {len(comparable)}/5; "
        f"Google calls {control_total} -> {treatment_total}; "
        f"saved {saved_total} ({saved_percent if saved_percent is not None else '-'}%).",
        "",
        "| Case | Pass off/on | Google calls off/on | Saved | Compact used | Discovery delta | Input token delta | Latency delta |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for row in rows:
        lines.append(
            "| {case} | {off}/{on} | {off_calls}/{on_calls} | {saved:+d} | "
            "{compact} | {discovery:+d} | {tokens:+d} | {latency:+d} ms |".format(
                case=row["case"].removeprefix("benchmark_google_"),
                off="yes" if row["disabled_success"] else "no",
                on="yes" if row["enabled_success"] else "no",
                off_calls=row["disabled_google_calls"],
                on_calls=row["enabled_google_calls"],
                saved=row["google_calls_saved"],
                compact=row["compact_calls"],
                discovery=row["discovery_call_delta"],
                tokens=row["input_token_delta"],
                latency=row["latency_delta_ms"],
            )
        )
    return report, "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts-dir", type=Path, required=True)
    parser.add_argument("--json-out", type=Path, required=True)
    parser.add_argument("--markdown-out", type=Path, required=True)
    args = parser.parse_args()

    report, markdown = build_report(load_results(args.artifacts_dir))
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.markdown_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    args.markdown_out.write_text(markdown, encoding="utf-8")
    print(markdown, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
