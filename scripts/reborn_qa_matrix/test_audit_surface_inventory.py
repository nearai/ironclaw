#!/usr/bin/env python3
"""Unit tests for WebUI v2/ResponsesAPI surface inventory auditing."""

from __future__ import annotations

import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

import audit_surface_inventory

# audit_surface_inventory (imported above) already inserts scripts/ci/lib onto
# sys.path as a side effect of module import; the explicit insert here is
# belt-and-suspenders so this file does not depend on that import ordering.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ci" / "lib"))
import crate_tree  # noqa: E402


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


def _write_repo(
    root: Path,
    *,
    webui_dir_rel: str = "crates/ironclaw_webui",
    openai_dir_rel: str = "crates/ironclaw_reborn_openai_compat",
) -> None:
    # Real Cargo.toml files (not just directories with source in them) so
    # crate_tree.py's discovery can find ironclaw_webui and
    # ironclaw_reborn_openai_compat wherever `webui_dir_rel`/`openai_dir_rel`
    # place them — the fixture must clear the MIN_CRATE_DIRECTORIES floor too,
    # so it also carries filler crates rather than being a two-crate stub.
    (root / webui_dir_rel).mkdir(parents=True, exist_ok=True)
    (root / webui_dir_rel / "Cargo.toml").write_text(
        '[package]\nname = "ironclaw_webui"\n', encoding="utf-8"
    )
    (root / openai_dir_rel).mkdir(parents=True, exist_ok=True)
    (root / openai_dir_rel / "Cargo.toml").write_text(
        '[package]\nname = "ironclaw_reborn_openai_compat"\n', encoding="utf-8"
    )
    for index in range(crate_tree.MIN_CRATE_DIRECTORIES + 2):
        filler = root / "crates" / f"ironclaw_filler_{index}"
        filler.mkdir(parents=True, exist_ok=True)
        (filler / "Cargo.toml").write_text(
            f'[package]\nname = "ironclaw_filler_{index}"\n', encoding="utf-8"
        )

    app_dir = root / webui_dir_rel / "frontend/src/app"
    app_dir.mkdir(parents=True)
    (app_dir / "app.tsx").write_text(
        """
        <Route path="chat" element={<ChatPage />} />
        <Route path="jobs" element={<JobsPage />} />
        <Route path="settings/:tab" element={<SettingsPage />} />
        """,
        encoding="utf-8",
    )
    # descriptors.rs lives under src/webui_v2/, matching
    # api_surfaces()'s real production path — a fixture bug (this
    # subdirectory was missing) predates WS10 and is fixed here in passing;
    # confirmed pre-existing via a HEAD-only run in the scratchpad before this
    # change (see the WS10 report for this crate move).
    webui_src_dir = root / webui_dir_rel / "src" / "webui_v2"
    webui_src_dir.mkdir(parents=True)
    (webui_src_dir / "descriptors.rs").write_text(
        'pub const WEBUI_V2_PATTERN_LIST_THREADS: &str = "/api/webchat/v2/threads";\n'
        'pub const WEBUI_V2_PATTERN_LIST_PROJECTS: &str = "/api/webchat/v2/projects";\n',
        encoding="utf-8",
    )
    openai_dir = root / openai_dir_rel / "src"
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

    def test_real_repository_api_surfaces_are_extractable(self):
        # WS10: this crosses the ironclaw_webui and ironclaw_reborn_openai_compat
        # crate-directory resolution against whatever the LIVE repo's current
        # layout is (flat or already family-moved) — unlike
        # test_build_audit_flags_only_surfaces_missing_from_feature_inventory,
        # which pins a synthetic, stable fixture.
        surfaces = audit_surface_inventory.api_surfaces(audit_surface_inventory.ROOT)
        by_kind = {surface.kind for surface in surfaces}
        self.assertIn("webui_api_pattern", by_kind)
        self.assertIn("openai_compat_api_pattern", by_kind)

    def test_crate_resolution_follows_a_family_move(self):
        """WS10: browser_routes/api_surfaces still find their sources when
        ironclaw_webui and ironclaw_reborn_openai_compat sit one family
        directory down (crates/<family>/ironclaw_*, PROPOSAL §5) instead of
        flat under crates/ — the exact shape the restructure produces."""
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            _write_repo(
                root,
                webui_dir_rel="crates/substrates/ironclaw_webui",
                openai_dir_rel="crates/substrates/ironclaw_reborn_openai_compat",
            )

            routes = audit_surface_inventory.browser_routes(root)
            self.assertTrue(
                any(route.identifier == "/chat" for route in routes)
            )
            self.assertTrue(
                all(
                    route.source.startswith("crates/substrates/ironclaw_webui/")
                    for route in routes
                )
            )

            surfaces = audit_surface_inventory.api_surfaces(root)
            self.assertTrue(surfaces)
            self.assertTrue(
                all(
                    surface.source.startswith("crates/substrates/")
                    for surface in surfaces
                )
            )

    def test_crate_resolution_fails_closed_when_webui_crate_missing(self):
        """A repo with no ironclaw_webui crate at all must refuse loudly,
        never silently report zero routes (which build_audit would read as
        "fully covered" — the WS10 vacuous-pass failure mode)."""
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            for index in range(crate_tree.MIN_CRATE_DIRECTORIES + 2):
                filler = root / "crates" / f"ironclaw_filler_{index}"
                filler.mkdir(parents=True)
                (filler / "Cargo.toml").write_text(
                    f'[package]\nname = "ironclaw_filler_{index}"\n',
                    encoding="utf-8",
                )
            with self.assertRaisesRegex(
                RuntimeError, "cannot resolve the ironclaw_webui crate"
            ):
                audit_surface_inventory.browser_routes(root)

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
