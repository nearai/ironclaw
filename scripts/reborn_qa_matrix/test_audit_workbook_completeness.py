#!/usr/bin/env python3
"""Unit tests for scoped QA workbook completeness auditing."""

from __future__ import annotations

import tempfile
import unittest
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

import audit_workbook_completeness


def _sheet_xml(rows: list[list[str]]) -> str:
    rendered_rows = []
    for row_index, row in enumerate(rows, start=1):
        cells = []
        for col_index, value in enumerate(row):
            column = chr(ord("A") + col_index)
            cells.append(
                f'<x:c r="{column}{row_index}" t="str"><x:v>'
                f"{escape(value)}</x:v></x:c>"
            )
        rendered_rows.append(f'<x:row r="{row_index}">{"".join(cells)}</x:row>')
    return (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        f'<x:sheetData>{"".join(rendered_rows)}</x:sheetData>'
        "</x:worksheet>"
    )


def _write_workbook(
    path: Path, *, feature_rows: list[list[str]], test_rows: list[list[str]]
) -> None:
    workbook_xml = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        "<x:sheets>"
        '<x:sheet name="Feature Inventory" sheetId="1" r:id="rId1" />'
        '<x:sheet name="Test Cases" sheetId="2" r:id="rId2" />'
        "</x:sheets>"
        "</x:workbook>"
    )
    rels_xml = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="worksheet" Target="/xl/worksheets/sheet1.xml" />'
        '<Relationship Id="rId2" Type="worksheet" Target="/xl/worksheets/sheet2.xml" />'
        "</Relationships>"
    )
    with zipfile.ZipFile(path, "w") as workbook:
        workbook.writestr("xl/workbook.xml", workbook_xml)
        workbook.writestr("xl/_rels/workbook.xml.rels", rels_xml)
        workbook.writestr("xl/worksheets/sheet1.xml", _sheet_xml(feature_rows))
        workbook.writestr("xl/worksheets/sheet2.xml", _sheet_xml(test_rows))


def _complete_feature(
    feature_id: str,
    name: str,
    *,
    dependencies: str = "WebUI v2 and ResponsesAPI crates.",
) -> list[str]:
    return [
        feature_id,
        name,
        "As a user, I can exercise this workflow.",
        "The workflow behaves according to current code.",
        "Empty, denied, malformed, and stale states are handled.",
        "Happy, error, boundary, invalid, permission, and performance cases.",
        "Passed",
        "0",
        "None open",
        "Evidence recorded.",
        "2026-06-28",
        "Inputs are validated by current code paths.",
        dependencies,
        "Assumes current Reborn beta routing.",
    ]


def _category_tests(feature_id: str) -> list[list[str]]:
    return [
        [f"{feature_id}-TC-01", feature_id, "Happy"],
        [f"{feature_id}-TC-02", feature_id, "Error"],
        [f"{feature_id}-TC-03", feature_id, "Boundary"],
        [f"{feature_id}-TC-04", feature_id, "Invalid Input"],
        [f"{feature_id}-TC-05", feature_id, "Permission/Security"],
        [f"{feature_id}-TC-06", feature_id, "Performance/Operational"],
    ]


class AuditWorkbookCompletenessTests(unittest.TestCase):
    def test_build_audit_passes_complete_scoped_feature(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _complete_feature("REBCLI-001", "WebUI v2 Chat Screen"),
                    _complete_feature(
                        "REBCLI-002",
                        "Legacy CLI out of scope",
                        dependencies="Legacy CLI crate.",
                    ),
                ],
                test_rows=[
                    ["Test ID", "Feature ID", "Category"],
                    *_category_tests("REBCLI-001"),
                ],
            )

            report = audit_workbook_completeness.build_audit(workbook_path)

            self.assertEqual(report["scoped_feature_count"], 1)
            self.assertEqual(report["scoped_test_count"], 6)
            self.assertEqual(report["missing_feature_field_count"], 0)
            self.assertEqual(report["missing_test_suite_count"], 0)
            self.assertEqual(report["missing_test_category_count"], 0)

    def test_build_audit_reports_missing_fields_suites_and_categories(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            incomplete = _complete_feature("REBCLI-010", "OpenAI-Compatible Responses API")
            incomplete[2] = ""
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    incomplete,
                    _complete_feature("REBCLI-011", "WebUI v2 Settings Screen"),
                    _complete_feature("REBCLI-012", "WebUI v2 Admin Screen"),
                ],
                test_rows=[
                    ["Test ID", "Feature ID", "Category"],
                    ["REBCLI-010-TC-01", "REBCLI-010", "Happy"],
                    *_category_tests("REBCLI-011")[:5],
                ],
            )

            report = audit_workbook_completeness.build_audit(workbook_path)

            self.assertEqual(report["missing_feature_field_count"], 1)
            self.assertEqual(
                report["missing_feature_fields"][0]["column"],
                "User Story",
            )
            self.assertEqual(
                {gap["feature_id"] for gap in report["missing_test_suites"]},
                {"REBCLI-012"},
            )
            missing_by_feature = {
                gap["feature_id"]: gap["missing_categories"]
                for gap in report["missing_test_categories"]
            }
            self.assertIn("Error", missing_by_feature["REBCLI-010"])
            self.assertEqual(
                missing_by_feature["REBCLI-011"],
                ["Performance/Operational"],
            )

    def test_main_exits_nonzero_when_gaps_exist(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _complete_feature("REBCLI-020", "WebUI v2 Logs Screen"),
                ],
                test_rows=[["Test ID", "Feature ID", "Category"]],
            )

            self.assertEqual(
                audit_workbook_completeness.main(["--workbook", str(workbook_path)]),
                1,
            )


if __name__ == "__main__":
    unittest.main()
