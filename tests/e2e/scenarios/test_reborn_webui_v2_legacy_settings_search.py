"""Legacy settings search coverage ported to Reborn WebChat v2."""

import json
from urllib.parse import urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


MOCK_TOOL_ENTRIES = [
    {
        "key": "agent.auto_approve_tools",
        "value": False,
        "mutable": True,
        "source": "default",
    },
    {
        "key": "tool.echo",
        "value": {
            "name": "echo",
            "description": "Echo text back for deterministic tests.",
            "state": "ask_each_time",
            "default_state": "ask_each_time",
            "effective_source": "default",
        },
        "mutable": True,
        "source": "default",
    },
    {
        "key": "tool.search_web",
        "value": {
            "name": "search_web",
            "description": "Search the web for current answers.",
            "state": "always_allow",
            "default_state": "ask_each_time",
            "effective_source": "override",
        },
        "mutable": True,
        "source": "override",
    },
]

MOCK_SKILLS = [
    {
        "name": "markdown-helper",
        "description": "Formats markdown for deterministic tests.",
        "version": "1.0.0",
        "trust": "Installed",
        "source_kind": "installed",
        "keywords": ["markdown"],
        "usage_hint": "Use for markdown formatting.",
        "can_edit": True,
        "can_delete": True,
        "auto_activate": True,
    },
    {
        "name": "workspace-helper",
        "description": "Reads workspace context.",
        "version": "1.0.0",
        "trust": "Trusted",
        "source_kind": "workspace",
        "keywords": ["workspace"],
        "can_edit": False,
        "can_delete": False,
    },
]

MOCK_CHANNEL_EXTENSION = {
    "name": "telegram-channel",
    "package_ref": {"kind": "extension", "id": "telegram-channel"},
    "display_name": "Telegram Channel",
    "kind": "wasm_channel",
    "description": "Configured messaging channel.",
    "active": True,
    "authenticated": True,
    "onboarding_state": "ready",
}

MOCK_MCP_EXTENSION = {
    "name": "beta-mcp",
    "package_ref": {"kind": "extension", "id": "beta-mcp"},
    "display_name": "Beta MCP",
    "kind": "mcp_server",
    "description": "Installed MCP server.",
    "active": False,
    "authenticated": False,
}


async def _open_mocked_settings_page(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    tab: str,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    browser_messages: list[str] = []
    page.on(
        "console",
        lambda message: browser_messages.append(f"{message.type}: {message.text}"),
    )
    page.on("pageerror", lambda error: browser_messages.append(f"pageerror: {error}"))

    async def fulfill_json(route, payload, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(payload),
            headers={"Cache-Control": "no-store"},
        )

    async def handle_settings_tools(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/settings/tools" and request.method == "GET":
            await fulfill_json(route, {"entries": MOCK_TOOL_ENTRIES})
            return

        await route.continue_()

    async def handle_skills(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/skills" and request.method == "GET":
            await fulfill_json(
                route,
                {
                    "skills": MOCK_SKILLS,
                    "count": len(MOCK_SKILLS),
                    "auto_activate_learned": True,
                },
            )
            return

        await route.continue_()

    async def handle_extensions(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/extensions" and request.method == "GET":
            await fulfill_json(
                route,
                {"extensions": [MOCK_CHANNEL_EXTENSION, MOCK_MCP_EXTENSION]},
            )
            return

        if path == "/api/webchat/v2/extensions/registry" and request.method == "GET":
            await fulfill_json(route, {"entries": []})
            return

        await route.continue_()

    await page.route("**/api/webchat/v2/settings/tools**", handle_settings_tools)
    await page.route("**/api/webchat/v2/skills**", handle_skills)
    await page.route("**/api/webchat/v2/extensions**", handle_extensions)

    await page.goto(f"{reborn_v2_server}/v2/settings/{tab}?token={REBORN_V2_AUTH_TOKEN}")
    search = page.get_by_placeholder("Search settings...")
    try:
        await expect(search).to_be_visible(timeout=15000)
    except AssertionError as error:
        body_text = await page.locator("body").inner_text(timeout=1000)
        raise AssertionError(
            f"Settings search toolbar did not render on {page.url}.\n"
            f"Browser messages: {browser_messages}\n"
            f"Body text:\n{body_text}"
        ) from error

    return {"context": context, "page": page, "search": search}


async def test_reborn_legacy_settings_tools_search_and_clear(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_settings_page(
        reborn_v2_server,
        reborn_v2_browser,
        tab="tools",
    )
    try:
        page = harness["page"]
        search = harness["search"]

        await expect(page.get_by_text("echo", exact=True)).to_be_visible(timeout=5000)
        await expect(page.get_by_text("search_web", exact=True)).to_be_visible(timeout=5000)

        await search.fill("echo")
        await expect(page.get_by_text("echo", exact=True)).to_be_visible()
        await expect(page.get_by_text("search_web", exact=True)).to_have_count(0)
        await expect(page.get_by_text("1 / 2")).to_be_visible()

        await page.get_by_role("button", name="Clear search").click()
        await expect(search).to_have_value("")
        await expect(page.get_by_text("search_web", exact=True)).to_be_visible()

        await search.fill("missing-tool")
        await expect(page.get_by_text("No tools match the filter.")).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_settings_skills_search_empty_state(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_settings_page(
        reborn_v2_server,
        reborn_v2_browser,
        tab="skills",
    )
    try:
        page = harness["page"]
        search = harness["search"]

        await expect(page.get_by_text("markdown-helper", exact=True)).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("workspace-helper", exact=True)).to_be_visible(
            timeout=5000
        )

        await search.fill("workspace")
        await expect(page.get_by_text("workspace-helper", exact=True)).to_be_visible()
        await expect(page.get_by_text("markdown-helper", exact=True)).to_have_count(0)

        await search.fill("no-such-skill")
        await expect(page.get_by_text('No settings match "no-such-skill"')).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_settings_channels_search(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_settings_page(
        reborn_v2_server,
        reborn_v2_browser,
        tab="channels",
    )
    try:
        page = harness["page"]
        search = harness["search"]

        await expect(page.get_by_text("Telegram Channel", exact=True)).to_be_visible(
            timeout=5000
        )
        await expect(page.get_by_text("Beta MCP", exact=True)).to_be_visible(timeout=5000)

        await search.fill("telegram")
        await expect(page.get_by_text("Telegram Channel", exact=True)).to_be_visible()
        await expect(page.get_by_text("Beta MCP", exact=True)).to_have_count(0)

        await search.fill("nothing-matches-this")
        await expect(
            page.get_by_text('No settings match "nothing-matches-this"')
        ).to_be_visible()
    finally:
        await harness["context"].close()
