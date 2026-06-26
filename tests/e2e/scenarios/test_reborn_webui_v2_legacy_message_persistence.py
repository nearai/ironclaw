"""Legacy message-persistence activity-card checks ported to Reborn WebUI v2."""

from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture dependency
    reborn_v2_yolo_page,  # noqa: F401 - imported fixture
    reborn_v2_yolo_server,  # noqa: F401 - imported fixture dependency
)


async def _expand_if_needed(toggle):
    if await toggle.get_attribute("aria-expanded") != "true":
        await toggle.click()


async def test_reborn_legacy_tool_activity_cards_expand_after_reload(
    reborn_v2_yolo_page,
):
    """Port of legacy tool-call history card rendering to Reborn timeline records."""
    marker = "reborn activity persistence marker 8157"
    composer = reborn_v2_yolo_page.locator(SEL_V2["chat_composer"])

    await composer.fill(f"reborn builtin echo {marker}")
    await composer.press("Enter")

    await expect(
        reborn_v2_yolo_page.locator(SEL_V2["msg_assistant"]).filter(has_text=marker)
    ).to_be_visible(timeout=45000)
    await expect(
        reborn_v2_yolo_page.locator(SEL_V2["activity_run"]).first
    ).to_be_visible(timeout=15000)

    await reborn_v2_yolo_page.reload(wait_until="load")
    await expect(composer).to_be_visible(timeout=15000)
    await expect(
        reborn_v2_yolo_page.locator(SEL_V2["msg_assistant"]).filter(has_text=marker)
    ).to_be_visible(timeout=15000)

    activity_run = reborn_v2_yolo_page.locator(SEL_V2["activity_run"]).first
    await expect(activity_run).to_be_visible(timeout=15000)
    await _expand_if_needed(activity_run.locator(SEL_V2["activity_run_toggle"]))

    echo_card = activity_run.locator(
        SEL_V2["tool_activity_card_for"].format(name="echo")
    ).first
    await expect(echo_card).to_be_visible(timeout=5000)
    assert await echo_card.get_attribute("data-tool-status") == "success"

    await _expand_if_needed(echo_card.locator(SEL_V2["tool_activity_toggle"]))
    detail = echo_card.locator(SEL_V2["tool_activity_detail"])
    await expect(detail).to_be_visible(timeout=5000)
    await expect(detail).to_contain_text(marker)
