"""Scenario: Command palette keyboard accessibility and thread metadata rendering."""

import json

from helpers import SEL


async def _open_command_palette(page) -> None:
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)
    await chat_input.click()

    await page.evaluate(
        """() => {
            document.dispatchEvent(
                new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true })
            );
        }"""
    )

    overlay = page.locator(SEL["palette_overlay"])
    await overlay.wait_for(state="visible", timeout=5000)
    await page.wait_for_function(
        "() => document.activeElement && document.activeElement.id === 'palette-input'",
        timeout=5000,
    )


async def test_command_palette_traps_tab_focus_and_renders_channel_once(page):
    """Tab should cycle within the palette, and thread channel badges should not duplicate description text.

    Covers the production Mod+K open path plus the palette's own keyboard handler.
    A regression here would reopen the original review issue where Tab was trapped
    without any actual focus movement.
    """

    async def patch_threads(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "threads": [
                        {
                            "id": "thread-http-1",
                            "title": "Remote HTTP Thread",
                            "channel": "http",
                            "updated_at": "2026-04-11T11:00:00Z",
                        }
                    ]
                }
            ),
        )

    await page.route("**/api/chat/threads", patch_threads)
    await _open_command_palette(page)

    await page.wait_for_function(
        """() => {
            const item = document.querySelector('#palette-results .command-palette-item');
            if (!item) return false;
            const badge = item.querySelector('.command-palette-item-badge');
            const desc = item.querySelector('.command-palette-item-desc');
            return !!badge && badge.textContent.trim() === 'http' && !desc;
        }""",
        timeout=10000,
    )

    palette_input = page.locator(SEL["palette_input"])
    palette_items = page.locator(SEL["palette_item"])
    await palette_items.first.wait_for(state="visible", timeout=5000)

    result_count = await palette_items.count()
    assert result_count >= 1, "Expected at least one command palette result"

    await palette_input.press("Tab")
    await page.wait_for_function(
        "() => document.activeElement && document.activeElement.id === 'palette-item-0'",
        timeout=5000,
    )

    for _ in range(result_count):
        await page.keyboard.press("Tab")

    await page.wait_for_function(
        "() => document.activeElement && document.activeElement.id === 'palette-input'",
        timeout=5000,
    )
