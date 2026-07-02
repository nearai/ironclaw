#!/usr/bin/env python3
"""Audit QA workbook feature and test-suite completeness for scoped rows."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import report_coverage

ROOT = Path(__file__).resolve().parents[2]

REQUIRED_FEATURE_COLUMNS = (
    "Feature ID",
    "Feature Name",
    "User Story",
    "Expected Behaviour",
    "Edge Cases",
    "Test Cases",
    "Current Status",
    "Defect Count",
    "Severity",
    "Notes",
    "Last Tested Date",
    "Validation Rules",
    "Dependencies",
    "Known Assumptions",
)

REQUIRED_TEST_CATEGORIES = (
    "Happy",
    "Error",
    "Boundary",
    "Invalid Input",
    "Permission/Security",
    "Performance/Operational",
)

DEFAULT_SCOPE_TOKENS = (
    "webui v2",
    "openai-compatible",
    "responsesapi",
    "responses api",
    "/responses",
    "chat completions",
    "models api",
)


def _records(workbook_path: Path, sheet_name: str) -> list[dict[str, str]]:
    return report_coverage.workbook_sheet_records(workbook_path, sheet_name)


def _in_scope(feature: dict[str, str], scope_tokens: tuple[str, ...]) -> bool:
    scope_fields = (
        "Feature ID",
        "Feature Name",
        "User Story",
        "Expected Behaviour",
        "Validation Rules",
        "Dependencies",
    )
    haystack = " ".join(str(feature.get(field) or "") for field in scope_fields).lower()
    return any(token.lower() in haystack for token in scope_tokens)


def build_audit(
    workbook_path: Path,
    *,
    scope_tokens: tuple[str, ...] = DEFAULT_SCOPE_TOKENS,
) -> dict[str, object]:
    features = _records(workbook_path, "Feature Inventory")
    tests = _records(workbook_path, "Test Cases")
    scoped_features = [feature for feature in features if _in_scope(feature, scope_tokens)]
    scoped_ids = {feature["Feature ID"] for feature in scoped_features}
    tests_by_feature: dict[str, list[dict[str, str]]] = {feature_id: [] for feature_id in scoped_ids}
    for test in tests:
        feature_id = test.get("Feature ID", "")
        if feature_id in tests_by_feature:
            tests_by_feature[feature_id].append(test)

    missing_feature_fields: list[dict[str, str]] = []
    for feature in scoped_features:
        for column in REQUIRED_FEATURE_COLUMNS:
            if not (feature.get(column) or "").strip():
                missing_feature_fields.append(
                    {
                        "feature_id": feature.get("Feature ID", ""),
                        "feature_name": feature.get("Feature Name", ""),
                        "column": column,
                    }
                )

    missing_test_suites = [
        {
            "feature_id": feature["Feature ID"],
            "feature_name": feature.get("Feature Name", ""),
        }
        for feature in scoped_features
        if not tests_by_feature.get(feature["Feature ID"])
    ]

    missing_test_categories: list[dict[str, object]] = []
    for feature in scoped_features:
        feature_id = feature["Feature ID"]
        category_text = "\n".join(
            test.get("Category", "") for test in tests_by_feature.get(feature_id, [])
        ).lower()
        missing = [
            category
            for category in REQUIRED_TEST_CATEGORIES
            if category.lower() not in category_text
        ]
        if missing:
            missing_test_categories.append(
                {
                    "feature_id": feature_id,
                    "feature_name": feature.get("Feature Name", ""),
                    "missing_categories": missing,
                }
            )

    scoped_test_count = sum(len(rows) for rows in tests_by_feature.values())
    return {
        "workbook": str(workbook_path),
        "scope_tokens": list(scope_tokens),
        "scoped_feature_count": len(scoped_features),
        "scoped_test_count": scoped_test_count,
        "missing_feature_field_count": len(missing_feature_fields),
        "missing_test_suite_count": len(missing_test_suites),
        "missing_test_category_count": len(missing_test_categories),
        "missing_feature_fields": missing_feature_fields,
        "missing_test_suites": missing_test_suites,
        "missing_test_categories": missing_test_categories,
    }


def _total_gap_count(report: dict[str, object]) -> int:
    return (
        int(report["missing_feature_field_count"])
        + int(report["missing_test_suite_count"])
        + int(report["missing_test_category_count"])
    )


def print_report(report: dict[str, object]) -> None:
    print(f"Workbook: {report['workbook']}")
    print(f"Scoped features: {report['scoped_feature_count']}")
    print(f"Scoped tests: {report['scoped_test_count']}")
    print(f"Missing feature fields: {report['missing_feature_field_count']}")
    print(f"Missing test suites: {report['missing_test_suite_count']}")
    print(f"Missing test category groups: {report['missing_test_category_count']}")
    for gap in report["missing_feature_fields"]:
        print(f"- missing field {gap['feature_id']} {gap['column']}")
    for gap in report["missing_test_suites"]:
        print(f"- missing test suite {gap['feature_id']} {gap['feature_name']}")
    for gap in report["missing_test_categories"]:
        missing = ",".join(gap["missing_categories"])
        print(f"- missing categories {gap['feature_id']} {missing}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workbook", type=Path, required=True)
    parser.add_argument(
        "--scope-token",
        action="append",
        dest="scope_tokens",
        help="case-insensitive feature row token; may be repeated",
    )
    parser.add_argument("--json", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    scope_tokens = tuple(args.scope_tokens) if args.scope_tokens else DEFAULT_SCOPE_TOKENS
    report = build_audit(args.workbook, scope_tokens=scope_tokens)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_report(report)
    return 0 if _total_gap_count(report) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
