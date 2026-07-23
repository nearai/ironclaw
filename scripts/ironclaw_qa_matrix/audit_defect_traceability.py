#!/usr/bin/env python3
"""Audit scoped non-passing QA rows for defect or waiver traceability."""

from __future__ import annotations

import argparse
import json
import re
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
RESOLVED_STATUS_PREFIXES = ("resolved",)
WAIVED_STATUS_PREFIXES = (
    "waived",
    "external-existing",
    "duplicate",
)
CLOSED_STATUS_PREFIXES = RESOLVED_STATUS_PREFIXES + WAIVED_STATUS_PREFIXES


def _normalized_status(row: dict[str, str]) -> str:
    return row.get("Status", "").strip().lower()


def _status_starts_with(row: dict[str, str], prefixes: tuple[str, ...]) -> bool:
    return _normalized_status(row).startswith(prefixes)


def _defect_summary(row: dict[str, str]) -> dict[str, str]:
    return {
        "defect_id": row.get("Defect ID", ""),
        "feature_id": row.get("Feature ID", ""),
        "severity": row.get("Severity", ""),
        "status": row.get("Status", ""),
        "title": row.get("Title", ""),
    }


def _is_resolved(row: dict[str, str]) -> bool:
    return _status_starts_with(row, RESOLVED_STATUS_PREFIXES)


def _is_waived(row: dict[str, str]) -> bool:
    return _status_starts_with(row, WAIVED_STATUS_PREFIXES)


def _is_closed_or_waived(row: dict[str, str]) -> bool:
    return _status_starts_with(row, CLOSED_STATUS_PREFIXES)


def _row_text(row: dict[str, str]) -> str:
    return " ".join(str(value or "") for value in row.values())


def _contains_test_id(row: dict[str, str], test_id: str) -> bool:
    pattern = re.compile(rf"(?<![A-Z0-9-]){re.escape(test_id)}(?![A-Z0-9-])")
    return bool(pattern.search(_row_text(row).upper()))


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
    scoped_defects = [
        defect for defect in defects if defect.get("Feature ID", "") in scoped_ids
    ]
    non_passing_tests = [
        test
        for test in scoped_tests
        if audit_execution_evidence._status_class(test.get("Status", ""))
        not in {"passed", "external_existing"}
    ]

    missing_defect_fields: list[dict[str, str]] = []
    for defect in scoped_defects:
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
            if defect.get("Feature ID", "") == feature_id
            and _contains_test_id(defect, test_id)
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

    resolved_defects = [defect for defect in scoped_defects if _is_resolved(defect)]
    waived_defects = [defect for defect in scoped_defects if _is_waived(defect)]
    open_defects = [
        _defect_summary(defect)
        for defect in scoped_defects
        if not _is_closed_or_waived(defect)
    ]

    open_high_critical_defects = [
        _defect_summary(defect)
        for defect in scoped_defects
        if defect.get("Severity", "").strip().lower() in HIGH_SEVERITIES
        and not _is_closed_or_waived(defect)
    ]

    return {
        "workbook": str(workbook_path),
        "scope_tokens": list(scope_tokens),
        "scoped_defect_count": len(scoped_defects),
        "resolved_defect_count": len(resolved_defects),
        "waived_defect_count": len(waived_defects),
        "open_defect_count": len(open_defects),
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
        "open_defects": open_defects,
        "open_high_critical_defects": open_high_critical_defects,
    }


def _gap_count(report: dict[str, object], *, strict_no_open_high: bool) -> int:
    gap_count = int(report["undocumented_non_passing_test_count"]) + int(
        report["missing_defect_field_count"]
    )
    gap_count += int(report["open_defect_count"])
    return gap_count


def print_report(report: dict[str, object]) -> None:
    print(f"Workbook: {report['workbook']}")
    print(f"Scoped defects: {report['scoped_defect_count']}")
    print(f"Resolved defects: {report['resolved_defect_count']}")
    print(f"Waived defects: {report['waived_defect_count']}")
    print(f"Open defects: {report['open_defect_count']}")
    print(f"Scoped non-passing tests: {report['scoped_non_passing_test_count']}")
    print(f"Documented non-passing tests: {report['documented_non_passing_test_count']}")
    print(f"Undocumented non-passing tests: {report['undocumented_non_passing_test_count']}")
    print(f"Missing defect fields: {report['missing_defect_field_count']}")
    print(f"Open high/critical defects: {report['open_high_critical_defect_count']}")
    for gap in report["open_defects"]:
        print(f"- open defect {gap['defect_id']} {gap['severity']} {gap['status']}")
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
        help=(
            "compatibility flag; all scoped open defects now return nonzero "
            "because Phase 4 requires every defect resolved or explicitly waived"
        ),
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
