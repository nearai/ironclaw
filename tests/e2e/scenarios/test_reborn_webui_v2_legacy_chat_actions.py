"""Reborn WebChat v2 ports of legacy chat action coverage."""

import re

from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture dependency
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture dependency
)


async def test_reborn_legacy_message_copy_button_writes_raw_text(reborn_v2_page):
    """Port of legacy per-message Copy behavior to Reborn's message actions."""
    page = reborn_v2_page
    await page.evaluate(
        """() => {
          window.__copiedText = null;
          Object.defineProperty(navigator, "clipboard", {
            configurable: true,
            value: {
              writeText: (text) => {
                window.__copiedText = text;
                return Promise.resolve();
              },
            },
          });
        }"""
    )

    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill("link test")
    await composer.press("Enter")

    user_message = page.locator(SEL_V2["msg_user"]).last
    assistant_message = page.locator(SEL_V2["msg_assistant"]).last
    await expect(user_message).to_contain_text("link test", timeout=15000)
    await expect(assistant_message).to_contain_text("the pull request", timeout=30000)

    user_copy = user_message.locator(SEL_V2["message_copy_button"]).first
    await user_copy.click(force=True)
    await page.wait_for_function(
        "() => window.__copiedText === 'link test'",
        timeout=5000,
    )
    await expect(user_copy).to_have_attribute("aria-label", "Copied", timeout=5000)
    await expect(user_copy).to_have_attribute("aria-label", "Copy message", timeout=3000)

    assistant_copy = assistant_message.locator(SEL_V2["message_copy_button"]).first
    await assistant_copy.click(force=True)
    await page.wait_for_function(
        """() => window.__copiedText ===
          'See [the pull request](https://example.com/pr/1) for details.'""",
        timeout=5000,
    )


async def test_reborn_legacy_selection_copy_forces_plain_text(reborn_v2_page):
    """Port of legacy selected-chat copy behavior to Reborn's message list."""
    page = reborn_v2_page
    composer = page.locator(SEL_V2["chat_composer"])
    await composer.fill("link test")
    await composer.press("Enter")

    assistant_message = page.locator(SEL_V2["msg_assistant"]).last
    await expect(assistant_message).to_contain_text("the pull request", timeout=30000)

    copied = await page.evaluate(
        """
        (selector) => {
          const content = Array.from(document.querySelectorAll(selector))
            .find((el) => (el.textContent || '').includes('the pull request'));
          if (!content) return { ok: false, reason: 'no content' };
          const range = document.createRange();
          range.selectNodeContents(content);
          const selection = window.getSelection();
          selection.removeAllRanges();
          selection.addRange(range);

          const store = {};
          const event = new Event('copy', { bubbles: true, cancelable: true });
          event.clipboardData = {
            clearData: () => { Object.keys(store).forEach((key) => delete store[key]); },
            setData: (type, value) => { store[type] = value; },
            getData: (type) => store[type] || '',
          };

          content.dispatchEvent(event);
          return {
            ok: true,
            defaultPrevented: event.defaultPrevented,
            text: store['text/plain'] || '',
            html: store['text/html'] || '',
          };
        }
        """,
        SEL_V2["msg_assistant_markdown"],
    )

    assert copied["ok"], copied.get("reason", "copy setup failed")
    assert copied["defaultPrevented"] is True
    assert "the pull request" in copied["text"]
    assert copied["html"] == ""


async def test_reborn_legacy_command_palette_filters_and_navigates(reborn_v2_page):
    """Port the command-discovery affordance to Reborn's command palette."""
    page = reborn_v2_page

    await page.keyboard.press("Control+K")
    palette = page.get_by_role("dialog", name="Command palette")
    await expect(palette).to_be_visible(timeout=5000)
    await expect(palette.get_by_role("button", name="New chat")).to_be_visible()
    await expect(palette.get_by_role("button", name="Go to Extensions")).to_be_visible()
    await expect(palette.get_by_role("button", name="Go to Settings")).to_be_visible()

    search = palette.get_by_placeholder("Type a command or search")
    await search.fill("settings")
    await expect(palette.get_by_role("button", name="Go to Settings")).to_be_visible()
    await expect(palette.get_by_role("button", name="Go to Extensions")).to_have_count(0)

    await search.press("Enter")
    await expect(page).to_have_url(re.compile(r".*/v2/settings.*"), timeout=10000)
