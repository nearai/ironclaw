"""Legacy extension lifecycle coverage ported to Reborn WebChat v2."""

import json
from urllib.parse import unquote, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


def _package_ref(package_id: str) -> dict:
    return {"kind": "extension", "id": package_id}


REGISTRY_TOOL = {
    "package_ref": _package_ref("registry-tool"),
    "display_name": "Registry Tool",
    "kind": "wasm_tool",
    "description": "A registry WASM tool",
    "keywords": ["search", "utility"],
    "installed": False,
}

REGISTRY_MCP = {
    "package_ref": _package_ref("registry-mcp"),
    "display_name": "Registry MCP Server",
    "kind": "mcp_server",
    "description": "An MCP server from the registry",
    "keywords": ["tools"],
    "installed": False,
}

ACTIVE_TOOL = {
    "package_ref": _package_ref("active-tool"),
    "display_name": "Active Tool",
    "kind": "wasm_tool",
    "description": "An installed WASM tool extension",
    "active": True,
    "authenticated": True,
    "has_auth": False,
    "needs_setup": False,
    "tools": ["search", "fetch"],
    "activation_status": "active",
}

INACTIVE_MCP = {
    "package_ref": _package_ref("inactive-mcp"),
    "display_name": "Inactive MCP",
    "kind": "mcp_server",
    "description": "An inactive MCP server",
    "active": False,
    "authenticated": False,
    "has_auth": False,
    "needs_setup": False,
    "tools": ["lookup"],
    "activation_status": "installed",
}

CHANNEL_READY = {
    "package_ref": _package_ref("telegram-channel"),
    "display_name": "Telegram Channel",
    "kind": "wasm_channel",
    "description": "A configured messaging channel",
    "active": True,
    "authenticated": True,
    "has_auth": True,
    "needs_setup": False,
    "tools": [],
    "activation_status": "ready",
    "onboarding_state": "ready",
}

AVAILABLE_CHANNEL = {
    "package_ref": _package_ref("slack-channel"),
    "display_name": "Slack Channel",
    "kind": "wasm_channel",
    "description": "A registry channel",
    "keywords": ["slack"],
    "installed": False,
}

LABEL_CHANNEL_BASE = {
    "package_ref": _package_ref("label-channel"),
    "display_name": "Label Channel",
    "kind": "wasm_channel",
    "description": "A WASM channel used to assert card action labels.",
    "active": False,
    "authenticated": False,
    "has_auth": False,
    "needs_setup": True,
    "tools": [],
}


async def _open_mocked_extensions_page(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    installed=None,
    registry=None,
    tab="registry",
    setup_payloads=None,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    installed_extensions = [dict(extension) for extension in (installed or [])]
    registry_entries = [dict(entry) for entry in (registry or [])]
    setup_payloads_by_id = dict(setup_payloads or {})
    install_requests: list[dict] = []
    activate_requests: list[str] = []
    remove_requests: list[str] = []

    async def fulfill_json(route, payload, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(payload),
            headers={"Cache-Control": "no-store"},
        )

    async def handle_extensions(route):
        nonlocal installed_extensions
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/extensions" and request.method == "GET":
            await fulfill_json(route, {"extensions": installed_extensions})
            return

        if path == "/api/webchat/v2/extensions/registry" and request.method == "GET":
            await fulfill_json(route, {"entries": registry_entries})
            return

        if (
            path.startswith("/api/webchat/v2/extensions/")
            and path.endswith("/setup")
            and request.method == "GET"
        ):
            package_id = unquote(
                path.removeprefix("/api/webchat/v2/extensions/").removesuffix("/setup")
            )
            await fulfill_json(
                route,
                setup_payloads_by_id.get(
                    package_id,
                    {
                        "name": package_id,
                        "kind": "wasm_channel",
                        "secrets": [],
                        "fields": [],
                        "onboarding": None,
                    },
                ),
            )
            return

        if path == "/api/webchat/v2/extensions/install" and request.method == "POST":
            payload = json.loads(request.post_data or "{}")
            package_ref = payload.get("package_ref") or {}
            package_id = package_ref.get("id")
            install_requests.append(payload)
            entry = next(
                (
                    registry_entry
                    for registry_entry in registry_entries
                    if registry_entry.get("package_ref", {}).get("id") == package_id
                ),
                None,
            )
            if entry and not any(
                extension.get("package_ref", {}).get("id") == package_id
                for extension in installed_extensions
            ):
                installed = dict(entry)
                installed.update(
                    {
                        "active": False,
                        "authenticated": False,
                        "activation_status": "installed",
                        "tools": entry.get("tools") or [],
                    }
                )
                installed.pop("installed", None)
                installed_extensions.append(installed)
            await fulfill_json(route, {"success": True, "message": "Registry Tool installed"})
            return

        if (
            path.startswith("/api/webchat/v2/extensions/")
            and path.endswith("/activate")
            and request.method == "POST"
        ):
            package_id = unquote(
                path.removeprefix("/api/webchat/v2/extensions/").removesuffix("/activate")
            )
            activate_requests.append(package_id)
            for extension in installed_extensions:
                if extension.get("package_ref", {}).get("id") == package_id:
                    extension["active"] = True
                    extension["activation_status"] = "active"
            await fulfill_json(route, {"success": True, "message": f"{package_id} activated"})
            return

        if (
            path.startswith("/api/webchat/v2/extensions/")
            and path.endswith("/remove")
            and request.method == "POST"
        ):
            package_id = unquote(
                path.removeprefix("/api/webchat/v2/extensions/").removesuffix("/remove")
            )
            remove_requests.append(package_id)
            installed_extensions = [
                extension
                for extension in installed_extensions
                if extension.get("package_ref", {}).get("id") != package_id
            ]
            await fulfill_json(route, {"success": True, "message": f"{package_id} removed"})
            return

        await route.continue_()

    async def handle_connectable_channels(route):
        await fulfill_json(route, {"channels": []})

    await page.route("**/api/webchat/v2/extensions**", handle_extensions)
    await page.route("**/api/webchat/v2/channels/connectable", handle_connectable_channels)
    await page.goto(f"{reborn_v2_server}/v2/extensions/{tab}?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.get_by_text("Registry").first).to_be_visible(timeout=15000)

    return {
        "context": context,
        "page": page,
        "install_requests": install_requests,
        "activate_requests": activate_requests,
        "remove_requests": remove_requests,
    }


def _card_by_title(page, title: str):
    return page.get_by_text(title, exact=True).locator(
        "xpath=ancestor::div[contains(@class, 'rounded-') and contains(@class, 'p-4')][1]"
    )


def _label_channel(**overrides):
    return {**LABEL_CHANNEL_BASE, **overrides}


async def _open_card_menu(card):
    await card.get_by_label("More actions").click()
    return card.get_by_role("menu")


async def test_reborn_legacy_extensions_registry_search_and_install(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        registry=[REGISTRY_TOOL, REGISTRY_MCP],
    )
    try:
        page = harness["page"]
        await expect(page.get_by_text("Registry Tool")).to_be_visible(timeout=5000)
        await expect(page.get_by_text("Registry MCP Server")).to_be_visible(timeout=5000)

        await page.locator('input[placeholder^="Search extensions"]').fill("mcp")
        await expect(page.get_by_text("Registry MCP Server")).to_be_visible()
        await expect(page.get_by_text("Registry Tool")).to_have_count(0)

        await page.locator('input[placeholder^="Search extensions"]').fill("")
        await page.get_by_text("Registry Tool").wait_for(state="visible", timeout=5000)
        registry_tool_card = _card_by_title(page, "Registry Tool")
        await registry_tool_card.get_by_role("button", name="Install").click()
        await expect(page.get_by_text("Registry Tool installed")).to_be_visible(timeout=5000)

        assert harness["install_requests"] == [
            {"package_ref": _package_ref("registry-tool")}
        ]
        await expect(page.get_by_text("Installed").first).to_be_visible(timeout=5000)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_installed_actions(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[ACTIVE_TOOL, INACTIVE_MCP],
        registry=[REGISTRY_TOOL, REGISTRY_MCP],
    )
    try:
        page = harness["page"]

        active_card = _card_by_title(page, "Active Tool")
        await expect(active_card).to_be_visible(timeout=5000)
        await active_card.get_by_role("button", name="2 capabilities").click()
        await expect(active_card.get_by_text("search")).to_be_visible()
        await expect(active_card.get_by_text("fetch")).to_be_visible()

        inactive_card = _card_by_title(page, "Inactive MCP")
        await expect(inactive_card).to_be_visible(timeout=5000)
        await inactive_card.get_by_role("button", name="Activate").click()
        await expect(page.get_by_text("inactive-mcp activated")).to_be_visible(timeout=5000)
        assert harness["activate_requests"] == ["inactive-mcp"]

        await active_card.get_by_label("More actions").click()
        await page.get_by_role("menuitem", name="Remove").click()
        await expect(page.get_by_text("Active Tool removed")).to_be_visible(timeout=5000)
        assert harness["remove_requests"] == ["active-tool"]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_channel_config_label_depends_on_authentication(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[
            _label_channel(
                authenticated=False,
                activation_status="configured",
                onboarding_state="activation_in_progress",
            ),
            _label_channel(
                package_ref=_package_ref("label-channel-authenticated"),
                display_name="Authenticated Label Channel",
                authenticated=True,
                activation_status="configured",
                onboarding_state="activation_in_progress",
            ),
        ],
        tab="channels",
    )
    try:
        page = harness["page"]

        unauthenticated = _card_by_title(page, "Label Channel")
        await expect(unauthenticated).to_be_visible(timeout=5000)
        await _open_card_menu(unauthenticated)
        await expect(
            page.get_by_role("menuitem", name="Configure", exact=True)
        ).to_have_count(1)
        await expect(
            page.get_by_role("menuitem", name="Reconfigure", exact=True)
        ).to_have_count(0)

        await page.mouse.click(8, 8)
        authenticated = _card_by_title(page, "Authenticated Label Channel")
        await expect(authenticated).to_be_visible(timeout=5000)
        await _open_card_menu(authenticated)
        await expect(
            page.get_by_role("menuitem", name="Reconfigure", exact=True)
        ).to_have_count(1)
        await expect(
            page.get_by_role("menuitem", name="Configure", exact=True)
        ).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_channel_setup_required_has_single_configure_action(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[
            _label_channel(
                authenticated=False,
                activation_status="installed",
                onboarding_state="setup_required",
            )
        ],
        tab="channels",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Label Channel")
        await expect(card).to_be_visible(timeout=5000)

        await expect(card.get_by_role("button", name="Configure")).to_have_count(1)
        await _open_card_menu(card)
        await expect(page.get_by_role("menuitem", name="Setup", exact=True)).to_have_count(
            0
        )
        await expect(
            page.get_by_role("menuitem", name="Configure", exact=True)
        ).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_channel_reconfigure_opens_modal_without_activate(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[
            _label_channel(
                active=True,
                authenticated=True,
                has_auth=True,
                needs_setup=True,
                activation_status="active",
                onboarding_state="ready",
            )
        ],
        setup_payloads={
            "label-channel": {
                "name": "label-channel",
                "kind": "wasm_channel",
                "secrets": [
                    {
                        "name": "BOT_TOKEN",
                        "prompt": "Bot token",
                        "provided": True,
                        "optional": False,
                        "auto_generate": False,
                    }
                ],
                "fields": [],
                "onboarding": None,
            }
        },
        tab="channels",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Label Channel")
        await expect(card).to_be_visible(timeout=5000)

        await _open_card_menu(card)
        await page.get_by_role("menuitem", name="Reconfigure", exact=True).click()
        await expect(page.get_by_role("heading", name="Configure Label Channel")).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("Bot token")).to_be_visible()
        assert harness["activate_requests"] == []
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_channels_and_mcp_tabs_render(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[CHANNEL_READY, INACTIVE_MCP],
        registry=[AVAILABLE_CHANNEL, REGISTRY_MCP],
        tab="channels",
    )
    try:
        page = harness["page"]
        await expect(page.get_by_text("Web Gateway")).to_be_visible(timeout=5000)
        await expect(page.get_by_text("HTTP Webhook")).to_be_visible()
        await expect(page.get_by_text("Telegram Channel")).to_be_visible()
        await expect(page.get_by_text("Slack Channel")).to_be_visible()

        await page.goto(f"{reborn_v2_server}/v2/extensions/mcp?token={REBORN_V2_AUTH_TOKEN}")
        await expect(page.get_by_text("Inactive MCP", exact=True)).to_be_visible(timeout=5000)
        await expect(page.get_by_text("Registry MCP Server", exact=True)).to_be_visible()
    finally:
        await harness["context"].close()
