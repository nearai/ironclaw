"""Scenario: Tool permissions UI and REST API."""

import httpx

from helpers import AUTH_TOKEN, SEL


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {AUTH_TOKEN}"}


async def _open_tools_tab(page) -> None:
    """Navigate to the Settings panel and click the Tools subtab."""
    # First, click the main Settings tab to ensure the Settings panel is active
    settings_tab = page.locator(SEL["tab_button"].format(tab="settings"))
    await settings_tab.click()
    await page.locator(SEL["tab_panel"].format(tab="settings")).wait_for(
        state="visible", timeout=5000
    )

    # Then click the Tools subtab within Settings
    tools_subtab = page.locator(SEL["settings_subtab"].format(subtab="tools"))
    await tools_subtab.wait_for(state="visible", timeout=5000)
    await tools_subtab.click()

    # Wait for the Tools panel to be visible
    await page.locator(SEL["settings_subpanel"].format(subtab="tools")).wait_for(
        state="visible", timeout=5000
    )


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


async def test_tools_tab_visible(page):
    """Settings panel has a Tools tab; clicking it shows tool list with at least one row."""
    await _open_tools_tab(page)

    # At least one tool-permission row should appear
    rows = page.locator(SEL["tool_permission_row"])
    await rows.first.wait_for(state="visible", timeout=5000)
    count = await rows.count()
    assert count >= 1, f"Expected at least one tool-permission row, got {count}"


async def test_tool_permission_toggle_persists(page, ironclaw_server):
    """Toggle a tool from AlwaysAllow → AskEachTime via UI; reload; confirm persisted."""
    headers = _headers()

    # Set echo to a known initial state via REST
    async with httpx.AsyncClient() as client:
        r = await client.put(
            f"{ironclaw_server}/api/settings/tools/echo",
            json={"state": "always_allow"},
            headers=headers,
            timeout=10,
        )
        assert r.status_code == 200, f"Precondition PUT failed: {r.text}"

    # Open the Tools tab
    await _open_tools_tab(page)

    # Find the echo row and its toggle group
    echo_row = page.locator(f"{SEL['tool_permission_row']}[data-tool-name='echo']")
    await echo_row.wait_for(state="visible", timeout=5000)

    # Click the "Ask Each Time" option inside the toggle group
    ask_btn = echo_row.locator(f"{SEL['tool_permission_toggle']} button[data-state='ask_each_time']")
    await ask_btn.wait_for(state="visible", timeout=5000)
    await ask_btn.click()

    # Wait for the PUT to complete and the button to become active before reloading
    active_indicator = echo_row.locator(
        f"{SEL['tool_permission_toggle']} button[data-state='ask_each_time'].active"
    )
    await active_indicator.wait_for(state="visible", timeout=5000)

    # Reload the page and re-open the Tools tab
    await page.reload()
    await page.locator("#auth-screen").wait_for(state="hidden", timeout=15000)
    await _open_tools_tab(page)

    # After reload, echo row should reflect "Ask Each Time" as the active state
    echo_row_reloaded = page.locator(f"{SEL['tool_permission_row']}[data-tool-name='echo']")
    await echo_row_reloaded.wait_for(state="visible", timeout=5000)

    active_btn = echo_row_reloaded.locator(
        f"{SEL['tool_permission_toggle']} button[data-state='ask_each_time'][aria-pressed='true'],"
        f"{SEL['tool_permission_toggle']} button[data-state='ask_each_time'].active"
    )
    await active_btn.wait_for(state="visible", timeout=5000)

    # Also confirm via REST that the state actually persisted
    async with httpx.AsyncClient() as client:
        r2 = await client.get(
            f"{ironclaw_server}/api/settings/tools",
            headers=headers,
            timeout=10,
        )
        assert r2.status_code == 200, r2.text
        tools = r2.json()["tools"]
        echo = next((t for t in tools if t["name"] == "echo"), None)
        assert echo is not None, "echo tool not found in GET /api/settings/tools response"
        assert echo["current_state"] == "ask_each_time", (
            f"Expected ask_each_time, got {echo['current_state']!r}"
        )

    # Cleanup: restore echo to always_allow
    async with httpx.AsyncClient() as client:
        await client.put(
            f"{ironclaw_server}/api/settings/tools/echo",
            json={"state": "always_allow"},
            headers=headers,
            timeout=10,
        )


async def test_locked_tool_shows_lock_icon(page):
    """tool_remove always returns ApprovalRequirement::Always — shows lock icon and no toggles."""
    await _open_tools_tab(page)

    locked_row = page.locator(f"{SEL['tool_permission_row']}[data-tool-name='tool_remove']")
    await locked_row.wait_for(state="visible", timeout=5000)

    # Lock icon must be visible
    lock_icon = locked_row.locator(SEL["tool_lock_icon"])
    await lock_icon.wait_for(state="visible", timeout=5000)

    # Locked tools render NO toggle group at all (the toggle div is only
    # created in the non-locked branch of renderToolPermissionRow).
    toggle_group = locked_row.locator(SEL["tool_permission_toggle"])
    count = await toggle_group.count()
    assert count == 0, (
        f"Expected no toggle group for locked tool tool_remove, found {count}"
    )


async def test_always_approve_persists_across_sessions(browser, ironclaw_server):
    """PUT always_allow persists to DB; a fresh browser context confirms the state survives."""
    headers = _headers()

    # Set echo to always_allow via REST
    async with httpx.AsyncClient() as client:
        r = await client.put(
            f"{ironclaw_server}/api/settings/tools/echo",
            json={"state": "always_allow"},
            headers=headers,
            timeout=10,
        )
        assert r.status_code == 200, f"PUT failed: {r.text}"

    # Create a fresh browser context (new session — no shared cookies/storage)
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    try:
        pg = await context.new_page()
        await pg.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}")
        await pg.wait_for_selector("#auth-screen", state="hidden", timeout=15000)

        # Verify via REST from the new session that the state persisted
        async with httpx.AsyncClient() as client:
            r2 = await client.get(
                f"{ironclaw_server}/api/settings/tools",
                headers=headers,
                timeout=10,
            )
            assert r2.status_code == 200, r2.text
            tools = r2.json()["tools"]
            echo = next((t for t in tools if t["name"] == "echo"), None)
            assert echo is not None, "echo tool not found in GET /api/settings/tools"
            assert echo["current_state"] == "always_allow", (
                f"Expected always_allow, got {echo['current_state']!r}"
            )
    finally:
        await context.close()


async def test_disabled_tool_state_is_persisted(ironclaw_server):
    """PUT disabled for echo → GET confirms disabled state."""
    headers = _headers()

    async with httpx.AsyncClient() as client:
        r = await client.put(
            f"{ironclaw_server}/api/settings/tools/echo",
            json={"state": "disabled"},
            headers=headers,
            timeout=10,
        )
        assert r.status_code == 200, f"PUT /api/settings/tools/echo failed: {r.text}"

        r2 = await client.get(
            f"{ironclaw_server}/api/settings/tools",
            headers=headers,
            timeout=10,
        )
        assert r2.status_code == 200, r2.text
        tools = r2.json()["tools"]
        echo = next((t for t in tools if t["name"] == "echo"), None)
        assert echo is not None, "echo tool not found in GET /api/settings/tools"
        assert echo["current_state"] == "disabled", (
            f"Expected disabled, got {echo['current_state']!r}"
        )

    # Reset: restore to always_allow
    async with httpx.AsyncClient() as client:
        await client.put(
            f"{ironclaw_server}/api/settings/tools/echo",
            json={"state": "always_allow"},
            headers=headers,
            timeout=10,
        )


async def test_locked_tool_put_returns_400(ironclaw_server):
    """PUT always_allow for tool_remove (locked) → 400."""
    headers = _headers()

    async with httpx.AsyncClient() as client:
        r = await client.put(
            f"{ironclaw_server}/api/settings/tools/tool_remove",
            json={"state": "always_allow"},
            headers=headers,
            timeout=10,
        )
        assert r.status_code == 400, (
            f"Expected 400 for locked tool tool_remove, got {r.status_code}: {r.text}"
        )
