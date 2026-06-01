"""Scenario: Settings search filters items across all section types.

Covers:
  - Tool permission rows (Tools subtab)
  - Extension cards (Extensions subtab, via mocked API)
  - User table rows (Users subtab)
  - Empty state when no results match
  - Clearing search restores all items
"""

import asyncio
import json
import uuid

from helpers import SEL, api_get, api_post


# ─── Helpers ──────────────────────────────────────────────────────────────────


async def _open_settings_subtab(page, subtab: str) -> None:
    """Navigate to Settings and open the given subtab."""
    settings_tab = page.locator(SEL["tab_button"].format(tab="settings"))
    await settings_tab.click()
    await page.locator(SEL["tab_panel"].format(tab="settings")).wait_for(
        state="visible", timeout=5000
    )
    st = page.locator(SEL["settings_subtab"].format(subtab=subtab))
    await st.wait_for(state="visible", timeout=5000)
    await st.click()
    await page.locator(SEL["settings_subpanel"].format(subtab=subtab)).wait_for(
        state="visible", timeout=5000
    )


async def _type_search(page, query: str) -> None:
    """Type into the settings search input."""
    search = page.locator(SEL["settings_search_input"])
    await search.fill(query)


# ─── Mock data for extensions ─────────────────────────────────────────────────

_EXT_ALPHA = {
    "name": "alpha-tool",
    "display_name": "Alpha Tool",
    "kind": "wasm_tool",
    "description": "First test extension",
    "url": None,
    "active": True,
    "authenticated": True,
    "has_auth": False,
    "needs_setup": False,
    "tools": ["alpha_search"],
    "activation_status": None,
    "activation_error": None,
}

_EXT_BETA = {
    "name": "beta-widget",
    "display_name": "Beta Widget",
    "kind": "wasm_tool",
    "description": "Second test extension",
    "url": None,
    "active": True,
    "authenticated": True,
    "has_auth": False,
    "needs_setup": False,
    "tools": ["beta_fetch"],
    "activation_status": None,
    "activation_error": None,
}


async def _mock_extension_apis(page):
    """Intercept extension APIs to return deterministic card data."""
    extensions_body = json.dumps({"extensions": [_EXT_ALPHA, _EXT_BETA]})
    registry_body = json.dumps({"entries": []})

    async def handler(route):
        url = route.request.url.split("?")[0]
        if url.endswith("/api/extensions/registry"):
            await route.fulfill(status=200, content_type="application/json", body=registry_body)
        elif url.endswith("/api/extensions"):
            await route.fulfill(status=200, content_type="application/json", body=extensions_body)
        else:
            await route.continue_()

    await page.route("**/api/extensions**", handler)


# ─── Tests ────────────────────────────────────────────────────────────────────


async def test_search_filters_tool_rows(page):
    """Typing in search hides non-matching tool-permission-row elements."""
    await _open_settings_subtab(page, "tools")

    rows = page.locator(SEL["tool_permission_row"])
    await rows.first.wait_for(state="visible", timeout=5000)
    total = await rows.count()
    assert total >= 2, f"Need at least 2 tool rows for this test, got {total}"

    # Search for "echo" — should match the echo tool row
    await _type_search(page, "echo")

    visible = page.locator(f"{SEL['tool_permission_row']}:not(.search-hidden)")
    await visible.first.wait_for(state="visible", timeout=5000)
    visible_count = await visible.count()
    assert visible_count >= 1, "Expected at least one visible tool row for 'echo'"
    assert visible_count < total, "Search should have hidden some rows"

    # The visible row should contain "echo"
    first_text = await visible.first.text_content()
    assert "echo" in first_text.lower(), f"Visible row should contain 'echo', got: {first_text}"


async def test_search_shows_empty_state(page):
    """Searching for a non-existent term shows the empty-state message."""
    await _open_settings_subtab(page, "tools")

    rows = page.locator(SEL["tool_permission_row"])
    await rows.first.wait_for(state="visible", timeout=5000)

    await _type_search(page, "zzz_nonexistent_tool_xyz")

    empty = page.locator(SEL["settings_search_empty"])
    await empty.wait_for(state="visible", timeout=5000)


async def test_search_clear_restores_all(page):
    """Clearing the search input makes all items visible again."""
    await _open_settings_subtab(page, "tools")

    rows = page.locator(SEL["tool_permission_row"])
    await rows.first.wait_for(state="visible", timeout=5000)
    total = await rows.count()

    # Search to hide some rows
    await _type_search(page, "echo")
    visible = page.locator(f"{SEL['tool_permission_row']}:not(.search-hidden)")
    await visible.first.wait_for(state="visible", timeout=5000)
    assert await visible.count() < total

    # Clear search
    await _type_search(page, "")

    restored = page.locator(f"{SEL['tool_permission_row']}:not(.search-hidden)")
    await restored.first.wait_for(state="visible", timeout=5000)
    assert await restored.count() == total, "All rows should be visible after clearing search"

    # Empty state should be gone
    empty = page.locator(SEL["settings_search_empty"])
    assert await empty.count() == 0


async def test_search_filters_extension_cards(page):
    """Search filters ext-card elements in the Extensions subtab."""
    await _mock_extension_apis(page)
    await _open_settings_subtab(page, "extensions")

    cards = page.locator(SEL["ext_card_installed"])
    await cards.first.wait_for(state="visible", timeout=5000)
    total = await cards.count()
    assert total == 2, f"Expected 2 installed extension cards, got {total}"

    # Search for "Alpha" — should show only the Alpha card
    await _type_search(page, "Alpha")

    visible = page.locator(f"#extensions-list .ext-card:not(.search-hidden)")
    await visible.first.wait_for(state="visible", timeout=5000)
    assert await visible.count() == 1

    name_text = await visible.first.locator(SEL["ext_name"]).text_content()
    assert "Alpha" in name_text

    # "Beta" card should be hidden
    hidden = page.locator(f"#extensions-list .ext-card.search-hidden")
    assert await hidden.count() == 1


async def test_search_filters_user_rows(page, ironclaw_server):
    """Search filters user table rows in the Users subtab."""
    # Seed two users via the admin API
    suffix = uuid.uuid4().hex[:8]
    alice = await api_post(ironclaw_server, "/api/admin/users", json={
        "display_name": f"Alice Searchtest {suffix}",
        "email": f"alice-searchtest-{suffix}@example.test",
        "role": "member",
    })
    assert alice.status_code == 200, alice.text
    bob = await api_post(ironclaw_server, "/api/admin/users", json={
        "display_name": f"Bob Searchtest {suffix}",
        "email": f"bob-searchtest-{suffix}@example.test",
        "role": "member",
    })
    assert bob.status_code == 200, bob.text

    expected_ids = {alice.json()["id"], bob.json()["id"]}
    for _ in range(20):
        listed = await api_get(ironclaw_server, "/api/admin/users")
        assert listed.status_code == 200, listed.text
        listed_ids = {u["id"] for u in listed.json().get("users", [])}
        if expected_ids <= listed_ids:
            break
        await asyncio.sleep(0.25)
    else:
        raise AssertionError("Seeded users did not appear in admin API list")

    await _open_settings_subtab(page, "users")

    rows = page.locator(SEL["users_tbody_row"])
    await rows.first.wait_for(state="visible", timeout=15000)
    total = await rows.count()
    assert total >= 2, f"Need at least 2 user rows, got {total}"

    # Search for "Alice" — should hide Bob's row
    await _type_search(page, "Alice")

    visible = page.locator(f"{SEL['users_tbody_row']}:not(.search-hidden)")
    await visible.first.wait_for(state="visible", timeout=5000)
    visible_count = await visible.count()
    assert visible_count >= 1, "Expected at least one visible row for 'Alice'"
    assert visible_count < total, "Search should have hidden some rows"

    first_text = await visible.first.text_content()
    assert "alice" in first_text.lower(), f"Visible row should contain 'alice', got: {first_text}"

    # Clear search restores all rows
    await _type_search(page, "")
    restored = page.locator(f"{SEL['users_tbody_row']}:not(.search-hidden)")
    await restored.first.wait_for(state="visible", timeout=5000)
    assert await restored.count() == total, "All user rows should be visible after clearing search"
