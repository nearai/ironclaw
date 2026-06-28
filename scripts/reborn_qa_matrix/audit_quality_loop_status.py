#!/usr/bin/env python3
"""Summarize the scoped Reborn WebUIv2/ResponsesAPI QA loop state."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import audit_defect_traceability
import audit_execution_evidence
import audit_surface_inventory
import audit_workbook_completeness
import report_coverage

ROOT = Path(__file__).resolve().parents[2]


def _count_blocking_gaps(
    surface: dict[str, object],
    completeness: dict[str, object],
    coverage: dict[str, object],
    execution: dict[str, object],
    defects: dict[str, object],
    *,
    strict_no_blocked: bool,
) -> int:
    gap_count = int(surface["uncovered_surface_count"])
    gap_count += int(completeness["missing_feature_field_count"])
    gap_count += int(completeness["missing_test_suite_count"])
    gap_count += int(completeness["missing_test_category_count"])
    gap_count += len(coverage["runner_ids_not_in_workbook"])
    gap_count += int(coverage["actionable_gap_test_count"])
    gap_count += int(execution["missing_execution_field_count"])
    gap_count += int(execution["unknown_status_test_count"])
    gap_count += int(execution["missing_external_reference_count"])
    gap_count += int(execution["stale_external_reference_count"])
    if strict_no_blocked:
        gap_count += int(execution["blocked_test_count"])
    gap_count += int(defects["undocumented_non_passing_test_count"])
    gap_count += int(defects["missing_defect_field_count"])
    gap_count += int(defects["open_defect_count"])
    return gap_count


def build_status(
    workbook_path: Path,
    repo_root: Path = ROOT,
    *,
    strict_no_blocked: bool = False,
) -> dict[str, object]:
    surface = audit_surface_inventory.build_audit(workbook_path, repo_root)
    completeness = audit_workbook_completeness.build_audit(workbook_path)
    coverage = report_coverage.build_report(workbook_path, repo_root)
    execution = audit_execution_evidence.build_audit(workbook_path)
    defects = audit_defect_traceability.build_audit(workbook_path)
    blocking_gap_count = _count_blocking_gaps(
        surface,
        completeness,
        coverage,
        execution,
        defects,
        strict_no_blocked=strict_no_blocked,
    )
    blocked_tests = [
        {
            "test_id": row["test_id"],
            "feature_id": row["feature_id"],
            "status": row["status"],
            "category": row["category"],
        }
        for row in execution["blocked_tests"]
    ]
    defects_found = (
        int(defects["scoped_defect_count"])
        + int(surface["uncovered_surface_count"])
        + int(completeness["missing_feature_field_count"])
        + int(completeness["missing_test_suite_count"])
        + int(completeness["missing_test_category_count"])
        + int(coverage["actionable_gap_test_count"])
        + int(execution["missing_execution_field_count"])
        + int(execution["unknown_status_test_count"])
        + int(execution["missing_external_reference_count"])
        + int(execution["stale_external_reference_count"])
    )
    traceable_runner_coverage_pct = coverage.get(
        "traceable_runner_coverage_pct",
        coverage["combined_runner_coverage_pct"],
    )
    traceable_feature_count = coverage.get(
        "traceable_feature_count",
        coverage["covered_feature_count"],
    )
    return {
        "workbook": str(workbook_path),
        "strict_no_blocked": strict_no_blocked,
        "blocking_gap_count": blocking_gap_count,
        "loop_gate_passed": blocking_gap_count == 0,
        "coverage_summary": {
            "scoped_features": coverage["feature_count"],
            "total_features": coverage["all_feature_count"],
            "scoped_tests": coverage["matrix_test_count"],
            "total_tests": coverage["all_matrix_test_count"],
            "hermetic_runner_coverage_pct": coverage["hermetic_runner_coverage_pct"],
            "combined_runner_coverage_pct": coverage["combined_runner_coverage_pct"],
            "traceable_runner_coverage_pct": traceable_runner_coverage_pct,
            "matrix_only_or_new_combined_coverage_pct": coverage[
                "matrix_only_or_new_combined_coverage_pct"
            ],
            "actionable_gap_test_count": coverage["actionable_gap_test_count"],
        },
        "features_tested": {
            "executable_feature_count": coverage["covered_feature_count"],
            "traceable_feature_count": traceable_feature_count,
            "scoped_feature_count": coverage["feature_count"],
            "matrix_only_or_new_feature_count": coverage[
                "matrix_only_or_new_feature_count"
            ],
        },
        "defects_found": defects_found,
        "defects_fixed": defects["resolved_defect_count"],
        "defects_documented_or_waived": defects["waived_defect_count"],
        "remaining_risks": {
            "blocked_test_count": execution["blocked_test_count"],
            "blocked_tests": blocked_tests,
            "undocumented_non_passing_test_count": defects[
                "undocumented_non_passing_test_count"
            ],
            "open_defect_count": defects["open_defect_count"],
            "open_high_critical_defect_count": defects[
                "open_high_critical_defect_count"
            ],
            "uncovered_surface_count": surface["uncovered_surface_count"],
            "missing_test_suite_count": completeness["missing_test_suite_count"],
            "unknown_status_test_count": execution["unknown_status_test_count"],
            "missing_external_reference_count": execution[
                "missing_external_reference_count"
            ],
            "stale_external_reference_count": execution[
                "stale_external_reference_count"
            ],
        },
        "confidence_score": f"{traceable_runner_coverage_pct}%",
        "surface": surface,
        "completeness": completeness,
        "coverage": coverage,
        "execution": execution,
        "defects": defects,
    }


def print_report(report: dict[str, object]) -> None:
    coverage = report["coverage_summary"]
    features = report["features_tested"]
    risks = report["remaining_risks"]
    print(f"Workbook: {report['workbook']}")
    print(f"Loop gate passed: {report['loop_gate_passed']}")
    print(f"Blocking gaps: {report['blocking_gap_count']}")
    print(
        "Coverage Summary: "
        f"{coverage['scoped_features']} scoped features, "
        f"{coverage['scoped_tests']} scoped tests, "
        f"{coverage['traceable_runner_coverage_pct']}% traceable coverage, "
        f"{coverage['combined_runner_coverage_pct']}% executable hermetic+live coverage, "
        f"{coverage['actionable_gap_test_count']} actionable gaps"
    )
    print(
        "Features Tested: "
        f"{features['traceable_feature_count']} / {features['scoped_feature_count']} "
        "scoped features have runner/live/existing-CI traceability "
        f"({features['executable_feature_count']} executable)"
    )
    print(f"Defects Found: {report['defects_found']}")
    print(f"Defects Fixed: {report['defects_fixed']}")
    print(f"Defects Documented/Waived: {report['defects_documented_or_waived']}")
    print(
        "Remaining Risks: "
        f"{risks['blocked_test_count']} blocked tests, "
        f"{risks['undocumented_non_passing_test_count']} undocumented non-passing rows, "
        f"{risks['missing_external_reference_count']} missing external references, "
        f"{risks['stale_external_reference_count']} stale external references, "
        f"{risks['open_defect_count']} open defects, "
        f"{risks['open_high_critical_defect_count']} open high/critical defects"
    )
    for blocked in risks["blocked_tests"]:
        print(f"- blocked {blocked['test_id']} {blocked['status']}")
    print(f"Confidence Score: {report['confidence_score']}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workbook", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    parser.add_argument(
        "--strict-no-blocked",
        action="store_true",
        help="treat documented blocked live rows as blocking gaps",
    )
    parser.add_argument("--json", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    report = build_status(
        args.workbook,
        args.repo_root,
        strict_no_blocked=args.strict_no_blocked,
    )
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_report(report)
    return 0 if report["loop_gate_passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
