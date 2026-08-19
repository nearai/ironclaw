"""Reborn WebChat v2 ports of legacy rendering-safety browser tests."""

from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture dependency
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture dependency
)


async def test_reborn_legacy_rendering_assistant_html_is_sanitized(reborn_v2_page):
    """Port of legacy assistant HTML-injection sanitization coverage."""
    page = reborn_v2_page
    composer = page.locator(SEL_V2["chat_composer"])

    await composer.fill("html test")
    await composer.press("Enter")

    assistant_msg = page.locator(SEL_V2["msg_assistant"]).last
    await expect(assistant_msg).to_contain_text("content", timeout=30000)

    inner_html = (await assistant_msg.inner_html()).lower()
    assert "<script" not in inner_html
    assert "<iframe" not in inner_html
    # The raw `<img ... onerror=...>` payload is sanitized before it reaches
    # the DOM (marked + DOMPurify on the completed path, streamdown while
    # streaming): the event handler is stripped even though the escaped text
    # still contains the literal `onerror=` substring. The safety contract is
    # that no live element with an event handler is ever created, so assert on
    # DOM nodes instead of the raw string. A bare sanitized `<img>` is
    # allowed.
    assert await assistant_msg.locator("script").count() == 0
    assert await assistant_msg.locator("iframe").count() == 0
    assert await assistant_msg.locator("[onerror]").count() == 0


async def test_reborn_legacy_rendering_user_html_stays_plain_text(reborn_v2_page):
    """Port of legacy user-message HTML escaping coverage."""
    page = reborn_v2_page
    composer = page.locator(SEL_V2["chat_composer"])
    dangerous_input = '<img src=x onerror="alert(1)">'

    await composer.fill(dangerous_input)
    await composer.press("Enter")

    user_msg = page.locator(SEL_V2["msg_user"]).last
    await expect(user_msg).to_contain_text(dangerous_input, timeout=15000)

    inner_html = (await user_msg.inner_html()).lower()
    assert "<img" not in inner_html
    assert "&lt;img" in inner_html


async def test_reborn_legacy_rendering_no_script_dom_nodes(reborn_v2_page):
    """Port of legacy script-node absence coverage."""
    page = reborn_v2_page
    composer = page.locator(SEL_V2["chat_composer"])

    await composer.fill("html injection test")
    await composer.press("Enter")

    await expect(page.locator(SEL_V2["msg_assistant"]).last).to_contain_text(
        "content", timeout=30000
    )
    assert await page.locator(f"{SEL_V2['msg_assistant']} script").count() == 0
