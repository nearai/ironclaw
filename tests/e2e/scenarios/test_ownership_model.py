"""Ownership model E2E tests.

Verifies:
- Bootstrap creates the owner user on startup (single-tenant).
- Owner identity is stable across requests (settings scoped correctly).
- Regression: existing owner-scope behaviour is preserved.
"""

import uuid

import httpx

from helpers import AUTH_TOKEN


def _headers():
    return {"Authorization": f"Bearer {AUTH_TOKEN}"}


# ---------------------------------------------------------------------------
# Bootstrap
# ---------------------------------------------------------------------------


async def test_server_starts_and_health_ok(ironclaw_server):
    """Server starts cleanly after bootstrap_ownership runs."""
    async with httpx.AsyncClient() as client:
        r = await client.get(f"{ironclaw_server}/api/health", timeout=10)
    assert r.status_code == 200


async def test_settings_written_and_readable(ironclaw_server):
    """Settings written by the owner are readable in the next request — scope stable."""
    key = f"e2e_ownership_{uuid.uuid4().hex[:8]}"

    async with httpx.AsyncClient() as client:
        w = await client.put(
            f"{ironclaw_server}/api/settings/{key}",
            json={"value": "ownership_ok"},
            headers=_headers(),
            timeout=10,
        )
    # Accept 200, 201, or 204
    assert w.status_code in (200, 201, 204), f"Write failed: {w.status_code} {w.text[:200]}"

    async with httpx.AsyncClient() as client:
        r = await client.get(
            f"{ironclaw_server}/api/settings/{key}",
            headers=_headers(),
            timeout=10,
        )
    assert r.status_code == 200
    assert "ownership_ok" in str(r.json()), f"Expected ownership_ok in: {r.json()}"


async def test_unauthenticated_cannot_read_settings(ironclaw_server):
    """Unauthenticated requests cannot read owner-scoped settings."""
    async with httpx.AsyncClient() as client:
        r = await client.get(
            f"{ironclaw_server}/api/settings/e2e_ownership_test",
            timeout=10,
        )
    assert r.status_code in (401, 403), f"Expected auth rejection, got {r.status_code}"


# ---------------------------------------------------------------------------
# Browser / Playwright tests — verify ownership model through the web UI
# ---------------------------------------------------------------------------


async def test_owner_can_login_and_see_chat_ui(page, ironclaw_server):
    """Owner navigates to the app, passes auth, sees the chat interface."""
    from helpers import SEL

    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=10000)
    assert await chat_input.is_visible(), "Chat input should be visible for authenticated owner"


async def test_unauthenticated_browser_sees_auth_screen(ironclaw_server, browser):
    """Browser without a valid token sees the auth screen, not the chat UI."""
    from helpers import SEL

    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    # Navigate WITHOUT a token
    await page.goto(ironclaw_server, wait_until="networkidle", timeout=15000)
    auth_screen = page.locator(SEL["auth_screen"])
    await auth_screen.wait_for(state="visible", timeout=10000)
    assert await auth_screen.is_visible(), "Auth screen should be visible without token"
    await context.close()


async def test_owner_can_send_message_and_get_response(page, ironclaw_server):
    """Owner sends a message via the browser UI and gets a mock LLM response."""
    from helpers import SEL

    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=10000)

    # Count existing assistant messages
    assistant_msgs = page.locator(SEL["message_assistant"])
    before_count = await assistant_msgs.count()

    # Send a message
    await chat_input.fill("hello ownership test")
    await chat_input.press("Enter")

    # Wait for a new assistant message to appear
    await page.wait_for_function(
        f"document.querySelectorAll('{SEL['message_assistant']}').length > {before_count}",
        timeout=30000,
    )
    after_count = await assistant_msgs.count()
    assert after_count > before_count, "Should have received at least one assistant response"


async def test_owner_settings_tab_renders(page, ironclaw_server):
    """Owner can navigate to the settings tab without errors."""
    from helpers import SEL

    tab_btn = page.locator(SEL["tab_button"].format(tab="settings"))
    await tab_btn.wait_for(state="visible", timeout=5000)
    await tab_btn.click()

    # Wait briefly for the panel to render
    settings_panel = page.locator(SEL["tab_panel"].format(tab="settings"))
    await settings_panel.wait_for(state="visible", timeout=5000)
    assert await settings_panel.is_visible(), "Settings panel should be visible after clicking the tab"


async def test_page_title_and_basic_structure(page, ironclaw_server):
    """The page loads with expected basic structure for an authenticated owner."""
    from helpers import SEL

    # Auth screen should be hidden (page fixture waits for this)
    auth_screen = page.locator(SEL["auth_screen"])
    assert not await auth_screen.is_visible(), "Auth screen should be hidden after login"

    # Chat input should be present
    chat_input = page.locator(SEL["chat_input"])
    assert await chat_input.is_visible(), "Chat input should be visible"
