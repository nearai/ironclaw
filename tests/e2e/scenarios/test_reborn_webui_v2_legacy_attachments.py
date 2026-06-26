"""Reborn WebChat v2 ports of legacy browser attachment coverage."""

import base64

from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture dependency
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture dependency
)

ATTACHMENT_MARKER = "IRONCLAW_ATTACHMENT_MARKER_4644"
ONE_BY_ONE_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO7Z0QAAAABJRU5ErkJggg=="
)


async def test_reborn_legacy_attachment_flow_renders_and_reaches_model(
    reborn_v2_page,
):
    """Stage browser files, render cards, and prove text extraction reaches the LLM."""
    page = reborn_v2_page
    await page.set_input_files(
        "input[type=file][multiple]",
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
