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

from pathlib import Path

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
