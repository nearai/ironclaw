#!/usr/bin/env python3
"""Unit tests for WebUI v2/ResponsesAPI surface inventory auditing."""

from __future__ import annotations

import tempfile
import unittest
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

import audit_surface_inventory


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


def _write_workbook(path: Path, feature_rows: list[list[str]]) -> None:
    workbook_xml = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        "<x:sheets>"
        '<x:sheet name="Feature Inventory" sheetId="1" r:id="rId1" />'
        "</x:sheets>"
        "</x:workbook>"
    )
    rels_xml = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="worksheet" Target="/xl/worksheets/sheet1.xml" />'
        "</Relationships>"
    )
    with zipfile.ZipFile(path, "w") as workbook:
        workbook.writestr("xl/workbook.xml", workbook_xml)
        workbook.writestr("xl/_rels/workbook.xml.rels", rels_xml)
        workbook.writestr("xl/worksheets/sheet1.xml", _sheet_xml(feature_rows))


def _write_repo(root: Path) -> None:
    app_dir = root / "crates/ironclaw_webui/frontend/src/app"
    app_dir.mkdir(parents=True)
    (app_dir / "app.tsx").write_text(
        """
        <Route path="chat" element={<ChatPage />} />
        <Route path="jobs" element={<JobsPage />} />
        <Route path="settings/:tab" element={<SettingsPage />} />
        """,
        encoding="utf-8",
    )
    webui_dir = root / "crates/ironclaw_webui/src"
    webui_dir.mkdir(parents=True)
    (webui_dir / "descriptors.rs").write_text(
        'pub const WEBUI_V2_PATTERN_LIST_THREADS: &str = "/api/webchat/v2/threads";\n'
        'pub const WEBUI_V2_PATTERN_LIST_PROJECTS: &str = "/api/webchat/v2/projects";\n',
        encoding="utf-8",
    )
    openai_dir = root / "crates/ironclaw_reborn_openai_compat/src"
    openai_dir.mkdir(parents=True)
    (openai_dir / "descriptors.rs").write_text(
        'pub const OPENAI_COMPAT_PATTERN_RESPONSES_API_CREATE: &str = "/api/v1/responses";\n'
        'pub const OPENAI_COMPAT_PATTERN_MODELS_LIST: &str = "/v1/models";\n',
        encoding="utf-8",
    )


class AuditSurfaceInventoryTests(unittest.TestCase):
    def test_real_repository_react_routes_are_extractable(self):
        routes = audit_surface_inventory.browser_routes(audit_surface_inventory.ROOT)
        identifiers = {route.identifier for route in routes}

        self.assertIn("/chat", identifiers)
        self.assertIn("/settings/:tab", identifiers)
        self.assertTrue(
            all(route.source.endswith("frontend/src/app/app.tsx") for route in routes)
        )

    def test_build_audit_flags_only_surfaces_missing_from_feature_inventory(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            _write_repo(root)
            workbook = root / "matrix.xlsx"
            _write_workbook(
                workbook,
                [
                    ["Feature ID", "Feature Name"],
                    ["REBCLI-001", "WebUI v2 Chat Screen and Message APIs"],
                    ["REBCLI-002", "WebUI v2 Settings Panels"],
                    ["REBCLI-003", "OpenAI-Compatible Responses API"],
                    ["REBCLI-004", "OpenAI-Compatible Models API"],
                    ["REBCLI-005", "WebUI v2 Project APIs"],
                ],
            )

            report = audit_surface_inventory.build_audit(workbook, root)

            uncovered = {
                surface["identifier"] for surface in report["uncovered_surfaces"]
            }
            self.assertIn("/jobs", uncovered)
            self.assertNotIn("/chat", uncovered)
            self.assertNotIn("/settings/:tab", uncovered)
            self.assertNotIn("/api/v1/responses", uncovered)
            self.assertNotIn("/v1/models", uncovered)
            self.assertNotIn("/api/webchat/v2/projects", uncovered)

    def test_main_exits_zero_when_all_surfaces_have_feature_keywords(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            _write_repo(root)
            workbook = root / "matrix.xlsx"
            _write_workbook(
                workbook,
                [
                    ["Feature ID", "Feature Name"],
                    ["REBCLI-001", "Chat Thread Jobs Settings Project Responses Models"],
                ],
            )

            exit_code = audit_surface_inventory.main(
                ["--workbook", str(workbook), "--repo-root", str(root)]
            )

            self.assertEqual(exit_code, 0)


if __name__ == "__main__":
    unittest.main()
