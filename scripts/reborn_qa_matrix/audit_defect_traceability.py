#!/usr/bin/env python3
"""Audit scoped non-passing QA rows for defect or waiver traceability."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import audit_execution_evidence
import audit_workbook_completeness

REQUIRED_DEFECT_COLUMNS = (
    "Defect ID",
    "Feature ID",
    "Title",
    "Reproduction Steps",
    "Expected Result",
    "Actual Result",
    "Severity",
    "Root Cause Hypothesis",
    "Status",
    "Last Updated",
)

HIGH_SEVERITIES = {"critical", "high"}
CLOSED_STATUS_PREFIXES = (
    "resolved",
    "waived",
    "external-existing",
    "duplicate",
)


def _row_text(row: dict[str, str]) -> str:
    return " ".join(str(value or "") for value in row.values())


def _is_closed_or_waived(status: str) -> bool:
    normalized = status.strip().lower()
    return normalized.startswith(CLOSED_STATUS_PREFIXES)


def build_audit(
    workbook_path: Path,
    *,
    scope_tokens: tuple[str, ...] = audit_workbook_completeness.DEFAULT_SCOPE_TOKENS,
) -> dict[str, object]:
    features = audit_workbook_completeness._records(workbook_path, "Feature Inventory")
    tests = audit_workbook_completeness._records(workbook_path, "Test Cases")
    defects = audit_workbook_completeness._records(workbook_path, "Defects")
    scoped_ids = {
        feature["Feature ID"]
        for feature in features
        if audit_workbook_completeness._in_scope(feature, scope_tokens)
    }
    scoped_tests = [test for test in tests if test.get("Feature ID", "") in scoped_ids]
    non_passing_tests = [
        test
        for test in scoped_tests
        if audit_execution_evidence._status_class(test.get("Status", ""))
        not in {"passed", "external_existing"}
    ]

    missing_defect_fields: list[dict[str, str]] = []
    for defect in defects:
        for column in REQUIRED_DEFECT_COLUMNS:
            if not (defect.get(column) or "").strip():
                missing_defect_fields.append(
                    {
                        "defect_id": defect.get("Defect ID", ""),
                        "feature_id": defect.get("Feature ID", ""),
                        "column": column,
                    }
                )

    defect_rows_by_test_id: dict[str, list[dict[str, str]]] = {}
    for test in non_passing_tests:
        test_id = test.get("Test ID", "")
        feature_id = test.get("Feature ID", "")
        defect_rows_by_test_id[test_id] = [
            defect
            for defect in defects
            if defect.get("Feature ID", "") == feature_id and test_id in _row_text(defect)
        ]

    undocumented_non_passing_tests = [
        {
            "test_id": test.get("Test ID", ""),
            "feature_id": test.get("Feature ID", ""),
            "status": test.get("Status", ""),
            "category": test.get("Category", ""),
        }
        for test in non_passing_tests
        if not defect_rows_by_test_id[test.get("Test ID", "")]
    ]

    open_high_critical_defects = [
        {
            "defect_id": defect.get("Defect ID", ""),
            "feature_id": defect.get("Feature ID", ""),
            "severity": defect.get("Severity", ""),
            "status": defect.get("Status", ""),
            "title": defect.get("Title", ""),
        }
        for defect in defects
        if defect.get("Feature ID", "") in scoped_ids
        and defect.get("Severity", "").strip().lower() in HIGH_SEVERITIES
        and not _is_closed_or_waived(defect.get("Status", ""))
    ]

    return {
        "workbook": str(workbook_path),
        "scope_tokens": list(scope_tokens),
        "scoped_non_passing_test_count": len(non_passing_tests),
        "documented_non_passing_test_count": sum(
            1
            for test in non_passing_tests
            if defect_rows_by_test_id[test.get("Test ID", "")]
        ),
        "undocumented_non_passing_test_count": len(undocumented_non_passing_tests),
        "missing_defect_field_count": len(missing_defect_fields),
        "open_high_critical_defect_count": len(open_high_critical_defects),
        "undocumented_non_passing_tests": undocumented_non_passing_tests,
        "missing_defect_fields": missing_defect_fields,
        "open_high_critical_defects": open_high_critical_defects,
    }


def _gap_count(report: dict[str, object], *, strict_no_open_high: bool) -> int:
    gap_count = int(report["undocumented_non_passing_test_count"]) + int(
        report["missing_defect_field_count"]
    )
    if strict_no_open_high:
        gap_count += int(report["open_high_critical_defect_count"])
    return gap_count


def print_report(report: dict[str, object]) -> None:
    print(f"Workbook: {report['workbook']}")
    print(f"Scoped non-passing tests: {report['scoped_non_passing_test_count']}")
    print(f"Documented non-passing tests: {report['documented_non_passing_test_count']}")
    print(f"Undocumented non-passing tests: {report['undocumented_non_passing_test_count']}")
    print(f"Missing defect fields: {report['missing_defect_field_count']}")
    print(f"Open high/critical defects: {report['open_high_critical_defect_count']}")
    for gap in report["undocumented_non_passing_tests"]:
        print(f"- undocumented {gap['test_id']} {gap['status']}")
    for gap in report["missing_defect_fields"]:
        print(f"- missing defect field {gap['defect_id']} {gap['column']}")
    for gap in report["open_high_critical_defects"]:
        print(f"- open high/critical {gap['defect_id']} {gap['severity']} {gap['status']}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workbook", type=Path, required=True)
    parser.add_argument(
        "--scope-token",
        action="append",
        dest="scope_tokens",
        help="case-insensitive feature row token; may be repeated",
    )
    parser.add_argument(
        "--strict-no-open-high",
        action="store_true",
        help="return nonzero when scoped high/critical defects are not closed or waived",
    )
    parser.add_argument("--json", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    scope_tokens = (
        tuple(args.scope_tokens)
        if args.scope_tokens
        else audit_workbook_completeness.DEFAULT_SCOPE_TOKENS
    )
    report = build_audit(args.workbook, scope_tokens=scope_tokens)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_report(report)
    return 0 if _gap_count(report, strict_no_open_high=args.strict_no_open_high) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
