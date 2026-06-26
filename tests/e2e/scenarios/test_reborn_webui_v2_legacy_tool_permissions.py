"""Legacy tool-permission coverage ported to Reborn WebChat v2."""

import json
from urllib.parse import unquote, urlparse

import httpx
import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


def _tool_entry(
    name: str,
    *,
    state: str,
    default_state: str = "ask_each_time",
    source: str = "override",
    mutable: bool = True,
    description: str | None = None,
) -> dict:
    return {
        "key": f"tool.{name}",
        "value": {
            "name": name,
            "description": description or f"{name} deterministic test tool.",
            "state": state,
            "default_state": default_state,
            "locked": not mutable,
            "effective_source": source,
        },
        "mutable": mutable,
        "source": source,
    }


async def _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    auto_approve = {"enabled": False}
    tool_states = {
        "echo": {
            "state": "always_allow",
            "default_state": "ask_each_time",
            "source": "override",
            "mutable": True,
            "description": "Echo text back.",
        },
        "tool.financial": {
            "state": "ask_each_time",
            "default_state": "ask_each_time",
            "source": "locked",
            "mutable": False,
            "description": "Hard-floor approval tool.",
        },
    }
    auto_approve_requests: list[dict] = []
    permission_requests: list[dict] = []

    def entries():
        return [
            {
                "key": "agent.auto_approve_tools",
                "value": auto_approve["enabled"],
                "mutable": True,
                "source": "override" if auto_approve["enabled"] else "default",
            },
            *[
                _tool_entry(
                    name,
                    state=data["state"],
                    default_state=data["default_state"],
                    source=data["source"],
                    mutable=data["mutable"],
                    description=data["description"],
                )
                for name, data in tool_states.items()
            ],
        ]

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
            await fulfill_json(route, {"entries": entries()})
            return

        if path == "/api/webchat/v2/settings/tools" and request.method == "POST":
            body = json.loads(request.post_data or "{}")
            auto_approve_requests.append(body)
            auto_approve["enabled"] = bool(body.get("enabled"))
            await fulfill_json(
                route,
                {
                    "entry": {
                        "key": "agent.auto_approve_tools",
                        "value": auto_approve["enabled"],
                        "mutable": True,
                        "source": "override",
                    }
                },
            )
            return

        if (
            path.startswith("/api/webchat/v2/settings/tools/")
            and request.method == "POST"
        ):
            name = unquote(path.removeprefix("/api/webchat/v2/settings/tools/"))
            body = json.loads(request.post_data or "{}")
            permission_requests.append({"name": name, "body": body})
            tool = tool_states[name]
            if not tool["mutable"]:
                await fulfill_json(
                    route,
                    {"kind": "bad_request", "message": "locked tool"},
                    status=400,
                )
                return

            requested = body.get("state") or "default"
            if requested == "default":
                tool["state"] = tool["default_state"]
                tool["source"] = "default"
            else:
                tool["state"] = requested
                tool["source"] = "override"

            await fulfill_json(
                route,
                {
                    "entry": _tool_entry(
                        name,
                        state=tool["state"],
                        default_state=tool["default_state"],
                        source=tool["source"],
                        mutable=tool["mutable"],
                        description=tool["description"],
                    )
                },
            )
            return

        await route.continue_()

    await page.route("**/api/webchat/v2/settings/tools**", handle_settings_tools)
    await page.goto(f"{reborn_v2_server}/v2/settings/tools?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.get_by_placeholder("Search settings...")).to_be_visible(timeout=15000)
    await expect(page.get_by_text("Tool permissions")).to_be_visible(timeout=5000)

    return {
        "context": context,
        "page": page,
        "auto_approve_requests": auto_approve_requests,
        "permission_requests": permission_requests,
    }


def _tool_row(page, name: str):
    return page.locator(f'[data-testid="settings-tool-row"][data-tool-name="{name}"]')


async def test_reborn_legacy_tool_permissions_tab_visible(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        await expect(page.get_by_text("Always allow eligible tools")).to_be_visible()
        await expect(_tool_row(page, "echo")).to_be_visible(timeout=5000)
        await expect(_tool_row(page, "tool.financial")).to_be_visible(timeout=5000)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_tool_permission_select_persists_after_reload(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        select = page.get_by_label("Permission for echo")
        await expect(select).to_have_value("always_allow", timeout=5000)

        await select.select_option("ask_each_time")
        await expect(select).to_have_value("ask_each_time")
        await expect(_tool_row(page, "echo").get_by_text("saved")).to_be_visible(timeout=5000)
        assert harness["permission_requests"][-1] == {
            "name": "echo",
            "body": {"state": "ask_each_time"},
        }

        await page.reload()
        await expect(page.get_by_placeholder("Search settings...")).to_be_visible(timeout=15000)
        await expect(page.get_by_label("Permission for echo")).to_have_value(
            "ask_each_time",
            timeout=5000,
        )

        await page.get_by_label("Permission for echo").select_option("default")
        await expect(page.get_by_label("Permission for echo")).to_have_value("default")
        assert harness["permission_requests"][-1] == {
            "name": "echo",
            "body": {"state": "default"},
        }
    finally:
        await harness["context"].close()


async def test_reborn_legacy_locked_tool_shows_badge_without_select(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        locked = _tool_row(page, "tool.financial")
        await expect(locked).to_be_visible(timeout=5000)
        await expect(locked.locator('[data-testid="settings-tool-lock"]')).to_be_visible()
        await expect(locked.get_by_label("Permission for tool.financial")).to_have_count(0)
        await expect(locked.get_by_text("Ask each time")).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_auto_approve_switch_persists(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_tools_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        switch = page.get_by_role("switch", name="Always allow eligible tools")
        await expect(switch).to_have_attribute("aria-checked", "false")
        await switch.click()
        await expect(switch).to_have_attribute("aria-checked", "true")
        assert harness["auto_approve_requests"] == [{"enabled": True}]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_tool_permission_real_api_persists_and_rejects_locked(
    reborn_v2_server,
):
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        response = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/settings/tools",
            timeout=15,
        )
        response.raise_for_status()
        entries = response.json().get("entries", [])

        tools = [entry for entry in entries if entry.get("key", "").startswith("tool.")]
        mutable = next((entry for entry in tools if entry.get("mutable") is not False), None)
        locked = next((entry for entry in tools if entry.get("mutable") is False), None)

        if mutable is None:
            pytest.skip("Reborn test catalog has no mutable operator tool")

        capability_id = mutable["key"].removeprefix("tool.")
        update = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/settings/tools/{capability_id}",
            json={"state": "disabled"},
            timeout=15,
        )
        update.raise_for_status()
        assert update.json()["entry"]["value"]["state"] == "disabled"
        assert update.json()["entry"]["mutable"] is True

        reset = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/settings/tools/{capability_id}",
            json={"state": "default"},
            timeout=15,
        )
        reset.raise_for_status()
        assert reset.json()["entry"]["key"] == mutable["key"]

        if locked is not None:
            locked_id = locked["key"].removeprefix("tool.")
            rejected = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/settings/tools/{locked_id}",
                json={"state": "always_allow"},
                timeout=15,
            )
            assert rejected.status_code >= 400
