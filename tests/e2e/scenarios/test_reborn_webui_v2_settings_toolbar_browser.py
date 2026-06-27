"""Stubbed-browser WebUI v2 Settings toolbar matrix.

These tests drive the committed Settings page in Chromium and stub only the
same-origin v2 browser API contracts. They cover the user-visible toolbar
search, JSON export, and JSON import workflows for REBCLI-090.
"""

import json
import re
from pathlib import Path

import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


class SettingsToolbarStub:
    def __init__(self):
        self.auto_approve_enabled = False
        self.session_requests: list[str] = []
        self.settings_get_requests: list[str] = []
        self.settings_post_bodies: list[dict] = []

    async def install(self, page) -> None:
        await page.route(re.compile(r".*/api/webchat/v2/session$"), self._session)
        await page.route(
            re.compile(r".*/api/webchat/v2/threads(?:\?.*)?$"),
            self._threads,
        )
        await page.route(
            re.compile(r".*/api/webchat/v2/settings/tools$"),
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
                "user_id": "settings-toolbar-browser-user",
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
        if route.request.method == "POST":
            body = route.request.post_data_json
            self.settings_post_bodies.append(body)
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
                    }
                ],
                "diagnostics": [],
                "precedence": [],
            },
        )


async def _open_settings_toolbar_page(page, reborn_v2_server, stub: SettingsToolbarStub) -> None:
    await stub.install(page)
    await page.goto(
        f"{reborn_v2_server}/v2/settings/networking?token={REBORN_V2_AUTH_TOKEN}",
        wait_until="domcontentloaded",
    )
    await expect(page.locator(SEL_V2["settings_toolbar_search"])).to_be_visible(
        timeout=15000
    )


async def test_settings_toolbar_search_filters_and_clears_networking_fields(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = SettingsToolbarStub()
    try:
        await _open_settings_toolbar_page(page, reborn_v2_server, stub)

        await expect(page.get_by_text("Gateway bind address", exact=True)).to_be_visible()
        await expect(page.get_by_text("Static tunnel endpoint", exact=True)).to_be_visible()

        await page.locator(SEL_V2["settings_toolbar_search"]).fill("public")
        await expect(page.get_by_text("Static tunnel endpoint", exact=True)).to_be_visible()
        await expect(page.get_by_text("Gateway bind address", exact=True)).to_have_count(0)

        await page.locator(SEL_V2["settings_toolbar_clear_search"]).click()
        await expect(page.locator(SEL_V2["settings_toolbar_search"])).to_have_value("")
        await expect(page.get_by_text("Gateway bind address", exact=True)).to_be_visible()

        assert stub.session_requests
        assert stub.settings_get_requests
    finally:
        await context.close()


async def test_settings_toolbar_exports_and_imports_json_through_v2_settings_api(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(
        viewport={"width": 1280, "height": 720},
        accept_downloads=True,
    )
    page = await context.new_page()
    stub = SettingsToolbarStub()
    try:
        await _open_settings_toolbar_page(page, reborn_v2_server, stub)

        async with page.expect_download() as download_info:
            await page.locator(SEL_V2["settings_toolbar_export"]).click()
        download = await download_info.value
        assert download.suggested_filename == "ironclaw-settings.json"
        export_path = await download.path()
        exported = json.loads(Path(export_path).read_text(encoding="utf-8"))
        assert exported == {
            "settings": {"agent.auto_approve_tools": False},
            "diagnostics": [],
            "precedence": [],
        }
        await expect(page.locator(SEL_V2["settings_toolbar_status"])).to_contain_text(
            "Settings exported"
        )

        payload = {"settings": {"agent.auto_approve_tools": True}}
        await page.locator(SEL_V2["settings_toolbar_import_input"]).set_input_files(
            {
                "name": "ironclaw-settings.json",
                "mimeType": "application/json",
                "buffer": json.dumps(payload).encode("utf-8"),
            }
        )

        await expect(page.locator(SEL_V2["settings_toolbar_status"])).to_contain_text(
            "Settings imported"
        )
        assert stub.settings_post_bodies == [{"enabled": True}]
        assert stub.auto_approve_enabled is True
    finally:
        await context.close()
