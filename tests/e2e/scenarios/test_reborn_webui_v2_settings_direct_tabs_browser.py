"""Stubbed-browser WebUI v2 Settings direct-tabs matrix.

These tests drive the committed Settings routes in Chromium and stub only the
same-origin v2 browser API contracts. They cover direct route role gating and
the Tools configuration panel for REBCLI-096.
"""

import json
import re
import time

import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


async def _wait_for(condition, page, *, timeout: float = 5) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        await page.wait_for_timeout(100)
    raise AssertionError("condition did not become true before timeout")


class SettingsDirectTabsStub:
    def __init__(self, *, operator: bool):
        self.operator = operator
        self.auto_approve_enabled = False
        self.session_requests: list[str] = []
        self.settings_get_requests: list[str] = []
        self.auto_approve_posts: list[dict] = []
        self.tool_permission_posts: list[tuple[str, dict]] = []

    async def install(self, page) -> None:
        await page.route(re.compile(r".*/api/webchat/v2/session$"), self._session)
        await page.route(
            re.compile(r".*/api/webchat/v2/threads(?:\?.*)?$"),
            self._threads,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/settings/tools(?:/[^/?]+)?$"),
            self._settings_tools,
        )

    async def _fulfill(self, route, body, status: int = 200) -> None:
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
        )

    async def _session(self, route) -> None:
        self.session_requests.append(route.request.headers.get("authorization", ""))
        await self._fulfill(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": "settings-direct-tabs-browser-user",
                "capabilities": {"operator_webui_config": self.operator},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def _threads(self, route) -> None:
        await self._fulfill(route, {"threads": [], "next_cursor": None})

    async def _settings_tools(self, route) -> None:
        url = route.request.url
        if route.request.method == "POST" and url.endswith("/settings/tools"):
            body = route.request.post_data_json
            self.auto_approve_posts.append(body)
            self.auto_approve_enabled = bool(body.get("enabled"))
            await self._fulfill(
                route,
                {
                    "entry": {
                        "key": "agent.auto_approve_tools",
                        "value": self.auto_approve_enabled,
                        "mutable": True,
                        "source": "global",
                    }
                },
            )
            return

        if route.request.method == "POST":
            tool_name = url.rsplit("/", 1)[-1]
            body = route.request.post_data_json
            self.tool_permission_posts.append((tool_name, body))
            await self._fulfill(
                route,
                {
                    "entry": {
                        "key": f"tool.{tool_name}",
                        "value": {
                            "name": tool_name,
                            "description": "Run shell commands",
                            "state": body.get("state"),
                            "default_state": "ask_each_time",
                            "locked": False,
                            "effective_source": "override",
                        },
                        "mutable": True,
                        "source": "override",
                    }
                },
            )
            return

        self.settings_get_requests.append(route.request.headers.get("authorization", ""))
        await self._fulfill(
            route,
            {
                "entries": [
                    {
                        "key": "agent.auto_approve_tools",
                        "value": self.auto_approve_enabled,
                        "mutable": True,
                        "source": "global",
                    },
                    {
                        "key": "tool.shell",
                        "value": {
                            "name": "shell",
                            "description": "Run shell commands",
                            "state": "ask_each_time",
                            "default_state": "ask_each_time",
                            "locked": False,
                            "effective_source": "default",
                        },
                        "mutable": True,
                        "source": "default",
                    },
                    {
                        "key": "tool.browser",
                        "value": {
                            "name": "browser",
                            "description": "Browser automation",
                            "state": "always_allow",
                            "default_state": "ask_each_time",
                            "locked": False,
                            "effective_source": "global",
                        },
                        "mutable": True,
                        "source": "global",
                    },
                ],
                "diagnostics": [],
                "precedence": [],
            },
        )


async def test_settings_direct_operator_tabs_redirect_for_members(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = SettingsDirectTabsStub(operator=False)
    try:
        await stub.install(page)
        await page.goto(
            f"{reborn_v2_server}/v2/settings/inference?token={REBORN_V2_AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )

        await expect(page).to_have_url(re.compile(r"/v2/settings/language/?(?:\?.*)?$"))
        await expect(page.get_by_role("heading", name="Language")).to_be_visible(
            timeout=15000
        )
        await expect(page.get_by_role("link", name="Inference")).to_have_count(0)
        assert stub.session_requests
        assert stub.settings_get_requests
    finally:
        await context.close()


async def test_settings_direct_tools_tab_search_and_permission_saves_use_v2_api(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = SettingsDirectTabsStub(operator=True)
    try:
        await stub.install(page)
        await page.goto(
            f"{reborn_v2_server}/v2/settings/tools?token={REBORN_V2_AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )

        await expect(page.get_by_text("Tool permissions", exact=True)).to_be_visible(
            timeout=15000
        )
        await expect(page.get_by_text("Run shell commands", exact=True)).to_be_visible()
        await expect(page.get_by_text("Browser automation", exact=True)).to_be_visible()

        await page.locator(SEL_V2["settings_toolbar_search"]).fill("shell")
        await expect(page.get_by_text("Run shell commands", exact=True)).to_be_visible()
        await expect(page.get_by_text("Browser automation", exact=True)).to_have_count(0)

        await page.get_by_role("switch", name="Always allow eligible tools").click()
        await page.get_by_label("Permission for shell").select_option("always_allow")

        await _wait_for(
            lambda: stub.auto_approve_posts == [{"enabled": True}],
            page,
        )
        await _wait_for(
            lambda: stub.tool_permission_posts == [("shell", {"state": "always_allow"})],
            page,
        )
        assert all(
            request.startswith("Bearer ") for request in stub.settings_get_requests
        )
    finally:
        await context.close()
