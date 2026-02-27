"""Scenario 1: Connection, auth, and tab navigation."""

import pytest
from helpers import AUTH_TOKEN, SEL, TABS


async def test_page_loads_and_connects(page):
    """After auth, the app shows Connected status and all tabs."""
    # Connection status
    status = page.locator(SEL["sse_status"])
    await status.wait_for(state="visible", timeout=10000)
    text = await status.text_content()
    assert text is not None
    assert "connect" in text.lower(), f"Expected 'Connected', got '{text}'"

    # All 6 main tabs visible
    for tab in TABS:
        btn = page.locator(SEL["tab_button"].format(tab=tab))
        assert await btn.is_visible(), f"Tab button '{tab}' not visible"


async def test_tab_navigation(page):
    """Clicking each tab shows its panel."""
    for tab in TABS:
        btn = page.locator(SEL["tab_button"].format(tab=tab))
        await btn.click()
        panel = page.locator(SEL["tab_panel"].format(tab=tab))
        await panel.wait_for(state="visible", timeout=5000)

    # Return to Chat tab
    await page.locator(SEL["tab_button"].format(tab="chat")).click()
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)


async def test_auth_rejection(page, ironclaw_server):
    """Navigating without a token shows the auth screen."""
    # Open a new page without the token
    new_page = await page.context.new_page()
    await new_page.goto(ironclaw_server)
    auth_screen = new_page.locator(SEL["auth_screen"])
    await auth_screen.wait_for(state="visible", timeout=10000)
    await new_page.close()
