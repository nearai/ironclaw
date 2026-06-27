"""Stubbed-browser WebUI v2 Settings restart banner matrix.

This drives the committed Settings page in Chromium and stubs only the local
v2 browser API contracts. It verifies the implemented v2 behavior: restart
required state is surfaced after a restart-required settings import, while the
restart action remains disabled because v2 does not expose a restart endpoint.
"""

import json
import re

import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


class SettingsRestartStub:
    def __init__(self):
        self.session_requests: list[str] = []
        self.settings_requests: list[str] = []
        self.legacy_restart_requests: list[str] = []

    async def install(self, page) -> None:
        await page.route(re.compile(r".*/api/webchat/v2/session$"), self._session)
        await page.route(
            re.compile(r".*/api/webchat/v2/threads(?:\?.*)?$"),
            self._threads,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/settings/tools(?:/.*)?$"),
            self._settings_tools,
        )
        await page.route(
            re.compile(r".*/(?:api/chat/events|restart)(?:\?.*)?$"),
            self._legacy_restart,
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
                "user_id": "settings-restart-browser-user",
                "capabilities": {"operator_webui_config": True},
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
        self.settings_requests.append(route.request.headers.get("authorization", ""))
        await self._fulfill(route, {"entries": [], "diagnostics": [], "precedence": []})

    async def _legacy_restart(self, route) -> None:
        self.legacy_restart_requests.append(route.request.url)
        await self._fulfill(route, {"error": "legacy restart must not be called"}, status=500)


async def test_settings_restart_banner_import_path_is_visible_but_disabled(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = SettingsRestartStub()
    try:
        await stub.install(page)
        await page.goto(
            f"{reborn_v2_server}/v2/settings/networking?token={REBORN_V2_AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )
        await expect(page.get_by_placeholder("Search settings")).to_be_visible(timeout=15000)
        await expect(page.locator(SEL_V2["settings_restart_banner"])).to_have_count(0)

        payload = {
            "settings": {
                "tunnel.public_url": "https://reborn.example.test",
            }
        }
        await page.locator("input[type='file'][accept*='json']").set_input_files(
            {
                "name": "ironclaw-settings.json",
                "mimeType": "application/json",
                "buffer": json.dumps(payload).encode("utf-8"),
            }
        )

        await expect(page.locator(SEL_V2["settings_restart_banner"])).to_be_visible()
        await expect(page.locator(SEL_V2["settings_restart_banner"])).to_contain_text(
            "Some changes require a restart"
        )
        await expect(page.locator(SEL_V2["settings_restart_unavailable"])).to_contain_text(
            "Restart from the web UI isn't available yet"
        )
        action = page.locator(SEL_V2["settings_restart_action"])
        await expect(action).to_be_disabled()
        await expect(action).to_have_attribute(
            "title",
            re.compile(r"Restart from the web UI isn't available yet"),
        )
        assert stub.session_requests
        assert stub.settings_requests
        assert stub.legacy_restart_requests == []
    finally:
        await context.close()
