"""Legacy extension lifecycle coverage ported to Reborn WebChat v2."""

import asyncio
import json
from urllib.parse import parse_qs, unquote, urlparse

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

TELEGRAM_CHANNEL_SETUP = {
    "package_ref": _package_ref("telegram"),
    "display_name": "Telegram",
    "kind": "wasm_channel",
    "description": "Telegram bot channel",
    "active": False,
    "authenticated": False,
    "has_auth": True,
    "needs_setup": True,
    "tools": [],
    "activation_status": "setup_required",
    "onboarding_state": "setup_required",
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

CONFIG_TOOL = {
    "package_ref": _package_ref("config-tool"),
    "display_name": "Config Tool",
    "kind": "wasm_tool",
    "description": "A tool that requires manual setup.",
    "active": False,
    "authenticated": False,
    "has_auth": True,
    "needs_setup": True,
    "tools": [],
    "activation_status": "setup_required",
    "onboarding_state": "setup_required",
}

OAUTH_TOOL = {
    "package_ref": _package_ref("oauth-tool"),
    "display_name": "OAuth Tool",
    "kind": "wasm_tool",
    "description": "A tool that requires OAuth setup.",
    "active": False,
    "authenticated": False,
    "has_auth": True,
    "needs_setup": True,
    "tools": [],
    "activation_status": "setup_required",
    "onboarding_state": "setup_required",
}

CONFIG_TOOL_REGISTRY = {
    "package_ref": _package_ref("config-tool"),
    "display_name": "Config Tool",
    "kind": "wasm_tool",
    "description": "A registry tool that requires manual setup.",
    "keywords": ["config"],
    "installed": True,
    "has_auth": True,
    "needs_setup": True,
}


async def _open_mocked_extensions_page(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    installed=None,
    registry=None,
    tab="registry",
    setup_payloads=None,
    setup_get_responses=None,
    setup_submit_responses=None,
    install_responses=None,
    oauth_start_responses=None,
    activate_responses=None,
    remove_responses=None,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    installed_extensions = [dict(extension) for extension in (installed or [])]
    registry_entries = [dict(entry) for entry in (registry or [])]
    setup_payloads_by_id = dict(setup_payloads or {})
    setup_get_responses_by_id = dict(setup_get_responses or {})
    setup_submit_responses_by_id = dict(setup_submit_responses or {})
    install_responses_by_id = dict(install_responses or {})
    oauth_start_responses_by_id = dict(oauth_start_responses or {})
    activate_responses_by_id = dict(activate_responses or {})
    remove_responses_by_id = dict(remove_responses or {})
    install_requests: list[dict] = []
    activate_requests: list[str] = []
    remove_requests: list[str] = []
    setup_submit_requests: list[dict] = []
    oauth_start_requests: list[dict] = []
    extension_list_requests: list[str] = []
    registry_requests: list[str] = []

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
            extension_list_requests.append(request.url)
            await fulfill_json(route, {"extensions": installed_extensions})
            return

        if path == "/api/webchat/v2/extensions/registry" and request.method == "GET":
            registry_requests.append(request.url)
            await fulfill_json(route, {"entries": registry_entries})
            return

        if (
            path.startswith("/api/webchat/v2/extensions/")
            and path.endswith("/setup/oauth/start")
            and request.method == "POST"
        ):
            package_id = unquote(
                path.removeprefix("/api/webchat/v2/extensions/").removesuffix(
                    "/setup/oauth/start"
                )
            )
            payload = json.loads(request.post_data or "{}")
            oauth_start_requests.append({"package_id": package_id, "body": payload})
            await fulfill_json(
                route,
                oauth_start_responses_by_id.get(
                    package_id,
                    {
                        "success": True,
                        "authorization_url": "https://example.com/oauth",
                    },
                ),
            )
            return

        if (
            path.startswith("/api/webchat/v2/extensions/")
            and path.endswith("/setup")
            and request.method == "GET"
        ):
            package_id = unquote(
                path.removeprefix("/api/webchat/v2/extensions/").removesuffix("/setup")
            )
            if package_id in setup_get_responses_by_id:
                response = setup_get_responses_by_id[package_id]
                await fulfill_json(
                    route,
                    response.get("body", response),
                    status=response.get("status", 200),
                )
                return
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

        if (
            path.startswith("/api/webchat/v2/extensions/")
            and path.endswith("/setup")
            and request.method == "POST"
        ):
            package_id = unquote(
                path.removeprefix("/api/webchat/v2/extensions/").removesuffix("/setup")
            )
            payload = json.loads(request.post_data or "{}")
            setup_submit_requests.append({"package_id": package_id, "body": payload})
            response = setup_submit_responses_by_id.get(
                package_id,
                {"success": True, "message": f"{package_id} configured"},
            )
            if response.get("success") is not False:
                for extension in installed_extensions:
                    if extension.get("package_ref", {}).get("id") == package_id:
                        extension["authenticated"] = True
                        extension["needs_setup"] = False
                        extension["activation_status"] = "configured"
                        extension["onboarding_state"] = "configured"
            await fulfill_json(route, response)
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
            response = install_responses_by_id.get(
                package_id,
                {
                    "success": True,
                    "message": f"{(entry or {}).get('display_name') or package_id} installed",
                },
            )
            if response.get("success") is not False and entry and not any(
                extension.get("package_ref", {}).get("id") == package_id
                for extension in installed_extensions
            ):
                requires_setup = bool(entry.get("needs_setup") or entry.get("has_auth"))
                installed = dict(entry)
                installed.update(
                    {
                        "active": False,
                        "authenticated": False,
                        "has_auth": bool(entry.get("has_auth") or requires_setup),
                        "needs_setup": requires_setup,
                        "activation_status": (
                            "setup_required" if requires_setup else "installed"
                        ),
                        "onboarding_state": (
                            "setup_required" if requires_setup else "installed"
                        ),
                        "tools": entry.get("tools") or [],
                    }
                )
                installed.pop("installed", None)
                installed_extensions.append(installed)
            if response.get("success") is not False and entry:
                entry["installed"] = True
            await fulfill_json(route, response)
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
            response = activate_responses_by_id.get(
                package_id,
                {"success": True, "message": f"{package_id} activated"},
            )
            if response.get("success") is not False:
                for extension in installed_extensions:
                    if extension.get("package_ref", {}).get("id") == package_id:
                        extension["active"] = True
                        extension["activation_status"] = "active"
            await fulfill_json(
                route,
                response,
            )
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
            response = remove_responses_by_id.get(
                package_id,
                {"success": True, "message": f"{package_id} removed"},
            )
            if response.get("success") is not False:
                installed_extensions = [
                    extension
                    for extension in installed_extensions
                    if extension.get("package_ref", {}).get("id") != package_id
                ]
                for entry in registry_entries:
                    if entry.get("package_ref", {}).get("id") == package_id:
                        entry["installed"] = False
            await fulfill_json(route, response)
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
        "setup_submit_requests": setup_submit_requests,
        "oauth_start_requests": oauth_start_requests,
        "extension_list_requests": extension_list_requests,
        "registry_requests": registry_requests,
    }


def _card_by_title(page, title: str):
    return page.get_by_text(title, exact=True).locator(
        "xpath=ancestor::div[contains(@class, 'rounded-') and contains(@class, 'p-4')][1]"
    )


def _label_channel(**overrides):
    return {**LABEL_CHANNEL_BASE, **overrides}


def _manual_config_setup_payload() -> dict:
    return {
        "name": "config-tool",
        "kind": "wasm_tool",
        "secrets": [
            {
                "name": "API_TOKEN",
                "prompt": "API token",
                "provided": False,
                "optional": False,
                "auto_generate": False,
            }
        ],
        "fields": [],
        "onboarding": None,
    }


async def _open_card_menu(card):
    await card.get_by_label("More actions").click()
    return card.get_by_role("menu")


async def _capture_next_confirm(page, *, accept: bool):
    loop = asyncio.get_running_loop()
    dialog_future = loop.create_future()

    def handle_dialog(dialog):
        if not dialog_future.done():
            dialog_future.set_result({"type": dialog.type, "message": dialog.message})
        loop.create_task(dialog.accept() if accept else dialog.dismiss())

    page.once("dialog", handle_dialog)
    return dialog_future


async def _wait_for_request_count(requests: list, count: int, *, timeout: float = 5.0):
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        if len(requests) > count:
            return
        await asyncio.sleep(0.05)
    raise AssertionError(f"Timed out waiting for request count > {count}; got {len(requests)}")


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

        registry_tool_card = _card_by_title(page, "Registry Tool")
        await registry_tool_card.get_by_role("button", name="2 keywords").click()
        await expect(registry_tool_card.get_by_text("search")).to_be_visible()
        await expect(registry_tool_card.get_by_text("utility")).to_be_visible()

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


async def test_reborn_legacy_extensions_page_refetches_on_revisit(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        registry=[REGISTRY_TOOL],
    )
    try:
        page = harness["page"]
        await expect(page.get_by_text("Registry Tool")).to_be_visible(timeout=5000)
        first_extension_fetches = len(harness["extension_list_requests"])
        first_registry_fetches = len(harness["registry_requests"])
        assert first_extension_fetches >= 1
        assert first_registry_fetches >= 1

        await page.get_by_role("link", name="Settings").first.click()
        await page.wait_for_function(
            "() => location.pathname.startsWith('/v2/settings')",
            timeout=5000,
        )

        await page.get_by_role("link", name="Extensions").first.click()
        await expect(page.get_by_text("Registry Tool")).to_be_visible(timeout=5000)

        await _wait_for_request_count(
            harness["extension_list_requests"],
            first_extension_fetches,
        )
        await _wait_for_request_count(
            harness["registry_requests"],
            first_registry_fetches,
        )
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_registry_search_no_match(
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
        await expect(page.get_by_text("Registry MCP Server")).to_be_visible()

        await page.locator('input[placeholder^="Search extensions"]').fill(
            "xyznonexistent999"
        )

        await expect(page.get_by_text("No extensions match the filter.")).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("Registry Tool")).to_have_count(0)
        await expect(page.get_by_text("Registry MCP Server")).to_have_count(0)
        assert harness["install_requests"] == []
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_multiple_installs_remain_listed(
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
        await expect(page.get_by_text("Registry MCP Server")).to_be_visible()

        registry_tool_card = _card_by_title(page, "Registry Tool")
        await registry_tool_card.get_by_role("button", name="Install").click()
        await expect(page.get_by_text("Registry Tool installed")).to_be_visible(
            timeout=5000
        )

        registry_mcp_card = _card_by_title(page, "Registry MCP Server")
        await registry_mcp_card.get_by_role("button", name="Install").click()
        await expect(page.get_by_text("Registry MCP Server installed")).to_be_visible(
            timeout=5000
        )

        assert harness["install_requests"] == [
            {"package_ref": _package_ref("registry-tool")},
            {"package_ref": _package_ref("registry-mcp")},
        ]

        installed_tool = _card_by_title(page, "Registry Tool")
        installed_mcp = _card_by_title(page, "Registry MCP Server")
        await expect(installed_tool.get_by_text("installed", exact=True)).to_be_visible(
            timeout=5000
        )
        await expect(installed_mcp.get_by_text("installed", exact=True)).to_be_visible()
        await expect(
            installed_tool.get_by_role("button", name="Install")
        ).to_have_count(0)
        await expect(installed_mcp.get_by_role("button", name="Install")).to_have_count(
            0
        )
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_install_failure_keeps_registry_entry_available(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        registry=[REGISTRY_TOOL],
        install_responses={
            "registry-tool": {
                "success": False,
                "message": "Registry Tool is not available for this workspace.",
            }
        },
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Registry Tool")
        await expect(card).to_be_visible(timeout=5000)
        await expect(card.get_by_text("available", exact=True)).to_be_visible()
        await card.get_by_role("button", name="Install").click()

        await expect(
            page.get_by_text("Registry Tool is not available for this workspace.")
        ).to_be_visible(timeout=5000)
        assert harness["install_requests"] == [
            {"package_ref": _package_ref("registry-tool")}
        ]

        await expect(card.get_by_text("available", exact=True)).to_be_visible(timeout=5000)
        await expect(card.get_by_role("button", name="Install")).to_have_count(1)
        await expect(card.get_by_text("installed", exact=True)).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_install_auth_url_opens_popup(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        registry=[REGISTRY_TOOL],
        install_responses={
            "registry-tool": {
                "success": True,
                "message": "Registry Tool installed",
                "auth_url": "HTTPS://example.com/oauth?state=install",
            }
        },
    )
    try:
        page = harness["page"]
        await page.evaluate(
            """
            () => {
              window.__openedUrls = [];
              window.open = (url) => {
                window.__openedUrls.push(url);
                return null;
              };
            }
            """
        )

        card = _card_by_title(page, "Registry Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Install").click()

        await expect(page.get_by_text("Registry Tool installed")).to_be_visible(
            timeout=5000
        )
        await page.wait_for_function(
            "() => window.__openedUrls.some((url) => /^https:\\/\\//i.test(url))",
            timeout=5000,
        )
        opened = await page.evaluate("() => window.__openedUrls")
        assert opened[-1].lower().startswith("https://example.com/oauth"), opened
        assert harness["install_requests"] == [
            {"package_ref": _package_ref("registry-tool")}
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_install_auth_url_requires_https(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        registry=[REGISTRY_TOOL],
        install_responses={
            "registry-tool": {
                "success": True,
                "message": "Registry Tool installed",
                "auth_url": "javascript:alert('install-xss')",
            }
        },
    )
    try:
        page = harness["page"]
        await page.evaluate(
            """
            () => {
              window.__openedUrls = [];
              window.open = (url) => {
                window.__openedUrls.push(url);
                return null;
              };
            }
            """
        )

        card = _card_by_title(page, "Registry Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Install").click()

        await expect(page.get_by_text("Authentication URL must use HTTPS.")).to_be_visible(
            timeout=5000
        )
        assert await page.evaluate("() => window.__openedUrls") == []
        assert harness["install_requests"] == [
            {"package_ref": _package_ref("registry-tool")}
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_install_setup_required_channel_opens_configure_modal(
    reborn_v2_server, reborn_v2_browser
):
    setup_channel = {
        **AVAILABLE_CHANNEL,
        "has_auth": True,
        "needs_setup": True,
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        registry=[setup_channel],
        setup_payloads={
            "slack-channel": {
                "name": "slack-channel",
                "kind": "wasm_channel",
                "secrets": [
                    {
                        "name": "SLACK_BOT_TOKEN",
                        "prompt": "Slack bot token",
                        "provided": False,
                        "optional": False,
                        "auto_generate": False,
                    }
                ],
                "fields": [],
                "onboarding": {
                    "credential_instructions": "Enter the Slack channel token.",
                    "credential_next_step": "Save to continue pairing.",
                },
            }
        },
        tab="channels",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Slack Channel")
        await expect(card).to_be_visible(timeout=5000)

        await card.get_by_role("button", name="Install").click()

        await expect(page.get_by_text("Slack Channel installed")).to_be_visible(
            timeout=5000
        )
        assert harness["install_requests"] == [
            {"package_ref": _package_ref("slack-channel")}
        ]
        await expect(
            page.get_by_role("heading", name="Configure Slack Channel")
        ).to_be_visible(timeout=5000)
        await expect(page.get_by_text("Enter the Slack channel token.")).to_be_visible()
        await expect(page.get_by_text("Slack bot token")).to_be_visible()
        await expect(page.locator('input[type="password"]')).to_have_count(1)
        assert harness["setup_submit_requests"] == []
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
        dialog_future = await _capture_next_confirm(page, accept=True)
        await page.get_by_role("menuitem", name="Remove").click()
        dialog = await asyncio.wait_for(dialog_future, timeout=5)
        assert dialog["type"] == "confirm"
        assert "Active Tool" in dialog["message"]
        await expect(page.get_by_text("Active Tool removed")).to_be_visible(timeout=5000)
        assert harness["remove_requests"] == ["active-tool"]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_activate_success_marks_extension_active_with_capabilities(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[INACTIVE_MCP],
        tab="mcp",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Inactive MCP")
        await expect(card).to_be_visible(timeout=5000)
        await expect(card.get_by_text("installed", exact=True)).to_be_visible()

        await card.get_by_role("button", name="Activate").click()
        await expect(page.get_by_text("inactive-mcp activated")).to_be_visible(timeout=5000)
        assert harness["activate_requests"] == ["inactive-mcp"]

        await expect(card.get_by_text("active", exact=True)).to_be_visible(timeout=5000)
        await expect(card.get_by_role("button", name="Activate")).to_have_count(0)
        await card.get_by_role("button", name="1 capability").click()
        await expect(card.get_by_text("lookup", exact=True)).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_remove_cancel_keeps_card(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[ACTIVE_TOOL],
        tab="registry",
    )
    try:
        page = harness["page"]
        active_card = _card_by_title(page, "Active Tool")
        await expect(active_card).to_be_visible(timeout=5000)

        await active_card.get_by_label("More actions").click()
        dialog_future = await _capture_next_confirm(page, accept=False)
        await page.get_by_role("menuitem", name="Remove").click()
        dialog = await asyncio.wait_for(dialog_future, timeout=5)
        assert dialog["type"] == "confirm"
        assert "Active Tool" in dialog["message"]

        await expect(active_card).to_be_visible(timeout=5000)
        assert harness["remove_requests"] == []
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_remove_failure_keeps_card(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[ACTIVE_TOOL],
        registry=[REGISTRY_TOOL],
        remove_responses={
            "active-tool": {
                "success": False,
                "message": "Active Tool is still handling an active run.",
            }
        },
        tab="registry",
    )
    try:
        page = harness["page"]
        active_card = _card_by_title(page, "Active Tool")
        await expect(active_card).to_be_visible(timeout=5000)
        await expect(active_card.get_by_text("active", exact=True)).to_be_visible()

        await active_card.get_by_label("More actions").click()
        dialog_future = await _capture_next_confirm(page, accept=True)
        await page.get_by_role("menuitem", name="Remove").click()
        dialog = await asyncio.wait_for(dialog_future, timeout=5)
        assert dialog["type"] == "confirm"
        assert "Active Tool" in dialog["message"]

        await expect(
            page.get_by_text("Active Tool is still handling an active run.")
        ).to_be_visible(timeout=5000)
        assert harness["remove_requests"] == ["active-tool"]

        await expect(active_card).to_be_visible(timeout=5000)
        await expect(active_card.get_by_text("active", exact=True)).to_be_visible()
        await expect(active_card.get_by_label("More actions")).to_have_count(1)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_remove_clears_installed_state(
    reborn_v2_server, reborn_v2_browser
):
    active_tool_registry_entry = {
        "package_ref": _package_ref("active-tool"),
        "display_name": "Active Tool",
        "kind": "wasm_tool",
        "description": "An installed WASM tool extension",
        "keywords": ["search"],
        "installed": True,
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[ACTIVE_TOOL],
        registry=[active_tool_registry_entry],
        tab="registry",
    )
    try:
        page = harness["page"]
        active_card = _card_by_title(page, "Active Tool")
        await expect(active_card).to_be_visible(timeout=5000)
        await expect(active_card.get_by_text("active", exact=True)).to_be_visible()
        await expect(page.get_by_text("Installed", exact=True)).to_be_visible()

        await active_card.get_by_label("More actions").click()
        dialog_future = await _capture_next_confirm(page, accept=True)
        await page.get_by_role("menuitem", name="Remove").click()
        dialog = await asyncio.wait_for(dialog_future, timeout=5)
        assert dialog["type"] == "confirm"
        assert "Active Tool" in dialog["message"]
        await expect(page.get_by_text("Active Tool removed")).to_be_visible(timeout=5000)
        assert harness["remove_requests"] == ["active-tool"]

        await expect(page.get_by_text("Installed", exact=True)).to_have_count(0)
        available_card = _card_by_title(page, "Active Tool")
        await expect(available_card.get_by_text("available", exact=True)).to_be_visible(
            timeout=5000
        )
        await expect(
            available_card.get_by_role("button", name="Install")
        ).to_be_visible()
        await expect(available_card.get_by_label("More actions")).to_have_count(0)
        await expect(available_card.get_by_text("active", exact=True)).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_reinstall_after_remove_requires_setup_again(
    reborn_v2_server, reborn_v2_browser
):
    configured_tool = {
        **CONFIG_TOOL,
        "active": True,
        "authenticated": True,
        "needs_setup": False,
        "activation_status": "active",
        "onboarding_state": "ready",
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[configured_tool],
        registry=[CONFIG_TOOL_REGISTRY],
        setup_payloads={"config-tool": _manual_config_setup_payload()},
        tab="installed",
    )
    try:
        page = harness["page"]
        configured_card = _card_by_title(page, "Config Tool")
        await expect(configured_card).to_be_visible(timeout=5000)
        await _open_card_menu(configured_card)
        await expect(
            page.get_by_role("menuitem", name="Reconfigure", exact=True)
        ).to_have_count(1)

        dialog_future = await _capture_next_confirm(page, accept=True)
        await page.get_by_role("menuitem", name="Remove").click()
        dialog = await asyncio.wait_for(dialog_future, timeout=5)
        assert dialog["type"] == "confirm"
        assert "Config Tool" in dialog["message"]
        await expect(page.get_by_text("Config Tool removed")).to_be_visible(timeout=5000)
        assert harness["remove_requests"] == ["config-tool"]

        await page.goto(
            f"{reborn_v2_server}/v2/extensions/registry?token={REBORN_V2_AUTH_TOKEN}"
        )
        available_card = _card_by_title(page, "Config Tool")
        await expect(available_card).to_be_visible(timeout=5000)
        await expect(available_card.get_by_role("button", name="Install")).to_be_visible()
        await available_card.get_by_role("button", name="Install").click()
        await expect(page.get_by_text("Config Tool installed")).to_be_visible(timeout=5000)
        assert harness["install_requests"] == [
            {"package_ref": _package_ref("config-tool")}
        ]

        reinstalled_card = _card_by_title(page, "Config Tool")
        await expect(reinstalled_card.get_by_text("setup needed")).to_be_visible(
            timeout=5000
        )
        await expect(reinstalled_card.get_by_role("button", name="Configure")).to_have_count(
            1
        )
        await expect(
            reinstalled_card.get_by_role("button", name="Reconfigure")
        ).to_have_count(0)

        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_be_visible(timeout=5000)
        await page.locator('input[type="password"]').first.fill("fresh-token")
        await page.get_by_role("button", name="Save").click()
        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_have_count(0)
        assert harness["setup_submit_requests"] == [
            {
                "package_id": "config-tool",
                "body": {
                    "action": "submit",
                    "payload": {
                        "secrets": {"API_TOKEN": "fresh-token"},
                        "fields": {},
                    },
                },
            }
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_extensions_null_tools_render_no_capabilities(
    reborn_v2_server, reborn_v2_browser
):
    null_tools_extension = {**ACTIVE_TOOL, "tools": None}
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[null_tools_extension],
        tab="registry",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Active Tool")
        await expect(card).to_be_visible(timeout=5000)
        await expect(card.get_by_text("No capabilities")).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_activate_failure_keeps_extension_inactive(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[INACTIVE_MCP],
        activate_responses={
            "inactive-mcp": {
                "success": False,
                "message": "Configure credentials before activation.",
            }
        },
        tab="mcp",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Inactive MCP")
        await expect(card).to_be_visible(timeout=5000)
        await expect(card.get_by_text("installed", exact=True)).to_be_visible()
        await card.get_by_role("button", name="Activate").click()

        await expect(
            page.get_by_text("Configure credentials before activation.")
        ).to_be_visible(timeout=5000)
        assert harness["activate_requests"] == ["inactive-mcp"]

        await expect(card.get_by_text("installed", exact=True)).to_be_visible(timeout=5000)
        await expect(card.get_by_role("button", name="Activate")).to_have_count(1)
        await expect(card.get_by_text("active", exact=True)).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_activate_auth_url_requires_https(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[INACTIVE_MCP],
        activate_responses={
            "inactive-mcp": {
                "success": True,
                "message": "Inactive MCP activated",
                "auth_url": "javascript:alert('xss')",
            }
        },
        tab="mcp",
    )
    try:
        page = harness["page"]
        await page.evaluate(
            """
            () => {
              window.__openedUrls = [];
              window.open = (url) => {
                window.__openedUrls.push(url);
                return null;
              };
            }
            """
        )

        card = _card_by_title(page, "Inactive MCP")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Activate").click()

        await expect(page.get_by_text("Authentication URL must use HTTPS.")).to_be_visible(
            timeout=5000
        )
        assert await page.evaluate("() => window.__openedUrls") == []
        assert harness["activate_requests"] == ["inactive-mcp"]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_activate_auth_url_accepts_uppercase_https(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[INACTIVE_MCP],
        activate_responses={
            "inactive-mcp": {
                "success": True,
                "message": "Inactive MCP activated",
                "auth_url": "HTTPS://example.com/oauth?state=abc",
            }
        },
        tab="mcp",
    )
    try:
        page = harness["page"]
        await page.evaluate(
            """
            () => {
              window.__openedUrls = [];
              window.open = (url) => {
                window.__openedUrls.push(url);
                return null;
              };
            }
            """
        )

        card = _card_by_title(page, "Inactive MCP")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Activate").click()

        await page.wait_for_function(
            "() => window.__openedUrls.some((url) => /^https:\\/\\//i.test(url))",
            timeout=5000,
        )
        opened = await page.evaluate("() => window.__openedUrls")
        assert opened[-1].lower().startswith("https://example.com/oauth"), opened
        assert harness["activate_requests"] == ["inactive-mcp"]
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


async def test_reborn_legacy_channel_pairing_redeems_trimmed_code(
    reborn_v2_server, reborn_v2_browser
):
    pairing_channel = {
        **TELEGRAM_CHANNEL_SETUP,
        "active": False,
        "authenticated": True,
        "activation_status": "pairing",
        "onboarding_state": "pairing_required",
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[pairing_channel],
        tab="channels",
    )
    try:
        page = harness["page"]
        redeem_requests: list[dict] = []

        async def handle_redeem(route):
            redeem_requests.append(json.loads(route.request.post_data or "{}"))
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    {
                        "provider": "telegram",
                        "provider_user_id": "123456789",
                    }
                ),
            )

        await page.route("**/api/webchat/v2/extensions/pairing/redeem", handle_redeem)

        card = _card_by_title(page, "Telegram")
        await expect(card).to_be_visible(timeout=5000)
        await expect(card.get_by_text("pairing", exact=True)).to_be_visible()
        await expect(card.get_by_role("button", name="Activate")).to_have_count(0)

        section = page.locator("[data-testid='pairing-section']").first
        await expect(section).to_be_visible(timeout=5000)
        input_field = section.locator("[data-testid='pairing-code-input']")
        await input_field.fill("  PAIR-1234  ")
        await section.locator("[data-testid='pairing-submit']").click()

        await expect(section.locator("[data-testid='pairing-success']")).to_contain_text(
            "Pairing complete.", timeout=5000
        )
        await expect(input_field).to_have_value("")
        assert redeem_requests == [{"channel": "telegram", "code": "PAIR-1234"}]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_channel_pairing_enter_key_submits_code(
    reborn_v2_server, reborn_v2_browser
):
    pairing_channel = {
        **TELEGRAM_CHANNEL_SETUP,
        "active": False,
        "authenticated": True,
        "activation_status": "pairing",
        "onboarding_state": "pairing_required",
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[pairing_channel],
        tab="channels",
    )
    try:
        page = harness["page"]
        redeem_requests: list[dict] = []

        async def handle_redeem(route):
            redeem_requests.append(json.loads(route.request.post_data or "{}"))
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    {
                        "provider": "telegram",
                        "provider_user_id": "987654321",
                    }
                ),
            )

        await page.route("**/api/webchat/v2/extensions/pairing/redeem", handle_redeem)

        section = page.locator("[data-testid='pairing-section']").first
        await expect(section).to_be_visible(timeout=5000)
        input_field = section.locator("[data-testid='pairing-code-input']")
        await input_field.fill("  pair-5678  ")
        await input_field.press("Enter")

        await expect(section.locator("[data-testid='pairing-success']")).to_contain_text(
            "Pairing complete.", timeout=5000
        )
        await expect(input_field).to_have_value("")
        assert redeem_requests == [{"channel": "telegram", "code": "PAIR-5678"}]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_channel_pairing_failure_keeps_code_for_retry(
    reborn_v2_server, reborn_v2_browser
):
    pairing_channel = {
        **TELEGRAM_CHANNEL_SETUP,
        "active": False,
        "authenticated": True,
        "activation_status": "pairing",
        "onboarding_state": "pairing_required",
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[pairing_channel],
        tab="channels",
    )
    try:
        page = harness["page"]
        redeem_requests: list[dict] = []

        async def handle_redeem(route):
            redeem_requests.append(json.loads(route.request.post_data or "{}"))
            await route.fulfill(
                status=400,
                content_type="application/json",
                body=json.dumps({"error": "Invalid pairing code"}),
            )

        await page.route("**/api/webchat/v2/extensions/pairing/redeem", handle_redeem)

        section = page.locator("[data-testid='pairing-section']").first
        await expect(section).to_be_visible(timeout=5000)
        input_field = section.locator("[data-testid='pairing-code-input']")
        await input_field.fill("bad-code")
        await section.locator("[data-testid='pairing-submit']").click()

        await expect(section.locator("[data-testid='pairing-error']")).to_contain_text(
            "Invalid pairing code", timeout=5000
        )
        await expect(input_field).to_have_value("bad-code")
        assert redeem_requests == [{"channel": "telegram", "code": "BAD-CODE"}]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_saves_manual_secret_and_fields(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[CONFIG_TOOL],
        setup_payloads={
            "config-tool": {
                "name": "config-tool",
                "kind": "wasm_tool",
                "secrets": [
                    {
                        "name": "API_TOKEN",
                        "prompt": "API token",
                        "provided": False,
                        "optional": False,
                        "auto_generate": False,
                    },
                    {
                        "name": "OPTIONAL_SECRET",
                        "prompt": "Optional secret",
                        "provided": True,
                        "optional": True,
                        "auto_generate": False,
                    },
                ],
                "fields": [
                    {
                        "name": "workspace",
                        "prompt": "Workspace",
                        "placeholder": "team-slug",
                        "optional": False,
                    }
                ],
                "onboarding": {
                    "credential_instructions": "Paste credentials from the provider.",
                    "credential_next_step": "Save to continue.",
                },
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Config Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()

        await expect(page.get_by_role("heading", name="Configure Config Tool")).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("Paste credentials from the provider.")).to_be_visible()
        await page.locator('input[type="password"]').nth(0).fill("secret-token")
        await page.locator('input[type="password"]').nth(1).fill("rotated-secret")
        await page.locator('input[type="text"][placeholder="team-slug"]').fill("team-a")
        await page.get_by_role("button", name="Save").click()

        await expect(page.get_by_role("heading", name="Configure Config Tool")).to_have_count(0)
        assert harness["setup_submit_requests"] == [
            {
                "package_id": "config-tool",
                "body": {
                    "action": "submit",
                    "payload": {
                        "secrets": {
                            "API_TOKEN": "secret-token",
                            "OPTIONAL_SECRET": "rotated-secret",
                        },
                        "fields": {"workspace": "team-a"},
                    },
                },
            }
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_renders_field_variants(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[CONFIG_TOOL],
        setup_payloads={
            "config-tool": {
                "name": "config-tool",
                "kind": "wasm_tool",
                "secrets": [
                    {
                        "name": "API_TOKEN",
                        "prompt": "Enter API key",
                        "provided": False,
                        "optional": False,
                        "auto_generate": False,
                    },
                    {
                        "name": "EXISTING_TOKEN",
                        "prompt": "Existing token",
                        "provided": True,
                        "optional": False,
                        "auto_generate": False,
                    },
                    {
                        "name": "OPTIONAL_SECRET",
                        "prompt": "Optional secret",
                        "provided": False,
                        "optional": True,
                        "auto_generate": False,
                    },
                    {
                        "name": "AUTO_SECRET",
                        "prompt": "Generated secret",
                        "provided": False,
                        "optional": False,
                        "auto_generate": True,
                    },
                ],
                "fields": [
                    {
                        "name": "workspace",
                        "prompt": "Workspace",
                        "placeholder": "team-slug",
                        "optional": True,
                    }
                ],
                "onboarding": None,
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Config Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()

        modal = page.get_by_role("dialog", name="Configure Config Tool")
        await expect(modal).to_be_visible(timeout=5000)
        await expect(modal).to_contain_text("Enter API key")
        await expect(modal).to_contain_text("Existing token")
        await expect(modal).to_contain_text("Optional secret")
        await expect(modal).to_contain_text("Generated secret")
        await expect(modal).to_contain_text("Workspace")
        await expect(modal.get_by_text("configured", exact=True)).to_be_visible()
        await expect(modal.get_by_text("optional", exact=True)).to_have_count(2)
        await expect(modal.get_by_text("Auto-generated if left blank")).to_be_visible()
        await expect(
            modal.locator('input[type="password"][placeholder*="leave blank to keep"]')
        ).to_have_count(1)
        await expect(modal.locator('input[type="text"][placeholder="team-slug"]')).to_be_visible()

        await modal.get_by_role("button", name="Cancel").click()
        assert harness["setup_submit_requests"] == []
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_setup_url_requires_https(
    reborn_v2_server, reborn_v2_browser
):
    async def open_with_setup_url(setup_url: str):
        harness = await _open_mocked_extensions_page(
            reborn_v2_server,
            reborn_v2_browser,
            installed=[CONFIG_TOOL],
            setup_payloads={
                "config-tool": {
                    "name": "config-tool",
                    "kind": "wasm_tool",
                    "secrets": [
                        {
                            "name": "API_TOKEN",
                            "prompt": "API token",
                            "provided": False,
                            "optional": False,
                            "auto_generate": False,
                        }
                    ],
                    "fields": [],
                    "onboarding": {
                        "credential_instructions": "Create a token before continuing.",
                        "setup_url": setup_url,
                    },
                }
            },
            tab="installed",
        )
        page = harness["page"]
        card = _card_by_title(page, "Config Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()
        modal = page.get_by_role("dialog", name="Configure Config Tool")
        await expect(modal).to_be_visible(timeout=5000)
        return harness, modal

    https_harness, https_modal = await open_with_setup_url(
        "https://platform.example.test/api-keys"
    )
    try:
        link = https_modal.get_by_role("link", name="Get credentials")
        await expect(link).to_be_visible()
        await expect(link).to_have_attribute(
            "href", "https://platform.example.test/api-keys"
        )
        await expect(link).to_have_attribute("target", "_blank")
        await expect(link).to_have_attribute("rel", "noopener noreferrer")
    finally:
        await https_harness["context"].close()

    bad_harness, bad_modal = await open_with_setup_url("javascript:alert(1)")
    try:
        await expect(bad_modal.get_by_role("link", name="Get credentials")).to_have_count(0)
        await expect(bad_modal.locator('a[href^="javascript:"]')).to_have_count(0)
    finally:
        await bad_harness["context"].close()


async def test_reborn_legacy_configure_handles_selector_sensitive_package_ids(
    reborn_v2_server, reborn_v2_browser
):
    package_id = 'quoted "tool" name'
    quoted_tool = {
        **CONFIG_TOOL,
        "display_name": 'Quoted "Tool" Name',
        "package_ref": _package_ref(package_id),
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[quoted_tool],
        setup_payloads={
            package_id: {
                "name": package_id,
                "kind": "wasm_tool",
                "secrets": [
                    {
                        "name": "API_TOKEN",
                        "prompt": "API token",
                        "provided": False,
                        "optional": False,
                        "auto_generate": False,
                    }
                ],
                "fields": [],
                "onboarding": None,
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, 'Quoted "Tool" Name')
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()

        modal = page.get_by_role("dialog", name='Configure Quoted "Tool" Name')
        await expect(modal).to_be_visible(timeout=5000)
        await modal.locator('input[type="password"]').fill("quoted-secret")
        await modal.get_by_role("button", name="Save").click()

        await expect(modal).to_have_count(0)
        assert harness["setup_submit_requests"] == [
            {
                "package_id": package_id,
                "body": {
                    "action": "submit",
                    "payload": {
                        "secrets": {"API_TOKEN": "quoted-secret"},
                        "fields": {},
                    },
                },
            }
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_blank_existing_secret_is_not_submitted(
    reborn_v2_server, reborn_v2_browser
):
    configured_tool = {
        **CONFIG_TOOL,
        "active": True,
        "authenticated": True,
        "needs_setup": False,
        "activation_status": "active",
        "onboarding_state": "ready",
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[configured_tool],
        setup_payloads={
            "config-tool": {
                "name": "config-tool",
                "kind": "wasm_tool",
                "secrets": [
                    {
                        "name": "API_TOKEN",
                        "prompt": "API token",
                        "provided": True,
                        "optional": False,
                        "auto_generate": False,
                    }
                ],
                "fields": [],
                "onboarding": None,
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Config Tool")
        await expect(card).to_be_visible(timeout=5000)
        await _open_card_menu(card)
        await page.get_by_role("menuitem", name="Reconfigure", exact=True).click()

        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_be_visible(timeout=5000)
        await expect(page.get_by_text("configured")).to_be_visible()
        await expect(page.locator('input[type="password"]').first).to_have_attribute(
            "placeholder", "••••••• (leave blank to keep)"
        )

        await page.get_by_role("button", name="Save").click()
        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_have_count(0)
        assert harness["setup_submit_requests"] == [
            {
                "package_id": "config-tool",
                "body": {
                    "action": "submit",
                    "payload": {
                        "secrets": {},
                        "fields": {},
                    },
                },
            }
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_save_failure_stays_open(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[CONFIG_TOOL],
        setup_payloads={
            "config-tool": {
                "name": "config-tool",
                "kind": "wasm_tool",
                "secrets": [
                    {
                        "name": "API_TOKEN",
                        "prompt": "API token",
                        "provided": False,
                        "optional": False,
                        "auto_generate": False,
                    }
                ],
                "fields": [],
                "onboarding": None,
            }
        },
        setup_submit_responses={
            "config-tool": {"success": False, "message": "Invalid API token"}
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Config Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()

        await expect(page.get_by_role("heading", name="Configure Config Tool")).to_be_visible(
            timeout=5000
        )
        await page.locator('input[type="password"]').first.fill("bad-token")
        await page.get_by_role("button", name="Save").click()

        await expect(page.get_by_text("Invalid API token")).to_be_visible(timeout=5000)
        await expect(page.get_by_role("heading", name="Configure Config Tool")).to_be_visible()
        assert harness["setup_submit_requests"][0]["body"]["payload"]["secrets"] == {
            "API_TOKEN": "bad-token"
        }
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_setup_load_failure_is_visible(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[CONFIG_TOOL],
        setup_get_responses={
            "config-tool": {
                "status": 404,
                "body": {
                    "error": "not_found",
                    "kind": "not_found",
                    "field": "package_ref",
                },
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Config Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()

        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_be_visible(timeout=5000)
        await expect(page.get_by_text("Failed to load setup:")).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("Not found (package_ref)")).to_be_visible()
        await expect(page.get_by_role("button", name="Save")).to_have_count(0)
        assert harness["setup_submit_requests"] == []
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_auto_resolved_setup_has_no_manual_fields(
    reborn_v2_server, reborn_v2_browser
):
    auto_resolved_tool = {
        **OAUTH_TOOL,
        "display_name": "Auto Resolved OAuth Tool",
        "package_ref": _package_ref("auto-resolved-oauth-tool"),
    }
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[auto_resolved_tool],
        setup_payloads={
            "auto-resolved-oauth-tool": {
                "name": "auto-resolved-oauth-tool",
                "kind": "wasm_tool",
                "secrets": [],
                "fields": [],
                "onboarding": None,
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Auto Resolved OAuth Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()

        await expect(
            page.get_by_role("heading", name="Configure Auto Resolved OAuth Tool")
        ).to_be_visible(timeout=5000)
        await expect(
            page.get_by_text("No configuration required for this extension.")
        ).to_be_visible()
        await expect(page.get_by_role("button", name="Save")).to_have_count(0)
        await expect(page.get_by_role("button", name="Authorize")).to_have_count(0)
        await expect(page.locator('input[type="password"]')).to_have_count(0)
        assert harness["setup_submit_requests"] == []
        assert harness["oauth_start_requests"] == []
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_dismisses_without_saving(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[CONFIG_TOOL],
        setup_payloads={"config-tool": _manual_config_setup_payload()},
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Config Tool")
        await expect(card).to_be_visible(timeout=5000)

        async def open_modal():
            await card.get_by_role("button", name="Configure").click()
            await expect(
                page.get_by_role("heading", name="Configure Config Tool")
            ).to_be_visible(timeout=5000)

        await open_modal()
        await page.get_by_role("button", name="Cancel").click()
        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_have_count(0)

        await open_modal()
        await page.mouse.click(5, 5)
        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_have_count(0)

        await open_modal()
        await page.keyboard.press("Escape")
        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_have_count(0)

        assert harness["setup_submit_requests"] == []
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_modal_enter_key_submits(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[CONFIG_TOOL],
        setup_payloads={"config-tool": _manual_config_setup_payload()},
        tab="installed",
    )
    try:
        page = harness["page"]
        card = _card_by_title(page, "Config Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()

        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_be_visible(timeout=5000)
        await page.locator('input[type="password"]').first.fill("enter-token")
        await page.locator('input[type="password"]').first.press("Enter")

        await expect(
            page.get_by_role("heading", name="Configure Config Tool")
        ).to_have_count(0)
        assert harness["setup_submit_requests"] == [
            {
                "package_id": "config-tool",
                "body": {
                    "action": "submit",
                    "payload": {
                        "secrets": {"API_TOKEN": "enter-token"},
                        "fields": {},
                    },
                },
            }
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_telegram_token_configure_preserves_token_characters(
    reborn_v2_server, reborn_v2_browser
):
    token = "123456789:ABCdef_GHI-jkl_mnop-QRSTuvwxyz"
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[TELEGRAM_CHANNEL_SETUP],
        setup_payloads={
            "telegram": {
                "name": "telegram",
                "kind": "wasm_channel",
                "secrets": [
                    {
                        "name": "telegram_bot_token",
                        "prompt": "Telegram Bot Token",
                        "provided": False,
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
        card = _card_by_title(page, "Telegram")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()

        await expect(
            page.get_by_role("heading", name="Configure Telegram")
        ).to_be_visible(timeout=5000)
        await expect(page.get_by_text("Telegram Bot Token")).to_be_visible()
        await page.locator('input[type="password"]').first.fill(token)
        await page.get_by_role("button", name="Save").click()

        await expect(
            page.get_by_role("heading", name="Configure Telegram")
        ).to_have_count(0)
        assert harness["setup_submit_requests"] == [
            {
                "package_id": "telegram",
                "body": {
                    "action": "submit",
                    "payload": {
                        "secrets": {"telegram_bot_token": token},
                        "fields": {},
                    },
                },
            }
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_oauth_requires_https_authorization_url(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[OAUTH_TOOL],
        setup_payloads={
            "oauth-tool": {
                "name": "oauth-tool",
                "kind": "wasm_tool",
                "secrets": [
                    {
                        "name": "GOOGLE_AUTH",
                        "prompt": "Google account",
                        "provider": "google",
                        "provided": False,
                        "optional": False,
                        "auto_generate": False,
                        "setup": {
                            "kind": "oauth",
                            "account_label": "QA account",
                            "scopes": ["email"],
                            "invocation_id": "inv-1",
                        },
                    }
                ],
                "fields": [],
                "onboarding": None,
            }
        },
        oauth_start_responses={
            "oauth-tool": {
                "success": True,
                "authorization_url": "javascript:alert('xss')",
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        await page.evaluate(
            """
            () => {
              window.__openedUrls = [];
              window.open = (url) => {
                const popup = {
                  closed: false,
                  close() { this.closed = true; },
                  location: {
                    _href: url,
                    get href() { return this._href; },
                    set href(value) {
                      this._href = value;
                      window.__openedUrls.push(value);
                    },
                  },
                };
                window.__openedUrls.push(url);
                return popup;
              };
            }
            """
        )

        card = _card_by_title(page, "OAuth Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()
        await page.get_by_role("button", name="Authorize").click()

        await expect(page.get_by_text("Authorization URL must use HTTPS.")).to_be_visible(
            timeout=5000
        )
        opened = await page.evaluate("() => window.__openedUrls")
        assert opened == ["about:blank"]
        assert harness["oauth_start_requests"][0]["body"]["provider"] == "google"
        assert harness["oauth_start_requests"][0]["body"]["scopes"] == ["email"]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_oauth_start_failure_stays_visible(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[OAUTH_TOOL],
        setup_payloads={
            "oauth-tool": {
                "name": "oauth-tool",
                "kind": "wasm_tool",
                "secrets": [
                    {
                        "name": "GOOGLE_AUTH",
                        "prompt": "Google account",
                        "provider": "google",
                        "provided": False,
                        "optional": False,
                        "auto_generate": False,
                        "setup": {
                            "kind": "oauth",
                            "account_label": "QA account",
                            "scopes": ["email"],
                            "invocation_id": "inv-1",
                        },
                    }
                ],
                "fields": [],
                "onboarding": None,
            }
        },
        oauth_start_responses={
            "oauth-tool": {
                "success": False,
                "message": "Google OAuth is unavailable for this workspace.",
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        await page.evaluate(
            """
            () => {
              window.__openedPopups = [];
              window.open = (url) => {
                const popup = {
                  closed: false,
                  close() { this.closed = true; },
                  location: {
                    _href: url,
                    get href() { return this._href; },
                    set href(value) { this._href = value; },
                  },
                };
                window.__openedPopups.push(popup);
                return popup;
              };
            }
            """
        )

        card = _card_by_title(page, "OAuth Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()
        await page.get_by_role("button", name="Authorize").click()

        await expect(
            page.get_by_text("Google OAuth is unavailable for this workspace.")
        ).to_be_visible(timeout=5000)
        await expect(
            page.get_by_role("heading", name="Configure OAuth Tool")
        ).to_be_visible()
        assert harness["oauth_start_requests"][0]["body"]["provider"] == "google"
        assert await page.evaluate("() => window.__openedPopups[0].closed") is True
    finally:
        await harness["context"].close()


async def test_reborn_legacy_configure_oauth_accepts_uppercase_https_url(
    reborn_v2_server, reborn_v2_browser
):
    authorization_url = (
        "HTTPS://accounts.google.com/o/oauth2/v2/auth?"
        "client_id=client-123.apps.googleusercontent.com"
        "&response_type=code"
        "&redirect_uri=https%3A%2F%2Freborn.example.test%2Foauth%2Fcallback"
        "&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly+email"
        "&state=state-abc-123"
        "&access_type=offline"
        "&prompt=consent"
    )
    harness = await _open_mocked_extensions_page(
        reborn_v2_server,
        reborn_v2_browser,
        installed=[OAUTH_TOOL],
        setup_payloads={
            "oauth-tool": {
                "name": "oauth-tool",
                "kind": "wasm_tool",
                "secrets": [
                    {
                        "name": "GOOGLE_AUTH",
                        "prompt": "Google account",
                        "provider": "google",
                        "provided": False,
                        "optional": False,
                        "auto_generate": False,
                        "setup": {"kind": "oauth", "scopes": ["email"]},
                    }
                ],
                "fields": [],
                "onboarding": None,
            }
        },
        oauth_start_responses={
            "oauth-tool": {
                "success": True,
                "authorization_url": authorization_url,
            }
        },
        tab="installed",
    )
    try:
        page = harness["page"]
        await page.evaluate(
            """
            () => {
              window.__openedUrls = [];
              window.open = (url) => {
                const popup = {
                  closed: false,
                  close() { this.closed = true; },
                  location: {
                    _href: url,
                    get href() { return this._href; },
                    set href(value) {
                      this._href = value;
                      window.__openedUrls.push(value);
                    },
                  },
                };
                window.__openedUrls.push(url);
                return popup;
              };
            }
            """
        )

        card = _card_by_title(page, "OAuth Tool")
        await expect(card).to_be_visible(timeout=5000)
        await card.get_by_role("button", name="Configure").click()
        await page.get_by_role("button", name="Authorize").click()

        await page.wait_for_function(
            "() => window.__openedUrls.some((url) => /^https:\\/\\//i.test(url))",
            timeout=5000,
        )
        opened = await page.evaluate("() => window.__openedUrls")
        opened_url = opened[-1]
        parsed = urlparse(opened_url)
        params = parse_qs(parsed.query)
        assert parsed.scheme.lower() == "https", opened
        assert parsed.netloc == "accounts.google.com", opened
        assert parsed.path == "/o/oauth2/v2/auth", opened
        assert "client_id" in params, opened
        assert "clientid" not in params, opened
        assert params["client_id"] == ["client-123.apps.googleusercontent.com"]
        assert params["response_type"] == ["code"]
        assert params["redirect_uri"] == [
            "https://reborn.example.test/oauth/callback"
        ]
        assert params["scope"] == [
            "https://www.googleapis.com/auth/gmail.readonly email"
        ]
        assert params["state"] == ["state-abc-123"]
        assert params["access_type"] == ["offline"]
        assert params["prompt"] == ["consent"]
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
