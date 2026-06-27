"""Stubbed-browser WebUI v2 Trace Commons sidebar matrix.

These tests drive the committed WebUI v2 shell in Chromium and stub only the
local Trace Commons credit endpoint. That covers the user-visible sidebar
credit card without live ledger state or network credentials.
"""

import json
import re

import pytest
from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import open_reborn_v2_page


pytest_plugins = ["reborn_webui_harness"]
pytestmark = pytest.mark.asyncio


class TraceCreditsStub:
    def __init__(self, payload: dict):
        self.payload = payload
        self.requests: list[str] = []

    async def install(self, page) -> None:
        await page.route(
            re.compile(r".*/api/webchat/v2/traces/credit$"),
            self._credit_view,
        )

    async def _credit_view(self, route) -> None:
        self.requests.append(route.request.headers.get("authorization", ""))
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(self.payload),
        )


async def test_trace_credits_sidebar_shows_enrolled_summary_and_links_to_settings(
    reborn_v2_server,
    reborn_v2_browser,
):
    stub = TraceCreditsStub(
        {
            "enrolled": True,
            "final_credit": "2.345",
            "submissions_accepted": 3,
            "submissions_submitted": 5,
            "manual_review_hold_count": 2,
        }
    )
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await stub.install(page)
        await open_reborn_v2_page(page, reborn_v2_server)

        await expect(page.locator(SEL_V2["trace_credits_card"])).to_be_visible()
        await expect(page.locator(SEL_V2["trace_credits_final"])).to_have_text("+2.35")
        await expect(page.locator(SEL_V2["trace_credits_counts"])).to_contain_text("3")
        await expect(page.locator(SEL_V2["trace_credits_counts"])).to_contain_text("5")
        await expect(page.locator(SEL_V2["trace_credits_held"])).to_contain_text("2")
        assert stub.requests
        assert stub.requests[-1].startswith("Bearer ")

        await page.locator(SEL_V2["trace_credits_card"]).click()
        await expect(page).to_have_url(re.compile(r"/v2/settings/traces/?(?:\?.*)?$"))
    finally:
        await context.close()


async def test_trace_credits_sidebar_hides_not_enrolled_state(
    reborn_v2_server,
    reborn_v2_browser,
):
    stub = TraceCreditsStub({"enrolled": False})
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await stub.install(page)
        await open_reborn_v2_page(page, reborn_v2_server)
        for _ in range(50):
            if stub.requests:
                break
            await page.wait_for_timeout(100)
        assert stub.requests

        await expect(page.locator(SEL_V2["trace_credits_card"])).to_have_count(0)
    finally:
        await context.close()
