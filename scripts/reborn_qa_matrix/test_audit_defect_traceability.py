#!/usr/bin/env python3
"""Unit tests for scoped QA defect traceability auditing."""

from __future__ import annotations

import tempfile
import unittest
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

import audit_defect_traceability
import audit_workbook_completeness

TEST_COLUMNS = [
    "Test ID",
    "Feature ID",
    "Category",
    "Scenario",
    "Preconditions",
    "Steps",
    "Expected Result",
    "Actual Result",
    "Status",
    "Severity If Fail",
    "Last Tested Date",
    "Notes",
]

DEFECT_COLUMNS = list(audit_defect_traceability.REQUIRED_DEFECT_COLUMNS)


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
    path: Path,
    *,
    feature_rows: list[list[str]],
    test_rows: list[list[str]],
    defect_rows: list[list[str]],
) -> None:
    workbook_xml = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        "<x:sheets>"
        '<x:sheet name="Feature Inventory" sheetId="1" r:id="rId1" />'
        '<x:sheet name="Test Cases" sheetId="2" r:id="rId2" />'
        '<x:sheet name="Defects" sheetId="3" r:id="rId3" />'
        "</x:sheets>"
        "</x:workbook>"
    )
    rels_xml = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="worksheet" Target="/xl/worksheets/sheet1.xml" />'
        '<Relationship Id="rId2" Type="worksheet" Target="/xl/worksheets/sheet2.xml" />'
        '<Relationship Id="rId3" Type="worksheet" Target="/xl/worksheets/sheet3.xml" />'
        "</Relationships>"
    )
    with zipfile.ZipFile(path, "w") as workbook:
        workbook.writestr("xl/workbook.xml", workbook_xml)
        workbook.writestr("xl/_rels/workbook.xml.rels", rels_xml)
        workbook.writestr("xl/worksheets/sheet1.xml", _sheet_xml(feature_rows))
        workbook.writestr("xl/worksheets/sheet2.xml", _sheet_xml(test_rows))
        workbook.writestr("xl/worksheets/sheet3.xml", _sheet_xml(defect_rows))


def _feature(feature_id: str, name: str = "WebUI v2 Gateway") -> list[str]:
    return [
        feature_id,
        name,
        "story",
        "expected",
        "edge cases",
        "test cases",
        "Passed",
        "0",
        "None",
        "notes",
        "2026-06-28",
        "rules",
        "deps",
        "assumptions",
    ]


def _test_row(
    test_id: str,
    feature_id: str,
    *,
    status: str = "Blocked - external credential preflight",
) -> list[str]:
    return [
        test_id,
        feature_id,
        "Live",
        "Scenario",
        "Preconditions",
        "Steps",
        "Expected",
        "Actual",
        status,
        "High",
        "2026-06-28",
        "Evidence notes",
    ]


def _defect(
    defect_id: str,
    feature_id: str,
    test_id: str,
    *,
    severity: str = "High",
    status: str = "Waived - external credential preflight",
) -> list[str]:
    return [
        defect_id,
        feature_id,
        f"{test_id} blocked by external credentials",
        f"Run {test_id}.",
        "Live canary completes.",
        "Blocked at credential preflight.",
        severity,
        "Required external OAuth credentials are unavailable.",
        status,
        "2026-06-28",
    ]


class AuditDefectTraceabilityTests(unittest.TestCase):
    def test_blocked_scoped_row_requires_matching_defect_or_waiver(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-055"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row("REBCLI-055-TC-18", "REBCLI-055"),
                ],
                defect_rows=[DEFECT_COLUMNS],
            )

            report = audit_defect_traceability.build_audit(workbook_path)

            self.assertEqual(report["scoped_defect_count"], 0)
            self.assertEqual(report["resolved_defect_count"], 0)
            self.assertEqual(report["waived_defect_count"], 0)
            self.assertEqual(report["open_defect_count"], 0)
            self.assertEqual(report["scoped_non_passing_test_count"], 1)
            self.assertEqual(report["undocumented_non_passing_test_count"], 1)
            self.assertEqual(
                audit_defect_traceability.main(["--workbook", str(workbook_path)]),
                1,
            )

    def test_waived_defect_documents_blocked_row_and_satisfies_strict_open_high(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-055"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row("REBCLI-055-TC-18", "REBCLI-055"),
                ],
                defect_rows=[
                    DEFECT_COLUMNS,
                    _defect("DEF-053", "REBCLI-055", "REBCLI-055-TC-18"),
                ],
            )

            report = audit_defect_traceability.build_audit(workbook_path)

            self.assertEqual(report["scoped_defect_count"], 1)
            self.assertEqual(report["resolved_defect_count"], 0)
            self.assertEqual(report["waived_defect_count"], 1)
            self.assertEqual(report["open_defect_count"], 0)
            self.assertEqual(report["documented_non_passing_test_count"], 1)
            self.assertEqual(report["undocumented_non_passing_test_count"], 0)
            self.assertEqual(report["open_high_critical_defect_count"], 0)
            self.assertEqual(
                audit_defect_traceability.main(
                    ["--workbook", str(workbook_path), "--strict-no-open-high"]
                ),
                0,
            )

    def test_prefix_colliding_test_ids_do_not_share_defect_linkage(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-055"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row("REBCLI-055-TC-1", "REBCLI-055"),
                    _test_row("REBCLI-055-TC-18", "REBCLI-055"),
                ],
                defect_rows=[
                    DEFECT_COLUMNS,
                    _defect("DEF-053", "REBCLI-055", "REBCLI-055-TC-18"),
                ],
            )

            report = audit_defect_traceability.build_audit(workbook_path)

            self.assertEqual(report["documented_non_passing_test_count"], 1)
            self.assertEqual(report["undocumented_non_passing_test_count"], 1)
            self.assertEqual(
                report["undocumented_non_passing_tests"][0]["test_id"],
                "REBCLI-055-TC-1",
            )

    def test_missing_defect_fields_and_open_high_defects_are_reported(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            defect = _defect(
                "DEF-053",
                "REBCLI-055",
                "REBCLI-055-TC-18",
                status="Open",
            )
            defect[7] = ""
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-055"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row("REBCLI-055-TC-18", "REBCLI-055"),
                ],
                defect_rows=[DEFECT_COLUMNS, defect],
            )

            report = audit_defect_traceability.build_audit(workbook_path)

            self.assertEqual(report["scoped_defect_count"], 1)
            self.assertEqual(report["resolved_defect_count"], 0)
            self.assertEqual(report["waived_defect_count"], 0)
            self.assertEqual(report["open_defect_count"], 1)
            self.assertEqual(report["missing_defect_field_count"], 1)
            self.assertEqual(
                report["missing_defect_fields"][0]["column"],
                "Root Cause Hypothesis",
            )
            self.assertEqual(report["open_high_critical_defect_count"], 1)
            self.assertEqual(
                audit_defect_traceability.main(
                    ["--workbook", str(workbook_path), "--strict-no-open-high"]
                ),
                1,
            )

    def test_resolved_defect_counts_as_fixed_and_closes_gap(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-097"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row(
                        "REBCLI-097-TC-01",
                        "REBCLI-097",
                        status="Passed",
                    ),
                ],
                defect_rows=[
                    DEFECT_COLUMNS,
                    _defect(
                        "DEF-055",
                        "REBCLI-097",
                        "REBCLI-097-TC-01",
                        severity="Low",
                        status="Resolved",
                    ),
                ],
            )

            report = audit_defect_traceability.build_audit(workbook_path)

            self.assertEqual(report["scoped_defect_count"], 1)
            self.assertEqual(report["resolved_defect_count"], 1)
            self.assertEqual(report["waived_defect_count"], 0)
            self.assertEqual(report["open_defect_count"], 0)
            self.assertEqual(
                audit_defect_traceability.main(["--workbook", str(workbook_path)]),
                0,
            )


if __name__ == "__main__":
    unittest.main()
