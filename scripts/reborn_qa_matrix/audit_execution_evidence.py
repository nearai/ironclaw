#!/usr/bin/env python3
"""Audit scoped QA workbook test rows for execution evidence."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import audit_workbook_completeness

REQUIRED_EXECUTION_COLUMNS = (
    "Actual Result",
    "Status",
    "Severity If Fail",
    "Last Tested Date",
    "Notes",
)

PASSED_STATUSES = ("passed",)
EXTERNAL_STATUSES = ("external-existing coverage",)
BLOCKED_PREFIXES = ("blocked",)


def _status_class(status: str) -> str:
    normalized = status.strip().lower()
    if normalized in PASSED_STATUSES:
        return "passed"
    if normalized in EXTERNAL_STATUSES:
        return "external_existing"
    if normalized.startswith(BLOCKED_PREFIXES):
        return "blocked"
    return "unknown"


def build_audit(
    workbook_path: Path,
    *,
    scope_tokens: tuple[str, ...] = audit_workbook_completeness.DEFAULT_SCOPE_TOKENS,
) -> dict[str, object]:
    features = audit_workbook_completeness._records(workbook_path, "Feature Inventory")
    tests = audit_workbook_completeness._records(workbook_path, "Test Cases")
    scoped_ids = {
        feature["Feature ID"]
        for feature in features
        if audit_workbook_completeness._in_scope(feature, scope_tokens)
    }
    scoped_tests = [test for test in tests if test.get("Feature ID", "") in scoped_ids]

    missing_execution_fields: list[dict[str, str]] = []
    status_counts = {
        "passed": 0,
        "external_existing": 0,
        "blocked": 0,
        "unknown": 0,
    }
    blocked_tests: list[dict[str, str]] = []
    unknown_status_tests: list[dict[str, str]] = []
    tests_with_missing_fields: set[str] = set()

    for test in scoped_tests:
        test_id = test.get("Test ID", "")
        for column in REQUIRED_EXECUTION_COLUMNS:
            if not (test.get(column) or "").strip():
                tests_with_missing_fields.add(test_id)
                missing_execution_fields.append(
                    {
                        "test_id": test_id,
                        "feature_id": test.get("Feature ID", ""),
                        "column": column,
                    }
                )
        status = test.get("Status", "")
        status_class = _status_class(status)
        status_counts[status_class] += 1
        if status_class == "blocked":
            blocked_tests.append(
                {
                    "test_id": test_id,
                    "feature_id": test.get("Feature ID", ""),
                    "status": status,
                    "category": test.get("Category", ""),
                    "notes": test.get("Notes", ""),
                }
            )
        elif status_class == "unknown":
            unknown_status_tests.append(
                {
                    "test_id": test_id,
                    "feature_id": test.get("Feature ID", ""),
                    "status": status,
                    "category": test.get("Category", ""),
                }
            )

    return {
        "workbook": str(workbook_path),
        "scope_tokens": list(scope_tokens),
        "scoped_test_count": len(scoped_tests),
        "passed_test_count": status_counts["passed"],
        "external_existing_test_count": status_counts["external_existing"],
        "blocked_test_count": status_counts["blocked"],
        "unknown_status_test_count": status_counts["unknown"],
        "missing_execution_field_count": len(missing_execution_fields),
        "execution_evidence_test_count": len(scoped_tests)
        - len(tests_with_missing_fields),
        "missing_execution_fields": missing_execution_fields,
        "blocked_tests": blocked_tests,
        "unknown_status_tests": unknown_status_tests,
    }


def _gap_count(report: dict[str, object], *, strict_no_blocked: bool) -> int:
    gap_count = int(report["missing_execution_field_count"]) + int(
        report["unknown_status_test_count"]
    )
    if strict_no_blocked:
        gap_count += int(report["blocked_test_count"])
    return gap_count


def print_report(report: dict[str, object]) -> None:
    print(f"Workbook: {report['workbook']}")
    print(f"Scoped tests: {report['scoped_test_count']}")
    print(f"Passed tests: {report['passed_test_count']}")
    print(f"External-existing tests: {report['external_existing_test_count']}")
    print(f"Blocked tests: {report['blocked_test_count']}")
    print(f"Unknown-status tests: {report['unknown_status_test_count']}")
    print(f"Missing execution fields: {report['missing_execution_field_count']}")
    for gap in report["missing_execution_fields"]:
        print(f"- missing execution field {gap['test_id']} {gap['column']}")
    for gap in report["unknown_status_tests"]:
        print(f"- unknown status {gap['test_id']} {gap['status']}")
    for blocked in report["blocked_tests"]:
        print(f"- blocked {blocked['test_id']} {blocked['status']}")


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
        "--strict-no-blocked",
        action="store_true",
        help="return nonzero when scoped rows include documented blocked tests",
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
    return 0 if _gap_count(report, strict_no_blocked=args.strict_no_blocked) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
