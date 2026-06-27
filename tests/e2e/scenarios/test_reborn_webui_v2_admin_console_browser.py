"""Stubbed-browser WebUI v2 Admin console matrix.

This drives the committed Admin routes in Chromium and stubs only the local
v2 session/thread contracts. The Admin data client is intentionally a TODO
stub, so the browser assertion is that the console renders its empty states
without falling back to legacy `/api/admin/*` routes.
"""

import json
import re

import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


class AdminConsoleStub:
    def __init__(self):
        self.session_requests: list[str] = []
        self.legacy_admin_requests: list[str] = []

    async def install(self, page) -> None:
        await page.route(re.compile(r".*/api/webchat/v2/session$"), self._session)
        await page.route(
            re.compile(r".*/api/webchat/v2/threads(?:\?.*)?$"),
            self._threads,
        )
        await page.route(re.compile(r".*/api/admin(?:/.*)?$"), self._legacy_admin)

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
                "user_id": "admin-console-browser-user",
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

    async def _legacy_admin(self, route) -> None:
        self.legacy_admin_requests.append(route.request.url)
        await self._fulfill(route, {"error": "legacy admin route must not be called"}, status=599)


async def test_admin_console_dashboard_users_and_usage_render_without_legacy_admin_api(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    stub = AdminConsoleStub()
    try:
        await stub.install(page)

        await page.goto(
            f"{reborn_v2_server}/v2/admin/dashboard?token={REBORN_V2_AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )
        await expect(page).to_have_url(re.compile(r"/v2/admin/dashboard/?(?:\?.*)?$"))
        await expect(page.get_by_text("System overview", exact=True)).to_be_visible(
            timeout=15000
        )
        await expect(page.locator("body")).to_contain_text("Total users")
        await expect(page.locator("body")).to_contain_text("No users yet.")

        await page.goto(
            f"{reborn_v2_server}/v2/admin/users?token={REBORN_V2_AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )
        await expect(page).to_have_url(re.compile(r"/v2/admin/users/?(?:\?.*)?$"))
        await expect(page.get_by_text("Users (0 / 0)", exact=True)).to_be_visible(
            timeout=15000
        )
        await expect(page.locator("body")).to_contain_text(
            "No users match the current filters."
        )
        await page.get_by_role("button", name="New user").click()
        await expect(page.get_by_role("heading", name="Create user")).to_be_visible()
        await expect(page.get_by_placeholder("Jane Doe")).to_be_visible()

        await page.goto(
            f"{reborn_v2_server}/v2/admin/usage?token={REBORN_V2_AUTH_TOKEN}",
            wait_until="domcontentloaded",
        )
        await expect(page).to_have_url(re.compile(r"/v2/admin/usage/?(?:\?.*)?$"))
        await expect(page.get_by_text("Usage overview", exact=True)).to_be_visible(
            timeout=15000
        )
        await expect(page.locator("body")).to_contain_text(
            "No usage data for this period."
        )
        await expect(page.get_by_role("button", name="24h")).to_be_visible()
        await page.get_by_role("button", name="7d").click()
        await expect(page.locator("body")).to_contain_text(
            "No usage data for this period."
        )

        assert stub.session_requests
        assert all(request.startswith("Bearer ") for request in stub.session_requests)
        assert stub.legacy_admin_requests == []
    finally:
        await context.close()
