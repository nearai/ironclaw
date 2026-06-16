"""Reborn WebChat v2: agent-produced files are downloadable from the UI.

Drives the real `ironclaw-reborn serve` binary (v2 SPA) under the
`local-dev-yolo` profile — minimal approvals, so an in-workspace `write_file`
auto-proceeds instead of parking on a destructive-write gate. The mock LLM
turns the prompt into two `write_file` tool calls (a CSV and a PDF), then a
reply that references their `/workspace` paths. The SPA renders those paths as
download chips; clicking one performs the bearer-authenticated blob fetch and
saves the file.

Complements `webui_v2_e2e.rs` (in-process, asserts the same endpoints against a
real agent-produced file) by covering the browser chip-render + click-download
integration. Requires the full E2E harness (cargo build + reborn serve + mock
LLM + Chromium); it is CI-run, not exercised by `cargo test`.
"""

import asyncio
import os
import signal
from pathlib import Path

import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2, wait_for_ready

# Reuse the smoke module's process-startup plumbing and parameterized config
# writer rather than duplicating the retry/coverage/teardown boilerplate.
from .test_reborn_webui_v2_smoke import (
    USER_ID,
    _find_free_port,
    _forward_coverage_env,
    _read_log,
    _stop_process,
    _write_config_toml,
    reborn_v2_browser,  # noqa: F401 — imported so pytest resolves the fixture here
)

YOLO_PROFILE = "local-dev-yolo"

CSV_PATH = "/workspace/report.csv"
PDF_PATH = "/workspace/report.pdf"
CSV_BYTES = b"name,score\nalice,90\nbob,85\n"


@pytest.fixture(scope="module")
async def reborn_v2_yolo_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """`ironclaw-reborn serve` on the `local-dev-yolo` profile (auto-approves writes)."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-yolo-home")
    reborn_home = home_dir / "reborn-home"
    reborn_home.mkdir(parents=True, exist_ok=True)
    _write_config_toml(reborn_home / "config.toml", mock_llm_server, profile=YOLO_PROFILE)

    proc = None
    base_url = None
    last_stderr = ""
    last_port = None

    for attempt in range(1, 4):
        port = _find_free_port()
        last_port = port
        stdout_path = home_dir / f"reborn-v2-yolo-attempt-{attempt}.stdout.log"
        stderr_path = home_dir / f"reborn-v2-yolo-attempt-{attempt}.stderr.log"

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home_dir),
            "IRONCLAW_REBORN_HOME": str(reborn_home),
            "IRONCLAW_REBORN_PROFILE": YOLO_PROFILE,
            "IRONCLAW_REBORN_WEBUI_TOKEN": REBORN_V2_AUTH_TOKEN,
            "IRONCLAW_REBORN_WEBUI_USER_ID": USER_ID,
            "MOCK_LLM_API_KEY": "mock-api-key",
            "NO_PROXY": "127.0.0.1,localhost,::1",
            "no_proxy": "127.0.0.1,localhost,::1",
            "RUST_LOG": "ironclaw=warn,ironclaw_reborn=warn",
            "RUST_BACKTRACE": "1",
        }
        _forward_coverage_env(env)

        with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
            proc = await asyncio.create_subprocess_exec(
                ironclaw_reborn_binary,
                "serve",
                "--host", "127.0.0.1",
                "--port", str(port),
                stdin=asyncio.subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                env=env,
            )
        base_url = f"http://127.0.0.1:{port}"

        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            break
        except TimeoutError:
            if proc.returncode is None:
                await _stop_process(proc, timeout=2)
            last_stderr = _read_log(stderr_path)
            proc = None
    else:
        pytest.fail(
            "Reborn WebUI v2 yolo server failed to start after 3 attempts.\n"
            f"Last attempted port: {last_port}\nstderr:\n{last_stderr}"
        )

    try:
        yield base_url
    finally:
        if proc is not None and proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


@pytest.fixture
async def reborn_v2_yolo_page(reborn_v2_yolo_server, reborn_v2_browser):
    """Authed page on the yolo v2 SPA, navigated past login (downloads enabled)."""
    context = await reborn_v2_browser.new_context(
        viewport={"width": 1280, "height": 720}, accept_downloads=True
    )
    page = await context.new_page()
    await page.goto(f"{reborn_v2_yolo_server}/v2/?token={REBORN_V2_AUTH_TOKEN}")
    await page.wait_for_selector(SEL_V2["chat_composer"], timeout=15000)
    yield page
    await context.close()


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
