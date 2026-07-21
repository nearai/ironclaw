"""Reborn WebChat v2: agent-produced files are downloadable from the UI.

Drives the real `ironclaw-reborn serve` binary (v2 SPA) under the
`local-dev-yolo` profile — minimal approvals, so an in-workspace `write_file`
auto-proceeds instead of parking on a destructive-write gate. The mock LLM
turns the prompt into two Reborn `builtin.write_file` capability calls via the
provider-facing `builtin__write_file` name (a CSV and a PDF), then a reply that
references their `/workspace` paths. The SPA renders those paths as download
chips; clicking one performs the bearer-authenticated blob fetch and saves the
file.

Complements `webui_v2_e2e.rs` (in-process, asserts the same endpoints against a
real agent-produced file) by covering the browser chip-render + click-download
integration. Requires the full E2E harness (cargo build + reborn serve + mock
LLM + Chromium); it is CI-run, not exercised by `cargo test`.
"""

import json
import re
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture dependency
    reborn_v2_yolo_page,  # noqa: F401 - imported fixture
    reborn_v2_yolo_server,  # noqa: F401 - imported fixture dependency
)

CSV_PATH = "/workspace/report.csv"
PDF_PATH = "/workspace/report.pdf"
CSV_BYTES = b"name,score\nalice,90\nbob,85\n"


async def _read_download_bytes(download) -> bytes:
    return Path(await download.path()).read_bytes()


async def test_reborn_v2_agent_files_render_download_chips(reborn_v2_yolo_page):
    """Agent writes a CSV + PDF; the reply renders chips that download the bytes."""
    page = reborn_v2_yolo_page

    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill("Please produce a downloadable CSV and PDF report.")
    await composer.press("Enter")

    # The assistant reply references both /workspace paths, which the SPA turns
    # into chips that open the shared attachment preview modal.
    csv_chip = page.locator(SEL_V2["project_file_chip_for"].format(path=CSV_PATH))
    pdf_chip = page.locator(SEL_V2["project_file_chip_for"].format(path=PDF_PATH))
    await expect(csv_chip).to_be_visible(timeout=45000)
    await expect(pdf_chip).to_be_visible(timeout=45000)

    # The chip's inline download icon performs the bearer-authenticated blob
    # fetch and saves the exact bytes the agent wrote — no modal needed.
    csv_download_icon = page.locator(
        SEL_V2["project_file_download_for"].format(path=CSV_PATH)
    )
    async with page.expect_download() as csv_dl:
        await csv_download_icon.click()
    csv_download = await csv_dl.value
    assert csv_download.suggested_filename == "report.csv"
    assert await _read_download_bytes(csv_download) == CSV_BYTES

    # Clicking the chip body instead opens the preview modal, whose footer
    # Download action saves the bytes too (covers the preview path for the PDF).
    modal_download = page.locator(SEL_V2["attachment_download"])
    await pdf_chip.click()
    await expect(modal_download).to_be_visible(timeout=15000)
    async with page.expect_download() as pdf_dl:
        await modal_download.click()
    pdf_download = await pdf_dl.value
    assert pdf_download.suggested_filename == "report.pdf"
    assert (await _read_download_bytes(pdf_download)).startswith(b"%PDF-")


async def test_workspace_viewer_reports_download_failure(reborn_v2_yolo_page):
    """A failed Workspace download produces localized, user-visible feedback."""
    page = reborn_v2_yolo_page

    async def serve_mounts(route):
        await route.fulfill(
            content_type="application/json",
            body=json.dumps({"mounts": [{"mount": "workspace"}]}),
        )

    async def serve_stat(route):
        await route.fulfill(
            content_type="application/json",
            body=json.dumps(
                {
                    "stat": {
                        "kind": "file",
                        "mime_type": "application/pdf",
                        "size_bytes": 12,
                    }
                }
            ),
        )

    async def serve_listing(route):
        await route.fulfill(
            content_type="application/json",
            body=json.dumps(
                {
                    "mount": "workspace",
                    "entries": [
                        {
                            "name": "report.pdf",
                            "path": "report.pdf",
                            "kind": "file",
                        }
                    ],
                }
            ),
        )

    async def fail_content_download(route):
        await route.abort("internetdisconnected")

    await page.route("**/api/webchat/v2/fs/mounts", serve_mounts)
    await page.route(
        re.compile(r".*/api/webchat/v2/fs/stat(?:\?.*)?$"),
        serve_stat,
    )
    await page.route(
        re.compile(r".*/api/webchat/v2/fs/list(?:\?.*)?$"),
        serve_listing,
    )
    await page.route(
        re.compile(r".*/api/webchat/v2/fs/content(?:\?.*)?$"),
        fail_content_download,
    )

    origin = await page.evaluate("location.origin")
    await page.goto(f"{origin}/v2/workspace/workspace")
    await expect(page.locator(SEL_V2["workspace_heading"])).to_be_visible(timeout=15000)
    report_file = page.locator(
        SEL_V2["workspace_directory_entry_for"].format(path="workspace/report.pdf")
    )
    await expect(report_file).to_be_visible(timeout=5000)
    await report_file.click()

    download_button = page.locator(SEL_V2["workspace_download"])
    await expect(download_button).to_be_visible(timeout=15000)

    await download_button.click()

    failure_toast = page.locator(SEL_V2["toast"]).filter(
        has_text="Couldn't download this file. Please try again."
    )
    await expect(failure_toast).to_be_visible(timeout=5000)


async def test_workspace_deep_link_expands_selected_file_parents(reborn_v2_yolo_page):
    """A nested file deep link reveals its selected node in the Workspace tree."""
    page = reborn_v2_yolo_page
    listings = {
        "": [{"name": "projects", "path": "projects", "kind": "directory"}],
        "projects": [
            {
                "name": "ironclaw",
                "path": "projects/ironclaw",
                "kind": "directory",
            }
        ],
        "projects/ironclaw": [
            {
                "name": "notes",
                "path": "projects/ironclaw/notes",
                "kind": "directory",
            }
        ],
        "projects/ironclaw/notes": [
            {
                "name": "plan",
                "path": "projects/ironclaw/notes/plan",
                "kind": "file",
            }
        ],
    }

    async def serve_mounts(route):
        await route.fulfill(
            content_type="application/json",
            body=json.dumps({"mounts": [{"mount": "workspace"}]}),
        )

    async def serve_listing(route):
        query = parse_qs(urlparse(route.request.url).query)
        path = query.get("path", [""])[0]
        assert path in listings, f"unexpected Workspace listing path: {path}"
        await route.fulfill(
            content_type="application/json",
            body=json.dumps({"mount": "workspace", "entries": listings[path]}),
        )

    async def serve_stat(route):
        await route.fulfill(
            content_type="application/json",
            body=json.dumps(
                {
                    "stat": {
                        "kind": "file",
                        "mime_type": "text/markdown",
                        "size_bytes": 6,
                    }
                }
            ),
        )

    async def serve_content(route):
        await route.fulfill(content_type="text/markdown", body="# Plan")

    await page.route("**/api/webchat/v2/fs/mounts", serve_mounts)
    await page.route(re.compile(r".*/api/webchat/v2/fs/list(?:\?.*)?$"), serve_listing)
    await page.route(re.compile(r".*/api/webchat/v2/fs/stat(?:\?.*)?$"), serve_stat)
    await page.route(
        re.compile(r".*/api/webchat/v2/fs/content(?:\?.*)?$"),
        serve_content,
    )

    origin = await page.evaluate("location.origin")
    await page.goto(
        f"{origin}/workspace/workspace/projects/ironclaw/notes/plan"
    )
    await expect(page.locator(SEL_V2["workspace_heading"])).to_be_visible(timeout=15000)

    for path, label in (
        ("workspace", "Home"),
        ("workspace/projects", "projects"),
        ("workspace/projects/ironclaw", "ironclaw"),
        ("workspace/projects/ironclaw/notes", "notes"),
    ):
        expanded_directory = page.locator(SEL_V2["workspace_tree_entry"]).filter(
            has_text=label
        )
        await expect(expanded_directory).to_have_attribute(
            "data-entry-path", path, timeout=5000
        )
        await expect(expanded_directory).to_have_attribute(
            "aria-expanded", "true", timeout=5000
        )

    selected_file = page.locator(SEL_V2["workspace_tree_entry"]).filter(
        has_text="plan"
    )
    await expect(selected_file).to_have_attribute(
        "data-entry-path", "workspace/projects/ironclaw/notes/plan", timeout=5000
    )
    await expect(selected_file).to_be_visible(timeout=5000)
