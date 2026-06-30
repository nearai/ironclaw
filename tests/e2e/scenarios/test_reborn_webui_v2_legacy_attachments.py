"""Reborn WebChat v2 ports of legacy browser attachment coverage."""

import asyncio
import base64
import io
import json
import zipfile
from pathlib import Path

import httpx
from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture dependency
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture dependency
    reborn_v2_vision_page,  # noqa: F401 - imported fixture
    reborn_v2_vision_server,  # noqa: F401 - imported fixture dependency
)

ROOT = Path(__file__).resolve().parents[3]
HELLO_PDF = ROOT / "tests" / "fixtures" / "hello.pdf"
ATTACHMENT_MARKER = "IRONCLAW_ATTACHMENT_MARKER_4644"
ONE_BY_ONE_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO7Z0QAAAABJRU5ErkJggg=="
)


def _make_test_pptx(slide_text: str) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as archive:
        archive.writestr(
            "ppt/slides/slide1.xml",
            f"""<?xml version="1.0" encoding="UTF-8"?>
            <p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                   xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
              <p:cSld>
                <p:spTree>
                  <p:sp>
                    <p:txBody>
                      <a:p><a:r><a:t>{slide_text}</a:t></a:r></a:p>
                    </p:txBody>
                  </p:sp>
                </p:sp>
              </p:cSld>
            </p:sld>""",
        )
    return buf.getvalue()


async def _wait_for_mock_llm_request_contains(
    mock_llm_url: str, needles: list[str], *, timeout: float = 45.0
) -> dict:
    last_payload = {}
    async with httpx.AsyncClient() as client:
        for _ in range(int(timeout * 2)):
            response = await client.get(
                f"{mock_llm_url}/__mock/last_chat_request",
                timeout=15,
            )
            response.raise_for_status()
            payload = response.json()
            last_payload = payload
            haystack = json.dumps(payload).lower()
            if all(needle.lower() in haystack for needle in needles):
                return payload
            await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for mock LLM request containing {needles!r}. "
        f"Last payload: {json.dumps(last_payload)[:1200]}"
    )


async def test_reborn_legacy_attachment_flow_renders_and_reaches_model(
    reborn_v2_page,
):
    """Stage browser files, render cards, and prove text extraction reaches the LLM."""
    page = reborn_v2_page
    await page.set_input_files(
        SEL_V2["chat_file_input"],
        files=[
            {
                "name": "marker.txt",
                "mimeType": "text/plain",
                "buffer": f"Internal report. {ATTACHMENT_MARKER}. End.".encode(),
            },
            {
                "name": "tiny.png",
                "mimeType": "image/png",
                "buffer": ONE_BY_ONE_PNG,
            },
        ],
    )

    await expect(page.get_by_text("marker.txt").first).to_be_visible(timeout=15000)
    await expect(page.get_by_text("tiny.png").first).to_be_visible(timeout=15000)

    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill("summarize the attached document")
    await composer.press("Enter")

    user_message = page.locator(SEL_V2["msg_user"]).last
    await expect(user_message).to_contain_text(
        "summarize the attached document", timeout=15000
    )
    await expect(user_message).to_contain_text("marker.txt", timeout=15000)
    await expect(user_message).to_contain_text("tiny.png", timeout=15000)
    await expect(page.locator(SEL_V2["msg_assistant"]).last).to_contain_text(
        "I can read the attached document text.", timeout=45000
    )

    await page.reload(wait_until="domcontentloaded")
    reloaded_user_message = page.locator(SEL_V2["msg_user"]).filter(has_text="marker.txt").last
    await expect(reloaded_user_message).to_contain_text("marker.txt", timeout=15000)
    await expect(reloaded_user_message).to_contain_text("tiny.png", timeout=15000)


async def test_reborn_legacy_attachment_document_extraction_reaches_model(
    reborn_v2_vision_page,
    mock_llm_server,
):
    """Port of legacy PDF/text/PPTX/image model-payload attachment assertions."""
    page = reborn_v2_vision_page
    await page.set_input_files(
        SEL_V2["chat_file_input"],
        files=[
            {
                "name": "tiny.png",
                "mimeType": "image/png",
                "buffer": ONE_BY_ONE_PNG,
            },
            {
                "name": "hello.pdf",
                "mimeType": "application/pdf",
                "buffer": HELLO_PDF.read_bytes(),
            },
            {
                "name": "notes.txt",
                "mimeType": "text/plain",
                "buffer": b"Quarterly roadmap notes\nShip the Reborn attachment flow.",
            },
            {
                "name": "roadmap.pptx",
                "mimeType": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                "buffer": _make_test_pptx("Reborn attachment roadmap slide"),
            },
        ],
    )

    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill("Please review these Reborn attachments.")
    await composer.press("Enter")

    user_message = page.locator(SEL_V2["msg_user"]).last
    await expect(user_message).to_contain_text(
        "Please review these Reborn attachments.", timeout=15000
    )
    for filename in ("tiny.png", "hello.pdf", "notes.txt", "roadmap.pptx"):
        await expect(user_message).to_contain_text(filename, timeout=15000)

    payload = await _wait_for_mock_llm_request_contains(
        mock_llm_server,
        [
            "Please review these Reborn attachments.",
            "hello.pdf",
            "Quarterly roadmap notes",
            "Ship the Reborn attachment flow.",
            "Reborn attachment roadmap slide",
        ],
        timeout=45.0,
    )
    serialized = json.dumps(payload)
    assert "data:image/png;base64," in serialized, serialized[:1200]


async def test_reborn_legacy_unextractable_attachment_uses_placeholder(
    reborn_v2_page,
    mock_llm_server,
):
    """Port of corrupt-PDF extraction fallback to Reborn's attachment marker."""
    page = reborn_v2_page
    corrupt_pdf = b"%PDF-1.4\n<<garbage>> not a real pdf body \x00\x01\x02"

    await page.set_input_files(
        SEL_V2["chat_file_input"],
        files=[
            {
                "name": "mystery.pdf",
                "mimeType": "application/pdf",
                "buffer": corrupt_pdf,
            }
        ],
    )

    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill("Please inspect this Reborn binary attachment.")
    await composer.press("Enter")

    user_message = page.locator(SEL_V2["msg_user"]).last
    await expect(user_message).to_contain_text(
        "Please inspect this Reborn binary attachment.", timeout=15000
    )
    await expect(user_message).to_contain_text("mystery.pdf", timeout=15000)

    payload = await _wait_for_mock_llm_request_contains(
        mock_llm_server,
        [
            "Please inspect this Reborn binary attachment.",
            "mystery.pdf",
            "text extraction unavailable",
        ],
        timeout=45.0,
    )
    serialized = json.dumps(payload)
    assert "failed to extract text" not in serialized.lower(), serialized[:1200]


async def test_reborn_legacy_files_only_attachments_reload_from_history(
    reborn_v2_page,
):
    """Port of legacy files-only attachment send and history re-render coverage."""
    page = reborn_v2_page
    await page.set_input_files(
        SEL_V2["chat_file_input"],
        files=[
            {
                "name": "files-only.pdf",
                "mimeType": "application/pdf",
                "buffer": HELLO_PDF.read_bytes(),
            },
            {
                "name": "files-only-notes.txt",
                "mimeType": "text/plain",
                "buffer": b"Files-only attachment note.",
            },
        ],
    )

    await expect(page.get_by_text("files-only.pdf")).to_be_visible(timeout=15000)
    await expect(page.get_by_text("files-only-notes.txt")).to_be_visible(timeout=15000)
    await page.get_by_label("Send").click()

    user_message = page.locator(SEL_V2["msg_user"]).filter(
        has_text="files-only-notes.txt"
    ).last
    await expect(user_message).to_contain_text("files-only.pdf", timeout=15000)
    await expect(user_message).to_contain_text("files-only-notes.txt", timeout=15000)
    await expect(user_message).not_to_contain_text("(files attached)")
    await expect(user_message).not_to_contain_text("<attachments>")
    await expect(page.locator(SEL_V2["msg_assistant"]).last).to_be_visible(
        timeout=45000
    )

    await page.reload(wait_until="domcontentloaded")
    reloaded_user_message = page.locator(SEL_V2["msg_user"]).filter(
        has_text="files-only-notes.txt"
    ).last
    await expect(reloaded_user_message).to_contain_text(
        "files-only.pdf", timeout=15000
    )
    await expect(reloaded_user_message).to_contain_text(
        "files-only-notes.txt", timeout=15000
    )
    await expect(reloaded_user_message).not_to_contain_text("(files attached)")
    await expect(reloaded_user_message).not_to_contain_text("<attachments>")


async def test_reborn_legacy_attachment_count_limit_blocks_extra_files(
    reborn_v2_page,
):
    """Port of legacy batch count validation to Reborn's staging alert UI."""
    page = reborn_v2_page
    await page.set_input_files(
        SEL_V2["chat_file_input"],
        files=[
            {
                "name": f"limit-{index}.txt",
                "mimeType": "text/plain",
                "buffer": f"file {index}".encode(),
            }
            for index in range(1, 12)
        ],
    )

    await expect(page.get_by_text("limit-10.txt")).to_be_visible(timeout=15000)
    await expect(page.get_by_text("limit-11.txt")).to_have_count(0, timeout=5000)
    await expect(page.get_by_role("alert")).to_contain_text(
        "at most 10 files", timeout=15000
    )


async def test_reborn_legacy_attachment_size_limits_block_invalid_files(
    reborn_v2_page,
):
    """Port of legacy per-file and total attachment budget validation."""
    page = reborn_v2_page

    await page.set_input_files(
        SEL_V2["chat_file_input"],
        files=[
            {
                "name": "too-large.txt",
                "mimeType": "text/plain",
                "buffer": b"x" * (6 * 1024 * 1024),
            }
        ],
    )
    await expect(page.get_by_role("alert")).to_contain_text(
        "max 5 MB per file", timeout=15000
    )
    await expect(page.get_by_label("Remove attachment")).to_have_count(0, timeout=5000)

    await page.get_by_label("Dismiss").click()
    await page.set_input_files(
        SEL_V2["chat_file_input"],
        files=[
            {
                "name": f"chunk-{index}.txt",
                "mimeType": "text/plain",
                "buffer": b"x" * (4 * 1024 * 1024),
            }
            for index in range(1, 4)
        ],
    )
    await expect(page.get_by_text("chunk-1.txt")).to_be_visible(timeout=15000)
    await expect(page.get_by_text("chunk-2.txt")).to_be_visible(timeout=15000)
    await expect(page.get_by_text("chunk-3.txt")).to_have_count(0, timeout=5000)
    await expect(page.get_by_label("Remove attachment")).to_have_count(2, timeout=5000)
    await expect(page.get_by_role("alert")).to_contain_text(
        "10 MB total limit", timeout=15000
    )


async def test_reborn_legacy_attachment_unsupported_type_is_rejected(
    reborn_v2_page,
):
    """Port of legacy attachment type rejection through Reborn's accept contract."""
    page = reborn_v2_page
    await page.set_input_files(
        SEL_V2["chat_file_input"],
        files=[
            {
                "name": "unsupported.bin",
                "mimeType": "application/octet-stream",
                "buffer": b"binary",
            }
        ],
    )

    await expect(page.get_by_role("alert")).to_contain_text(
        "unsupported.bin is not a supported file type", timeout=15000
    )
    await expect(page.get_by_label("Remove attachment")).to_have_count(0, timeout=5000)
