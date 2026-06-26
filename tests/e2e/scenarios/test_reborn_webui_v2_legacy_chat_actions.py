"""Reborn WebChat v2 ports of legacy chat action coverage."""

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

    user_copy = user_message.locator("button[title]").first
    await user_copy.click(force=True)
    await page.wait_for_function(
        "() => window.__copiedText === 'link test'",
        timeout=5000,
    )
    await expect(user_copy).to_have_attribute("aria-label", "Copied", timeout=5000)
    await expect(user_copy).to_have_attribute("aria-label", "Copy message", timeout=3000)

    assistant_copy = assistant_message.locator("button[title]").first
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
        () => {
          const content = Array.from(document.querySelectorAll(
            '[data-testid="msg-assistant"] .markdown-body'
          )).find((el) => (el.textContent || '').includes('the pull request'));
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
        """
    )

    assert copied["ok"], copied.get("reason", "copy setup failed")
    assert copied["defaultPrevented"] is True
    assert "the pull request" in copied["text"]
    assert copied["html"] == ""
