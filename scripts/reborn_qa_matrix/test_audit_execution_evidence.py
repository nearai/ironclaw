#!/usr/bin/env python3
"""Unit tests for scoped QA execution-evidence auditing."""

from __future__ import annotations

import tempfile
import unittest
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

import audit_execution_evidence
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


def _feature(feature_id: str, name: str = "WebUI v2 Chat") -> list[str]:
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
    status: str = "Passed",
    actual: str = "Actual",
    notes: str = "Evidence notes",
) -> list[str]:
    return [
        test_id,
        feature_id,
        "Happy",
        "Scenario",
        "Preconditions",
        "Steps",
        "Expected",
        actual,
        status,
        "High",
        "2026-06-28",
        notes,
    ]


class AuditExecutionEvidenceTests(unittest.TestCase):
    def test_build_audit_counts_passed_external_and_blocked_rows(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-001"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row("REBCLI-001-TC-01", "REBCLI-001"),
                    _test_row(
                        "REBCLI-001-TC-02",
                        "REBCLI-001",
                        status="External-existing coverage",
                    ),
                    _test_row(
                        "REBCLI-001-TC-03",
                        "REBCLI-001",
                        status="Blocked - external credential preflight",
                    ),
                ],
            )

            report = audit_execution_evidence.build_audit(workbook_path)

            self.assertEqual(report["scoped_test_count"], 3)
            self.assertEqual(report["passed_test_count"], 1)
            self.assertEqual(report["external_existing_test_count"], 1)
            self.assertEqual(report["blocked_test_count"], 1)
            self.assertEqual(report["missing_execution_field_count"], 0)
            self.assertEqual(report["unknown_status_test_count"], 0)

    def test_main_strict_no_blocked_fails_on_documented_blocked_rows(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-001"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row(
                        "REBCLI-001-TC-01",
                        "REBCLI-001",
                        status="Blocked - external credential preflight",
                    ),
                ],
            )

            self.assertEqual(
                audit_execution_evidence.main(["--workbook", str(workbook_path)]),
                0,
            )
            self.assertEqual(
                audit_execution_evidence.main(
                    ["--workbook", str(workbook_path), "--strict-no-blocked"]
                ),
                1,
            )

    def test_build_audit_reports_missing_execution_fields_and_unknown_status(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            missing = _test_row("REBCLI-001-TC-01", "REBCLI-001", status="Needs Review")
            missing[7] = ""
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-001"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    missing,
                ],
            )

            report = audit_execution_evidence.build_audit(workbook_path)

            self.assertEqual(report["missing_execution_field_count"], 1)
            self.assertEqual(
                report["missing_execution_fields"][0]["column"],
                "Actual Result",
            )
            self.assertEqual(report["unknown_status_test_count"], 1)
            self.assertEqual(
                report["unknown_status_tests"][0]["status"],
                "Needs Review",
            )

    def test_build_audit_requires_external_existing_source_reference(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-001"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row(
                        "REBCLI-001-TC-01",
                        "REBCLI-001",
                        status="External-existing coverage",
                        actual="Covered elsewhere",
                        notes="No source owner named",
                    ),
                ],
            )

            report = audit_execution_evidence.build_audit(workbook_path)

            self.assertEqual(report["missing_external_reference_count"], 1)
            self.assertEqual(
                report["missing_external_references"][0]["test_id"],
                "REBCLI-001-TC-01",
            )
            self.assertEqual(
                audit_execution_evidence.main(["--workbook", str(workbook_path)]),
                1,
            )

    def test_build_audit_rejects_superseded_pr_5348_without_split_pr_reference(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-001"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row(
                        "REBCLI-001-TC-01",
                        "REBCLI-001",
                        status="External-existing coverage",
                        actual="External-existing browser coverage",
                        notes="Covered by nearai/ironclaw#5348 legacy browser flows.",
                    ),
                    _test_row(
                        "REBCLI-001-TC-02",
                        "REBCLI-001",
                        status="External-existing coverage",
                        actual="External-existing browser coverage",
                        notes=(
                            "Covered by nearai/ironclaw#5348, superseded by "
                            "nearai/ironclaw#5371 and #5372 split browser flows."
                        ),
                    ),
                ],
            )

            report = audit_execution_evidence.build_audit(workbook_path)

            self.assertEqual(report["missing_external_reference_count"], 0)
            self.assertEqual(report["stale_external_reference_count"], 1)
            self.assertEqual(
                report["stale_external_references"][0]["test_id"],
                "REBCLI-001-TC-01",
            )

    def test_stale_pr_detection_uses_number_boundaries(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    list(audit_workbook_completeness.REQUIRED_FEATURE_COLUMNS),
                    _feature("REBCLI-001"),
                ],
                test_rows=[
                    TEST_COLUMNS,
                    _test_row(
                        "REBCLI-001-TC-01",
                        "REBCLI-001",
                        status="External-existing coverage",
                        actual="External-existing browser coverage",
                        notes="Covered by nearai/ironclaw#15348 unrelated workflow.",
                    ),
                ],
            )

            report = audit_execution_evidence.build_audit(workbook_path)

            self.assertEqual(report["missing_external_reference_count"], 0)
            self.assertEqual(report["stale_external_reference_count"], 0)


if __name__ == "__main__":
    unittest.main()
