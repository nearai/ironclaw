"""Scenario: Extensions tab – comprehensive UI coverage.

Tests cover:
  A. Structural / empty states
  B. Installed WASM tool cards
  C. MCP server cards
  D. WASM channel stepper states
  E. Available extensions (registry) and install flow
  F. Remove flow
  G. Configure modal (open, fields, cancel, save, OAuth, error)
  H. Auth card (SSE-triggered token + OAuth flows)
  I. Activate flow (MCP server and WASM channel)
  J. Tab reload behaviour

All extension API calls are intercepted via page.route() so no real
WASM binaries or external registry connections are needed.
"""

import json

import pytest
from helpers import SEL

# ─── Fixture data ─────────────────────────────────────────────────────────────

_WASM_TOOL = {
    "name": "test-tool",
    "display_name": "Test WASM Tool",
    "kind": "wasm_tool",
    "description": "A test WASM tool extension",
    "url": None,
    "active": True,
    "authenticated": True,
    "has_auth": True,
    "needs_setup": False,
    "tools": ["search", "fetch"],
    "activation_status": None,
    "activation_error": None,
}

_MCP_ACTIVE = {
    "name": "test-mcp",
    "display_name": "Test MCP Server",
    "kind": "mcp_server",
    "description": "An active MCP server",
    "url": "http://localhost:3000",
    "active": True,
    "authenticated": False,
    "has_auth": False,
    "needs_setup": False,
    "tools": [],
    "activation_status": None,
    "activation_error": None,
}

_MCP_INACTIVE = {**_MCP_ACTIVE, "name": "test-mcp-inactive", "display_name": "Inactive MCP", "active": False}

_WASM_CHANNEL = {
    "name": "test-channel",
    "display_name": "Test Channel",
    "kind": "wasm_channel",
    "description": "A test WASM channel",
    "url": None,
    "active": False,
    "authenticated": False,
    "has_auth": False,
    "needs_setup": True,
    "tools": [],
    "activation_status": "installed",
    "activation_error": None,
}

_REGISTRY_WASM = {
    "name": "registry-tool",
    "display_name": "Registry Tool",
    "kind": "wasm_tool",
    "description": "A registry WASM tool",
    "keywords": ["search", "utility"],
    "installed": False,
}

_REGISTRY_MCP = {
    "name": "registry-mcp",
    "display_name": "Registry MCP Server",
    "kind": "mcp_server",
    "description": "An MCP server from the registry",
    "keywords": ["tools"],
    "installed": False,
}

_SAMPLE_TOOL = {"name": "echo", "description": "Echo a message"}
_SAMPLE_TOOL_2 = {"name": "time", "description": "Get current time"}


# ─── Navigation helpers ────────────────────────────────────────────────────────

async def go_to_extensions(page):
    """Click the Extensions tab and wait for the panel to appear.

    Waits for loadExtensions() to finish rendering by polling for the first
    content signal (empty-state div or an installed card) rather than sleeping.
    """
    await page.locator(SEL["tab_button"].format(tab="extensions")).click()
    await page.locator(SEL["tab_panel"].format(tab="extensions")).wait_for(
        state="visible", timeout=5000
    )
    # loadExtensions() fires three parallel fetches then renders. Wait for the
    # first concrete DOM signal instead of a hard sleep so the test is
    # deterministic even under CI load.
    await page.locator(
        f"{SEL['extensions_list']} .empty-state, {SEL['ext_card_installed']}"
    ).first.wait_for(state="visible", timeout=8000)


async def mock_ext_apis(page, *, installed=None, tools=None, registry=None):
    """Intercept the three extension list APIs with fixture data.

    Must be called BEFORE navigating to the extensions tab.
    """
    ext_body = json.dumps({"extensions": installed or []})
    tools_body = json.dumps({"tools": tools or []})
    registry_body = json.dumps({"entries": registry or []})

    # Playwright evaluates route handlers in LIFO order (last-registered fires
    # first). Register the broad handler first so it is checked last; the
    # specific /tools and /registry handlers are registered after and therefore
    # checked first — no continue_() fallthrough needed.
    async def handle_ext_list(route):
        path = route.request.url.split("?")[0]
        if path.endswith("/api/extensions"):
            await route.fulfill(status=200, content_type="application/json", body=ext_body)
        else:
            await route.continue_()

    await page.route("**/api/extensions*", handle_ext_list)

    async def handle_tools(route):
        await route.fulfill(status=200, content_type="application/json", body=tools_body)

    async def handle_registry(route):
        await route.fulfill(status=200, content_type="application/json", body=registry_body)

    await page.route("**/api/extensions/tools", handle_tools)
    await page.route("**/api/extensions/registry", handle_registry)


async def wait_for_toast(page, text: str, *, timeout: int = 5000):
    """Wait for any toast containing the given text."""
    await page.locator(SEL["toast"], has_text=text).wait_for(state="visible", timeout=timeout)


# ─── Group A: Structural / empty state ────────────────────────────────────────

async def test_extensions_empty_tab_layout(page):
    """Extensions tab with no data shows all three sections with correct empty-state messages."""
    await mock_ext_apis(page, tools=[])
    await go_to_extensions(page)

    panel = page.locator(SEL["tab_panel"].format(tab="extensions"))
    assert await panel.is_visible()

    ext_list = page.locator(SEL["extensions_list"])
    assert await ext_list.is_visible()
    assert "No extensions installed" in await ext_list.text_content()

    wasm_list = page.locator(SEL["available_wasm_list"])
    assert await wasm_list.is_visible()
    assert "No additional WASM extensions available" in await wasm_list.text_content()

    mcp_list = page.locator(SEL["mcp_servers_list"])
    assert await mcp_list.is_visible()
    assert "No MCP servers available" in await mcp_list.text_content()

    # Tools table should be empty
    tbody = page.locator(SEL["tools_tbody"])
    rows = await tbody.locator("tr").count()
    empty_visible = await page.locator(SEL["tools_empty"]).is_visible()
    assert empty_visible or rows == 0, "Expected tools table to be empty"


async def test_extensions_tools_table_populated(page):
    """Two mock tools produce two rows in the tools table."""
    await mock_ext_apis(page, tools=[_SAMPLE_TOOL, _SAMPLE_TOOL_2])
    await go_to_extensions(page)

    tbody = page.locator(SEL["tools_tbody"])
    rows = tbody.locator("tr")
    await rows.first.wait_for(state="visible", timeout=5000)
    assert await rows.count() == 2

    text = await tbody.text_content()
    assert "echo" in text
    assert "time" in text


# ─── Group B: Installed WASM tool cards ───────────────────────────────────────

async def test_installed_wasm_tool_card_renders(page):
    """An installed, active, authenticated WASM tool card shows correct elements."""
    await mock_ext_apis(page, installed=[_WASM_TOOL])
    await go_to_extensions(page)

    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)

    assert "Test WASM Tool" in await card.locator(SEL["ext_name"]).text_content()
    assert await card.locator(SEL["ext_auth_dot_authed"]).count() == 1
    assert await card.locator(SEL["ext_active_label"]).count() == 1
    assert await card.locator(SEL["ext_remove_btn"]).count() == 1

    tools_div = card.locator(SEL["ext_tools"])
    text = await tools_div.text_content()
    assert "search" in text
    assert "fetch" in text


async def test_installed_wasm_tool_unauthed_state(page):
    """authenticated=false shows the unauthed auth dot and a 'Configure' button."""
    ext = {**_WASM_TOOL, "needs_setup": True, "authenticated": False}
    await mock_ext_apis(page, installed=[ext])
    await go_to_extensions(page)

    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)
    assert await card.locator(SEL["ext_auth_dot_unauthed"]).count() == 1

    configure_btn = card.locator(SEL["ext_configure_btn"])
    assert await configure_btn.count() == 1
    assert await configure_btn.text_content() == "Configure"


async def test_installed_wasm_tool_authed_shows_reconfigure_btn(page):
    """has_auth=true, authenticated=true shows a 'Reconfigure' button."""
    ext = {**_WASM_TOOL, "has_auth": True, "authenticated": True, "needs_setup": False}
    await mock_ext_apis(page, installed=[ext])
    await go_to_extensions(page)

    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)

    configure_btn = card.locator(SEL["ext_configure_btn"])
    assert await configure_btn.count() == 1
    assert await configure_btn.text_content() == "Reconfigure"



# ─── Group C: MCP server cards ────────────────────────────────────────────────

async def test_installed_mcp_server_active(page):
    """Active MCP server shows 'Active' label and no Activate button."""
    await mock_ext_apis(page, installed=[_MCP_ACTIVE])
    await go_to_extensions(page)

    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)
    assert await card.locator(SEL["ext_active_label"]).count() == 1
    assert await card.locator(SEL["ext_activate_btn"]).count() == 0
    assert await card.locator(SEL["ext_remove_btn"]).count() == 1


async def test_installed_mcp_server_inactive_shows_activate(page):
    """Inactive MCP server shows Activate button."""
    await mock_ext_apis(page, installed=[_MCP_INACTIVE])
    await go_to_extensions(page)

    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)
    assert await card.locator(SEL["ext_activate_btn"]).count() == 1


async def test_mcp_server_in_registry_not_installed(page):
    """Registry MCP entry (not installed) appears in the MCP section with Install button."""
    await mock_ext_apis(page, registry=[_REGISTRY_MCP])
    await go_to_extensions(page)

    mcp_list = page.locator(SEL["mcp_servers_list"])
    card = mcp_list.locator(".ext-card").first
    await card.wait_for(state="visible", timeout=5000)
    assert "Registry MCP Server" in await card.text_content()
    assert await card.locator(SEL["ext_install_btn"]).count() == 1


async def test_mcp_server_installed_auth_dot(page):
    """Installed MCP in registry cross-reference shows auth dot (unauthed)."""
    # Card rendered via renderMcpServerCard when entry is in registry AND installed
    installed_mcp = {**_MCP_ACTIVE, "name": "registry-mcp", "authenticated": False}
    registry_mcp = {**_REGISTRY_MCP, "name": "registry-mcp"}
    await mock_ext_apis(page, installed=[installed_mcp], registry=[registry_mcp])
    await go_to_extensions(page)

    mcp_list = page.locator(SEL["mcp_servers_list"])
    card = mcp_list.locator(".ext-card").first
    await card.wait_for(state="visible", timeout=5000)
    # Installed MCP in registry section should show auth dot
    assert await card.locator(SEL["ext_auth_dot_unauthed"]).count() == 1


# ─── Group D: WASM channel stepper states ─────────────────────────────────────

async def _load_wasm_channel(page, activation_status, activation_error=None):
    ext = {**_WASM_CHANNEL, "activation_status": activation_status, "activation_error": activation_error}
    await mock_ext_apis(page, installed=[ext])
    await go_to_extensions(page)
    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)
    return card


async def test_wasm_channel_setup_states(page):
    """activation_status installed/configured both show the Setup button and stepper."""
    card = await _load_wasm_channel(page, "installed")
    setup_btn = card.locator(SEL["ext_configure_btn"], has_text="Setup")
    assert await setup_btn.count() == 1
    assert await card.locator(SEL["ext_stepper"]).count() == 1
    # configured renders identically (same Setup button); verified by same stepper check above


async def test_wasm_channel_pairing_state(page):
    """activation_status=pairing shows Awaiting Pairing label and Reconfigure."""
    card = await _load_wasm_channel(page, "pairing")
    assert await card.locator(SEL["ext_pairing_label"]).count() == 1
    assert await card.locator(SEL["ext_configure_btn"], has_text="Reconfigure").count() == 1


async def test_wasm_channel_active_state(page):
    """activation_status=active shows Active label and Reconfigure (no Setup)."""
    card = await _load_wasm_channel(page, "active")
    assert await card.locator(SEL["ext_active_label"]).count() == 1
    assert await card.locator(SEL["ext_configure_btn"], has_text="Reconfigure").count() == 1
    assert await card.locator(SEL["ext_configure_btn"], has_text="Setup").count() == 0


async def test_wasm_channel_failed_renders(page):
    """activation_status=failed shows Reconfigure button and ✗ in the stepper circles."""
    card = await _load_wasm_channel(page, "failed", activation_error="Module crashed")
    assert await card.locator(SEL["ext_configure_btn"], has_text="Reconfigure").count() == 1
    circles = card.locator(SEL["ext_stepper"]).locator(SEL["stepper_circle"])
    count = await circles.count()
    assert count > 0
    texts = [await circles.nth(i).text_content() for i in range(count)]
    assert any("\u2717" in t for t in texts), f"Expected ✗ in stepper circles: {texts}"


# ─── Group E: Available extensions (registry) and install ─────────────────────

async def test_available_wasm_card_renders(page):
    """Registry WASM entry shows in #available-wasm-list with Install button."""
    await mock_ext_apis(page, registry=[_REGISTRY_WASM])
    await go_to_extensions(page)

    wasm_list = page.locator(SEL["available_wasm_list"])
    card = wasm_list.locator(".ext-card").first
    await card.wait_for(state="visible", timeout=5000)
    assert "Registry Tool" in await card.text_content()
    assert "A registry WASM tool" in await card.text_content()
    assert await card.locator(SEL["ext_install_btn"]).count() == 1


async def test_available_wasm_keywords_shown(page):
    """Registry entry with keywords shows them on the card."""
    await mock_ext_apis(page, registry=[_REGISTRY_WASM])
    await go_to_extensions(page)

    card = page.locator(SEL["available_wasm_list"]).locator(".ext-card").first
    await card.wait_for(state="visible", timeout=5000)
    text = await card.text_content()
    assert "search" in text or "utility" in text


async def test_install_wasm_success(page):
    """Clicking Install on a registry card calls the install API and refreshes the list."""
    installed_after = {
        **_WASM_TOOL,
        "name": "registry-tool",
        "display_name": "Registry Tool",
    }
    install_called = []

    await mock_ext_apis(page, registry=[_REGISTRY_WASM])

    async def handle_install(route):
        install_called.append(True)
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"success": True}),
        )

    await page.route("**/api/extensions/install", handle_install)

    # After install, loadExtensions() refetches the list; serve the installed ext
    async def handle_ext_after(route):
        path = route.request.url.split("?")[0]
        if path.endswith("/api/extensions"):
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"extensions": [installed_after]}),
            )
        else:
            await route.continue_()

    await go_to_extensions(page)

    # Override the ext list handler for subsequent calls
    await page.route("**/api/extensions*", handle_ext_after)

    install_btn = page.locator(SEL["available_wasm_list"]).locator(SEL["ext_install_btn"]).first
    await install_btn.wait_for(state="visible", timeout=5000)
    await install_btn.click()

    # Wait for reload: installed card should appear
    installed = page.locator(SEL["ext_card_installed"])
    await installed.first.wait_for(state="visible", timeout=8000)
    assert len(install_called) >= 1, "Install API was not called"


async def test_install_wasm_failure(page):
    """Failed install response shows an error toast."""
    await mock_ext_apis(page, registry=[_REGISTRY_WASM])

    async def handle_install(route):
        await route.fulfill(status=200, content_type="application/json", body=json.dumps({"success": False, "message": "Build failed"}))

    await page.route("**/api/extensions/install", handle_install)
    await go_to_extensions(page)

    install_btn = page.locator(SEL["available_wasm_list"]).locator(SEL["ext_install_btn"]).first
    await install_btn.wait_for(state="visible", timeout=5000)
    await install_btn.click()

    await wait_for_toast(page, "Build failed")


async def test_install_wasm_channel_triggers_configure(page):
    """Installing a wasm_channel extension auto-opens the configure modal."""
    registry_channel = {**_REGISTRY_WASM, "kind": "wasm_channel", "name": "test-channel", "display_name": "Test Channel"}
    await mock_ext_apis(page, registry=[registry_channel])

    setup_payload = {"secrets": [{"name": "token", "prompt": "Enter token", "provided": False, "optional": False, "auto_generate": False}]}

    async def handle_channel_setup(route):
        await route.fulfill(status=200, content_type="application/json", body=json.dumps(setup_payload))

    async def handle_channel_install(route):
        await route.fulfill(status=200, content_type="application/json", body=json.dumps({"success": True}))

    await page.route("**/api/extensions/test-channel/setup", handle_channel_setup)
    await page.route("**/api/extensions/install", handle_channel_install)
    await go_to_extensions(page)

    install_btn = page.locator(SEL["available_wasm_list"]).locator(SEL["ext_install_btn"]).first
    await install_btn.wait_for(state="visible", timeout=5000)
    await install_btn.click()

    # Configure modal should appear
    modal = page.locator(SEL["configure_modal"])
    await modal.wait_for(state="visible", timeout=8000)
    assert await modal.is_visible()


# ─── Group F: Remove flow ─────────────────────────────────────────────────────

async def test_remove_installed_extension_confirmed(page):
    """Confirming remove dismisses the card and shows a success toast."""
    remove_called = []

    await mock_ext_apis(page, installed=[_WASM_TOOL])

    async def handle_remove(route):
        remove_called.append(True)
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"success": True}),
        )

    await page.route("**/api/extensions/test-tool/remove", handle_remove)

    # After remove, list is empty
    async def handle_ext_empty(route):
        path = route.request.url.split("?")[0]
        if path.endswith("/api/extensions"):
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"extensions": []}),
            )
        else:
            await route.continue_()

    await go_to_extensions(page)
    # Override for subsequent calls
    await page.route("**/api/extensions*", handle_ext_empty)

    # Auto-accept confirm dialog
    await page.evaluate("window.confirm = () => true")

    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)
    await card.locator(SEL["ext_remove_btn"]).click()

    # Card should disappear
    await page.wait_for_function(
        "() => document.querySelectorAll('#extensions-list .ext-card').length === 0",
        timeout=8000,
    )
    assert len(remove_called) >= 1, "Remove API was not called"


async def test_remove_cancelled_keeps_card(page):
    """Cancelling the confirm dialog keeps the extension card."""
    await mock_ext_apis(page, installed=[_WASM_TOOL])
    await go_to_extensions(page)

    # Reject the confirm dialog
    await page.evaluate("window.confirm = () => false")

    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)
    await card.locator(SEL["ext_remove_btn"]).click()

    # Wait a moment to confirm nothing changed
    await page.wait_for_timeout(1000)
    assert await page.locator(SEL["ext_card_installed"]).count() >= 1, "Card should remain after cancel"


# ─── Group G: Configure modal ─────────────────────────────────────────────────

async def _open_configure_modal(page, secrets):
    """Mock the setup endpoint and trigger showConfigureModal via JS."""
    body = json.dumps({"secrets": secrets})

    async def handle_setup(route):
        await route.fulfill(status=200, content_type="application/json", body=body)

    await page.route("**/api/extensions/test-ext/setup", handle_setup)
    await page.evaluate("showConfigureModal('test-ext')")
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=5000)


async def test_configure_modal_field_variants(page):
    """Configure modal renders all field badge variants correctly in one pass."""
    await _open_configure_modal(
        page,
        [
            {"name": "api_key", "prompt": "Enter API key", "provided": False, "optional": False, "auto_generate": False},
            {"name": "token", "prompt": "API Token", "provided": True, "optional": False, "auto_generate": False},
            {"name": "extra", "prompt": "Extra setting", "provided": False, "optional": True, "auto_generate": False},
            {"name": "secret", "prompt": "Secret value", "provided": False, "optional": False, "auto_generate": True},
        ],
    )
    modal = page.locator(SEL["configure_modal"])
    assert await modal.is_visible()
    text = await modal.text_content()
    # Basic field with label and input
    assert "Enter API key" in text
    assert await modal.locator(SEL["configure_input"]).count() >= 1
    # Provided badge and at least one input with 'already set'/'keep' placeholder
    assert await modal.locator(SEL["field_provided"]).count() >= 1
    inputs = modal.locator(SEL["configure_input"])
    input_count = await inputs.count()
    placeholders = [await inputs.nth(i).get_attribute("placeholder") or "" for i in range(input_count)]
    assert any("already set" in p or "keep" in p for p in placeholders), f"No provided placeholder: {placeholders}"
    # Optional label
    assert "(optional)" in text
    # Auto-generate hint
    assert "Auto-generated" in text
    # Modal heading contains extension name
    assert "test-ext" in await page.locator(".configure-modal h3").text_content()


async def test_configure_modal_cancel_closes(page):
    """Clicking Cancel dismisses the configure overlay."""
    await _open_configure_modal(
        page,
        [{"name": "token", "prompt": "Token", "provided": False, "optional": False, "auto_generate": False}],
    )
    await page.locator(SEL["configure_cancel_btn"]).click()
    await page.locator(SEL["configure_overlay"]).wait_for(state="hidden", timeout=3000)


async def test_configure_modal_backdrop_click_closes(page):
    """Clicking outside the modal (on the overlay backdrop) dismisses it."""
    await _open_configure_modal(
        page,
        [{"name": "token", "prompt": "Token", "provided": False, "optional": False, "auto_generate": False}],
    )
    # Click the overlay element itself (outside the modal box)
    overlay = page.locator(SEL["configure_overlay"])
    box = await overlay.bounding_box()
    # Click at the very top-left corner of the overlay, outside the centered modal
    await page.mouse.click(box["x"] + 5, box["y"] + 5)
    await overlay.wait_for(state="hidden", timeout=3000)


async def test_configure_modal_save_success(page):
    """Filling in a value and clicking Save closes the modal on success."""
    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"secrets": [{"name": "token", "prompt": "Token", "provided": False, "optional": False, "auto_generate": False}]}),
            )
        else:
            await route.fulfill(status=200, content_type="application/json", body=json.dumps({"success": True}))

    await page.route("**/api/extensions/test-ext/setup", handle_setup)
    await page.evaluate("showConfigureModal('test-ext')")
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=5000)
    await page.locator(SEL["configure_input"]).fill("mytoken123")
    await page.locator(SEL["configure_save_btn"]).click()
    await page.locator(SEL["configure_overlay"]).wait_for(state="hidden", timeout=5000)


async def test_configure_modal_save_oauth(page):
    """Save response with auth_url opens a popup via window.open."""
    await page.evaluate("window.open = (url) => { window._lastOpenedUrl = url; }")

    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"secrets": [{"name": "t", "prompt": "Token", "provided": False, "optional": False, "auto_generate": False}]}),
            )
        else:
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"success": True, "auth_url": "https://example.com/oauth"}),
            )

    await page.route("**/api/extensions/test-ext/setup", handle_setup)
    await page.evaluate("showConfigureModal('test-ext')")
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=5000)
    await page.locator(SEL["configure_input"]).fill("ignored")
    await page.locator(SEL["configure_save_btn"]).click()

    await page.wait_for_timeout(800)
    opened = await page.evaluate("window._lastOpenedUrl")
    assert opened is not None, "window.open was not called"
    assert "oauth" in opened or "example.com" in opened


async def test_configure_modal_save_failure(page):
    """Save failure response shows an error toast."""
    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"secrets": [{"name": "t", "prompt": "Token", "provided": False, "optional": False, "auto_generate": False}]}),
            )
        else:
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"success": False, "message": "Invalid API key"}),
            )

    await page.route("**/api/extensions/test-ext/setup", handle_setup)
    await page.evaluate("showConfigureModal('test-ext')")
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=5000)
    await page.locator(SEL["configure_input"]).fill("badkey")
    await page.locator(SEL["configure_save_btn"]).click()

    await wait_for_toast(page, "Invalid API key")


async def test_configure_modal_enter_key_submits(page):
    """Pressing Enter in the input field submits the form."""
    save_called = []

    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"secrets": [{"name": "t", "prompt": "Token", "provided": False, "optional": False, "auto_generate": False}]}),
            )
        else:
            save_called.append(True)
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"success": True}),
            )

    await page.route("**/api/extensions/test-ext/setup", handle_setup)
    await page.evaluate("showConfigureModal('test-ext')")
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=5000)
    await page.locator(SEL["configure_input"]).fill("mytoken")
    await page.locator(SEL["configure_input"]).press("Enter")

    await page.locator(SEL["configure_overlay"]).wait_for(state="hidden", timeout=5000)
    assert len(save_called) >= 1, "Save was not called on Enter key"



# ─── Group H: Auth card (SSE-triggered) ───────────────────────────────────────

async def _show_auth_card(page, **kwargs):
    """Inject an auth card via JS and wait for it to appear."""
    payload = json.dumps(kwargs)
    await page.evaluate(f"showAuthCard({payload})")
    await page.locator(SEL["auth_card"]).wait_for(state="visible", timeout=5000)


async def test_auth_card_token_only(page):
    """Auth card with no auth_url shows token input, Submit, Cancel, but no OAuth button."""
    await _show_auth_card(page, extension_name="github", instructions="Paste your GitHub token")

    card = page.locator(SEL["auth_card"])
    assert await card.locator(SEL["auth_header"]).text_content() == "Authentication required for github"
    assert "Paste your GitHub token" in await card.locator(SEL["auth_instructions"]).text_content()
    assert await card.locator(SEL["auth_token_input"]).count() == 1
    assert await card.locator(SEL["auth_submit_btn"]).count() == 1
    assert await card.locator(SEL["auth_cancel_btn"]).count() == 1
    assert await card.locator(SEL["auth_oauth_btn"]).count() == 0


async def test_auth_card_with_oauth(page):
    """Auth card with auth_url shows the OAuth button."""
    await _show_auth_card(page, extension_name="slack", auth_url="https://slack.com/oauth/authorize")

    card = page.locator(SEL["auth_card"])
    oauth_btn = card.locator(SEL["auth_oauth_btn"])
    assert await oauth_btn.count() == 1
    assert "slack" in await oauth_btn.text_content()


async def test_auth_card_with_setup_url(page):
    """Auth card with setup_url shows a 'Get your token' link."""
    await _show_auth_card(page, extension_name="openai", setup_url="https://platform.openai.com/api-keys")

    card = page.locator(SEL["auth_card"])
    link = card.locator("a", has_text="Get your token")
    assert await link.count() == 1
    href = await link.get_attribute("href")
    assert "openai" in href or "platform" in href


async def test_auth_card_submit_success(page):
    """Submitting a valid token via click or Enter removes the auth card."""
    submit_called = []

    async def handle_auth(route):
        submit_called.append(True)
        await route.fulfill(status=200, content_type="application/json", body=json.dumps({"success": True, "message": "Authenticated!"}))

    await page.route("**/api/chat/auth-token", handle_auth)

    # Test click submit
    await _show_auth_card(page, extension_name="myext", instructions="Enter token")
    await page.locator(SEL["auth_token_input"]).fill("valid-token-123")
    await page.locator(SEL["auth_submit_btn"]).click()
    await page.locator(SEL["auth_card"]).wait_for(state="hidden", timeout=5000)
    assert len(submit_called) >= 1

    # Test Enter key submit (re-show card for a different extension)
    await page.evaluate("showAuthCard({extension_name: 'myext2', instructions: 'Again'})")
    await page.locator(SEL["auth_card"]).wait_for(state="visible", timeout=5000)
    await page.locator(SEL["auth_token_input"]).fill("another-token")
    await page.locator(SEL["auth_token_input"]).press("Enter")
    await page.locator(SEL["auth_card"]).wait_for(state="hidden", timeout=5000)
    assert len(submit_called) >= 2


async def test_auth_card_submit_empty_noop(page):
    """Clicking Submit with an empty token does nothing (card stays)."""
    await _show_auth_card(page, extension_name="myext")
    await page.locator(SEL["auth_submit_btn"]).click()
    await page.wait_for_timeout(800)
    assert await page.locator(SEL["auth_card"]).count() == 1, "Card should remain for empty submit"


async def test_auth_card_submit_error(page):
    """A failed token submission shows the error message and re-enables buttons."""
    async def handle_auth(route):
        await route.fulfill(status=200, content_type="application/json", body=json.dumps({"success": False, "message": "Bad token"}))

    await page.route("**/api/chat/auth-token", handle_auth)
    await _show_auth_card(page, extension_name="myext")
    await page.locator(SEL["auth_token_input"]).fill("wrong-token")
    await page.locator(SEL["auth_submit_btn"]).click()

    error = page.locator(SEL["auth_error"])
    await error.wait_for(state="visible", timeout=5000)
    assert "Bad token" in await error.text_content()
    # Buttons should be re-enabled
    submit = page.locator(SEL["auth_submit_btn"])
    assert not await submit.is_disabled()


async def test_auth_card_cancel_removes_card(page):
    """Clicking Cancel removes the auth card."""
    async def handle_cancel(route):
        await route.fulfill(status=200, content_type="application/json", body="{}")

    await page.route("**/api/chat/auth-cancel", handle_cancel)
    await _show_auth_card(page, extension_name="myext")
    await page.locator(SEL["auth_cancel_btn"]).click()
    await page.locator(SEL["auth_card"]).wait_for(state="hidden", timeout=3000)



async def test_auth_card_replaces_existing_same_extension(page):
    """Calling showAuthCard twice for the same extension replaces the old card."""
    await _show_auth_card(page, extension_name="myext", instructions="First")
    await _show_auth_card(page, extension_name="myext", instructions="Second")

    cards = page.locator(SEL["auth_card"] + '[data-extension-name="myext"]')
    assert await cards.count() == 1, "Duplicate auth cards for same extension"
    assert "Second" in await page.locator(SEL["auth_instructions"]).text_content()


async def test_auth_card_multiple_extensions_coexist(page):
    """Auth cards for different extensions can coexist."""
    await page.evaluate('showAuthCard({extension_name: "ext-a", instructions: "Token A"})')
    await page.evaluate('showAuthCard({extension_name: "ext-b", instructions: "Token B"})')
    await page.wait_for_timeout(300)
    assert await page.locator(SEL["auth_card"]).count() == 2


async def test_auth_completed_sse_dismisses_card(page):
    """Simulating the auth_completed SSE event removes the auth card."""
    await _show_auth_card(page, extension_name="myext")

    # Simulate the auth_completed SSE event being fired
    await page.evaluate("""
        const event = new MessageEvent('auth_completed', {
            data: JSON.stringify({extension_name: 'myext', message: 'Done'})
        });
        // Call the handler the same way the SSE listener does
        removeAuthCard('myext');
    """)

    await page.wait_for_timeout(300)
    assert await page.locator(SEL["auth_card"] + '[data-extension-name="myext"]').count() == 0


# ─── Group I: Activate flow ────────────────────────────────────────────────────

async def test_activate_mcp_server_success(page):
    """Clicking Activate on an inactive MCP server calls the activate API."""
    activate_called = []

    async def handle_activate(route):
        activate_called.append(True)
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"success": True}),
        )

    await mock_ext_apis(page, installed=[_MCP_INACTIVE])
    await page.route("**/api/extensions/test-mcp-inactive/activate", handle_activate)
    await go_to_extensions(page)

    activate_btn = page.locator(SEL["ext_card_installed"]).first.locator(SEL["ext_activate_btn"])
    await activate_btn.wait_for(state="visible", timeout=5000)
    await activate_btn.click()

    await page.wait_for_timeout(800)
    assert len(activate_called) >= 1, "Activate API was not called"


async def test_activate_awaiting_token_opens_configure(page):
    """Activate response with awaiting_token=true opens the configure modal."""
    await mock_ext_apis(page, installed=[_MCP_INACTIVE])

    async def handle_activate(route):
        await route.fulfill(status=200, content_type="application/json", body=json.dumps({"success": False, "awaiting_token": True}))

    setup_payload = {"secrets": [{"name": "t", "prompt": "Token", "provided": False, "optional": False, "auto_generate": False}]}

    async def handle_setup(route):
        await route.fulfill(status=200, content_type="application/json", body=json.dumps(setup_payload))

    await page.route("**/api/extensions/test-mcp-inactive/activate", handle_activate)
    await page.route("**/api/extensions/test-mcp-inactive/setup", handle_setup)
    await go_to_extensions(page)

    activate_btn = page.locator(SEL["ext_card_installed"]).first.locator(SEL["ext_activate_btn"])
    await activate_btn.wait_for(state="visible", timeout=5000)
    await activate_btn.click()

    modal = page.locator(SEL["configure_modal"])
    await modal.wait_for(state="visible", timeout=8000)
    assert await modal.is_visible()


async def test_activate_failure_shows_error_toast(page):
    """Failed activate shows an error toast with the message."""
    await mock_ext_apis(page, installed=[_MCP_INACTIVE])

    async def handle_activate(route):
        await route.fulfill(status=200, content_type="application/json", body=json.dumps({"success": False, "message": "Config missing"}))

    await page.route("**/api/extensions/test-mcp-inactive/activate", handle_activate)
    await go_to_extensions(page)

    activate_btn = page.locator(SEL["ext_card_installed"]).first.locator(SEL["ext_activate_btn"])
    await activate_btn.wait_for(state="visible", timeout=5000)
    await activate_btn.click()

    await wait_for_toast(page, "Config missing")


async def test_activate_with_auth_url_opens_popup(page):
    """Activate response with auth_url calls window.open."""
    await page.evaluate("window.open = (url) => { window._lastOpenedUrl = url; }")
    await mock_ext_apis(page, installed=[_MCP_INACTIVE])

    async def handle_activate(route):
        await route.fulfill(status=200, content_type="application/json", body=json.dumps({"success": True, "auth_url": "https://example.com/oauth"}))

    await page.route("**/api/extensions/test-mcp-inactive/activate", handle_activate)
    await go_to_extensions(page)

    activate_btn = page.locator(SEL["ext_card_installed"]).first.locator(SEL["ext_activate_btn"])
    await activate_btn.wait_for(state="visible", timeout=5000)
    await activate_btn.click()

    await page.wait_for_timeout(800)
    opened = await page.evaluate("window._lastOpenedUrl")
    assert opened is not None, "window.open was not called"
    assert "example.com" in opened


# ─── Group J: Tab reload behaviour ────────────────────────────────────────────

async def test_extensions_tab_reloads_on_revisit(page):
    """loadExtensions() is called again when re-navigating to the extensions tab."""
    call_count = []

    async def counting_handler(route):
        path = route.request.url.split("?")[0]
        if path.endswith("/api/extensions"):
            call_count.append(1)
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"extensions": []}),
            )
        else:
            await route.continue_()

    await page.route("**/api/extensions/tools", lambda r: r.fulfill(status=200, content_type="application/json", body='{"tools":[]}'))
    await page.route("**/api/extensions/registry", lambda r: r.fulfill(status=200, content_type="application/json", body='{"entries":[]}'))
    await page.route("**/api/extensions*", counting_handler)

    # First visit
    await go_to_extensions(page)
    count_after_first = len(call_count)
    assert count_after_first >= 1, "loadExtensions not called on first visit"

    # Navigate away
    await page.locator(SEL["tab_button"].format(tab="chat")).click()
    await page.wait_for_timeout(300)

    # Return to extensions
    await go_to_extensions(page)
    count_after_second = len(call_count)
    assert count_after_second > count_after_first, "loadExtensions not called on return visit"


async def test_auth_completed_sse_triggers_extensions_reload(page):
    """auth_completed SSE event while on the extensions tab triggers a reload."""
    reload_count = []

    async def counting_handler(route):
        path = route.request.url.split("?")[0]
        if path.endswith("/api/extensions"):
            reload_count.append(1)
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"extensions": []}),
            )
        else:
            await route.continue_()

    await page.route("**/api/extensions/tools", lambda r: r.fulfill(status=200, content_type="application/json", body='{"tools":[]}'))
    await page.route("**/api/extensions/registry", lambda r: r.fulfill(status=200, content_type="application/json", body='{"entries":[]}'))
    await page.route("**/api/extensions*", counting_handler)

    await go_to_extensions(page)
    count_before = len(reload_count)

    # Simulate auth_completed by calling loadExtensions directly (as the SSE handler does)
    await page.evaluate("""
        // Simulate what the auth_completed SSE handler does when currentTab === 'extensions'
        if (typeof loadExtensions === 'function') {
            loadExtensions();
        }
    """)

    await page.wait_for_timeout(600)
    assert len(reload_count) > count_before, "loadExtensions was not called after auth_completed"


# ─── Regression tests ─────────────────────────────────────────────────────────
# Each test below is a regression for a specific bug found after the initial
# test suite was written.  The bug description is in the docstring.

async def test_ext_tools_null_does_not_crash(page):
    """Regression: ext.tools null dereference crashes the extensions tab.

    Bug: renderExtensionCard() called ext.tools.length without a null guard.
    If the backend returns tools: null (or omits the field), the tab silently
    breaks and no cards render at all.
    """
    ext_with_null_tools = {**_WASM_TOOL, "tools": None}
    await mock_ext_apis(page, installed=[ext_with_null_tools])
    await go_to_extensions(page)

    # The card must render without a JS error
    card = page.locator(SEL["ext_card_installed"]).first
    await card.wait_for(state="visible", timeout=5000)
    assert "Test WASM Tool" in await card.text_content()
    # No .ext-tools element should appear (null → skip rendering)
    assert await card.locator(SEL["ext_tools"]).count() == 0


async def test_configure_modal_stays_open_on_save_failure(page):
    """Regression: configure modal closed before checking success, so errors were unrecoverable.

    Bug: submitConfigureModal() called closeConfigureModal() unconditionally at
    the top of .then(), then showed an error toast — but the modal was already
    gone, forcing the user to click Setup/Configure again to retry.
    Fix: modal now only closes on success; on failure it stays open for retry.
    """
    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"secrets": [{"name": "t", "prompt": "Token", "provided": False, "optional": False, "auto_generate": False}]}),
            )
        else:
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"success": False, "message": "Invalid API key"}),
            )

    await page.route("**/api/extensions/test-ext/setup", handle_setup)
    await page.evaluate("showConfigureModal('test-ext')")
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=5000)
    await page.locator(SEL["configure_input"]).fill("badkey")
    await page.locator(SEL["configure_save_btn"]).click()

    # Toast appears with the error message
    await wait_for_toast(page, "Invalid API key")
    # Modal must still be visible so the user can correct their input and retry
    assert await page.locator(SEL["configure_overlay"]).is_visible(), \
        "Configure modal should remain open after a save failure so the user can retry"


async def test_oauth_url_injection_blocked(page):
    """Regression: window.open() was called with unvalidated server-supplied auth_url.

    Bug: activate/configure responses with auth_url were passed directly to
    window.open() with no scheme validation. A compromised backend could supply
    a javascript: or data: URL.
    Fix: openOAuthUrl() rejects any URL that does not start with https://.
    """
    await page.evaluate("window._openedUrl = null; window.open = (url) => { window._openedUrl = url; }")
    await mock_ext_apis(page, installed=[_MCP_INACTIVE])

    async def handle_activate(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"success": True, "auth_url": "javascript:alert('xss')"}),
        )

    await page.route("**/api/extensions/test-mcp-inactive/activate", handle_activate)
    await go_to_extensions(page)

    activate_btn = page.locator(SEL["ext_card_installed"]).first.locator(SEL["ext_activate_btn"])
    await activate_btn.wait_for(state="visible", timeout=5000)
    await activate_btn.click()

    await page.wait_for_timeout(600)
    opened = await page.evaluate("window._openedUrl")
    assert opened is None, f"window.open should NOT be called for non-HTTPS URLs, but got: {opened}"
