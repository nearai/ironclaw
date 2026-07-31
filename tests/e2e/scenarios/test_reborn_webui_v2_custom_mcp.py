"""Browser coverage for custom hosted-MCP registration in WebChat v2.

The server-side lifecycle journey is covered by the hosted-MCP integration
suite. This file deliberately mocks only the browser-facing projection and
mutation routes so it can prove registration creates an available registry
entry, while the existing install/setup UI owns installation and credentials.
"""

import json
from urllib.parse import unquote, urlparse

import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


pytest_plugins = ["reborn_webui_harness"]

TENANT_CATALOG_MCP = {
    "package_ref": {"kind": "extension", "id": "tenant-catalog-mcp"},
    "display_name": "Tenant catalog MCP",
    "runtime": "mcp",
    "description": "Shared tenant catalog entry; this caller has not installed it.",
    "keywords": ["tenant", "mcp"],
    "installed": False,
    "surfaces": [{"kind": "tool"}],
}


def _registered_catalog_entry() -> dict:
    return {
        "package_ref": {"kind": "extension", "id": "custom-weather-mcp"},
        "display_name": "Custom weather MCP",
        "runtime": "mcp",
        "description": "A registered custom MCP server available to install.",
        "keywords": ["custom", "mcp"],
        "installed": False,
        "surfaces": [{"kind": "tool"}],
    }


def _installed_extension(auth_kind: str, state: str) -> dict:
    return {
        "package_ref": {"kind": "extension", "id": "custom-weather-mcp"},
        "display_name": "Custom weather MCP",
        "runtime": "mcp",
        "description": "A caller-installed custom MCP server.",
        "tools": ["weather"],
        "installation_state": state,
        "surfaces": [{"kind": "tool"}] + (
            [{"kind": "auth"}] if auth_kind != "no_auth" else []
        ),
    }


def _setup_projection(auth_kind: str) -> dict:
    secret = {
        "name": "MCP_BEARER_TOKEN",
        "prompt": "Bearer token",
        "provided": False,
        "optional": False,
        "auto_generate": False,
    }
    if auth_kind == "oauth":
        secret["setup"] = {"kind": "oauth"}
    return {
        "package_ref": {"kind": "extension", "id": "custom-weather-mcp"},
        "phase": "setup_needed",
        "blockers": ["credential_required"],
        "secrets": [secret],
        "fields": [],
        "onboarding": None,
    }


async def _open_custom_mcp_page(
    reborn_v2_server: str,
    reborn_v2_browser,
    *,
    auth_kind: str,
    setup_result: str = "success",
):
    """Open the real SPA with just its custom-MCP route family intercepted."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    installed: list[dict] = []
    registrations: list[dict] = []
    installations: list[dict] = []
    setup_submissions: list[dict] = []

    async def fulfill(route, payload: dict, status: int = 200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(payload),
            headers={"Cache-Control": "no-store"},
        )

    async def extensions_route(route) -> None:
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/extensions" and request.method == "GET":
            await fulfill(route, {"extensions": installed})
            return
        if path == "/api/webchat/v2/extensions/registry" and request.method == "GET":
            installed_ids = {
                extension["package_ref"]["id"] for extension in installed
            }
            registered_catalog = []
            if registrations:
                extension = _registered_catalog_entry()
                registered_catalog.append(
                    {
                        **extension,
                        "installed": extension["package_ref"]["id"] in installed_ids,
                    }
                )
            await fulfill(route, {"entries": [TENANT_CATALOG_MCP, *registered_catalog]})
            return
        if path == "/api/webchat/v2/extensions/register-hosted-mcp" and request.method == "POST":
            body = json.loads(request.post_data or "{}")
            registrations.append(body)
            await fulfill(
                route,
                {
                    "success": True,
                    "message": "Custom weather MCP registered",
                    "package_ref": {"kind": "extension", "id": "custom-weather-mcp"},
                },
            )
            return
        if path == "/api/webchat/v2/extensions/install" and request.method == "POST":
            body = json.loads(request.post_data or "{}")
            installations.append(body)
            state = "active" if auth_kind == "no_auth" else "setup_needed"
            installed[:] = [_installed_extension(auth_kind, state)]
            await fulfill(
                route,
                {
                    "success": True,
                    "message": "Custom weather MCP installed",
                    # Production returns the lifecycle phase here; the configure
                    # modal renders its credential prompts from it, so a bare
                    # success body leaves the secrets view unrendered.
                    "phase": state,
                    "package_ref": {"kind": "extension", "id": "custom-weather-mcp"},
                },
            )
            return
        if path.endswith("/setup") and request.method == "GET":
            package_id = unquote(path.removeprefix("/api/webchat/v2/extensions/").removesuffix("/setup"))
            if package_id == "custom-weather-mcp":
                await fulfill(route, _setup_projection(auth_kind))
                return
        if path.endswith("/setup") and request.method == "POST":
            package_id = unquote(path.removeprefix("/api/webchat/v2/extensions/").removesuffix("/setup"))
            if package_id == "custom-weather-mcp":
                body = json.loads(request.post_data or "{}")
                setup_submissions.append(body)
                if setup_result == "wrong":
                    await fulfill(route, {"success": False, "message": "Token was rejected"})
                    return
                installed[:] = [_installed_extension(auth_kind, "active")]
                await fulfill(route, {"success": True, "message": "Custom weather MCP configured"})
                return
        await route.continue_()

    await page.route("**/api/webchat/v2/extensions**", extensions_route)
    await page.goto(f"{reborn_v2_server}/extensions/registry?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.get_by_text("Registry").first).to_be_visible(timeout=15000)
    return context, page, registrations, installations, setup_submissions


async def _register(page) -> None:
    await page.get_by_role("button", name="Add MCP server").click()
    await page.get_by_label("Server name").fill("Custom weather MCP")
    await page.get_by_text("Advanced options").click()
    await page.get_by_label("Server ID").fill("custom-weather-mcp")
    await page.get_by_label("Server address").fill("https://weather.example.test/mcp")
    await page.get_by_role("button", name="Continue").click()
    await page.get_by_role("button", name="Add server").click()


async def _install_registered_mcp(page) -> None:
    card = page.get_by_test_id("extension-card").filter(has_text="Custom weather MCP")
    await expect(card.get_by_role("button", name="Install")).to_be_visible()
    await card.get_by_role("button", name="Install").click()


async def test_custom_mcp_registration_creates_uninstalled_registry_entry(
    reborn_v2_server, reborn_v2_browser
):
    context, page, registrations, installations, _ = await _open_custom_mcp_page(
        reborn_v2_server, reborn_v2_browser, auth_kind="no_auth"
    )
    try:
        # A tenant-visible catalog definition is not a caller installation.
        tenant_card = page.get_by_text("Tenant catalog MCP", exact=True)
        await expect(tenant_card).to_be_visible()
        await expect(page.get_by_role("button", name="Install")).to_be_visible()

        await _register(page)
        await expect(page.get_by_text("Server added", exact=True)).to_be_visible()
        assert registrations == [
            {
                "desired_id": "custom-weather-mcp",
                "desired_name": "Custom weather MCP",
                "endpoint": "https://weather.example.test/mcp",
                "auth_selection": {"kind": "auto"},
            }
        ]
        assert installations == []
        await expect(page.get_by_role("button", name="Continue setup")).to_have_count(0)
        await page.get_by_role("button", name="Done").click()
        await _install_registered_mcp(page)
        assert len(installations) == 1
        assert installations[0]["package_ref"] == {"kind": "extension", "id": "custom-weather-mcp"}
    finally:
        await context.close()


@pytest.mark.parametrize("setup_result", ["unfinished", "wrong", "success"])
async def test_custom_mcp_bearer_install_hands_off_to_existing_setup_states(
    reborn_v2_server, reborn_v2_browser, setup_result
):
    context, page, registrations, installations, submissions = await _open_custom_mcp_page(
        reborn_v2_server,
        reborn_v2_browser,
        auth_kind="bearer",
        setup_result=setup_result,
    )
    try:
        await _register(page)
        await expect(page.get_by_text("Server added", exact=True)).to_be_visible()
        await page.get_by_role("button", name="Done").click()
        await _install_registered_mcp(page)
        await expect(page.get_by_role("dialog")).to_contain_text("Configure Custom weather MCP")
        await expect(page.get_by_label("Bearer token")).to_be_visible()
        assert registrations[0]["auth_selection"] == {"kind": "auto"}
        assert len(installations) == 1

        if setup_result == "unfinished":
            assert submissions == []
            return

        await page.get_by_label("Bearer token").fill("wrong-or-right-token")
        await page.get_by_role("button", name="Save").click()
        if setup_result == "wrong":
            await expect(page.get_by_text("Token was rejected")).to_be_visible()
        else:
            await expect(page.get_by_role("dialog")).to_have_count(0)
            await expect(page.get_by_text("Custom weather MCP", exact=True).first).to_be_visible()
        assert submissions[0]["payload"]["secrets"] == {"MCP_BEARER_TOKEN": "wrong-or-right-token"}
    finally:
        await context.close()


async def test_custom_mcp_oauth_uses_existing_authorize_setup_control(
    reborn_v2_server, reborn_v2_browser
):
    context, page, registrations, installations, _ = await _open_custom_mcp_page(
        reborn_v2_server, reborn_v2_browser, auth_kind="oauth"
    )
    try:
        await _register(page)
        await expect(page.get_by_text("Server added", exact=True)).to_be_visible()
        await page.get_by_role("button", name="Done").click()
        await _install_registered_mcp(page)
        await expect(page.get_by_role("button", name="Authorize")).to_be_visible()
        assert registrations[0]["auth_selection"] == {"kind": "auto"}
        assert len(installations) == 1
    finally:
        await context.close()
