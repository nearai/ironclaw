#!/usr/bin/env python3
"""Unit tests for QA matrix coverage reporting."""

from __future__ import annotations

import tempfile
import unittest
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

import report_coverage


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


def _write_workbook(path: Path, *, feature_rows: list[list[str]], test_rows: list[list[str]]) -> None:
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


class ReportCoverageTests(unittest.TestCase):
    def test_workbook_ids_reads_artifact_style_string_cells(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[
                    ["Feature ID", "Feature Name"],
                    ["REBCLI-055", "WebUI v2 Gateway"],
                    ["REBCLI-099", "OpenAI Models"],
                ],
                test_rows=[
                    ["Test ID", "Feature ID"],
                    ["REBCLI-055-TC-01", "REBCLI-055"],
                    ["REBCLI-099-TC-01", "REBCLI-099"],
                    ["REBCLI-099", "REBCLI-099-TC-02"],
                ],
            )

            feature_ids, test_ids = report_coverage.workbook_ids(workbook_path)

            self.assertEqual(feature_ids, {"REBCLI-055", "REBCLI-099"})
            self.assertEqual(test_ids, {"REBCLI-055-TC-01", "REBCLI-099-TC-01"})

    def test_build_report_tracks_runner_ids_missing_from_workbook(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            workbook_path = Path(tmpdir) / "matrix.xlsx"
            _write_workbook(
                workbook_path,
                feature_rows=[["Feature ID"], ["REBCLI-099"]],
                test_rows=[
                    ["Test ID", "Feature ID"],
                    ["REBCLI-099-TC-01", "REBCLI-099"],
                ],
            )

            report = report_coverage.build_report(workbook_path, Path(tmpdir))

            self.assertEqual(report["feature_count"], 1)
            self.assertEqual(report["matrix_test_count"], 1)
            self.assertIn("REBCLI-099-TC-02", report["runner_ids_not_in_workbook"])
            self.assertGreater(report["hermetic_runner_test_count"], 0)


if __name__ == "__main__":
    unittest.main()
