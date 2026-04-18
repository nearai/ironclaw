"""Playwright coverage for the v2 activity shell."""

import pytest
from playwright.async_api import expect

from helpers import AUTH_TOKEN, api_post

from .test_v2_engine_approval_flow import _wait_for_approval, v2_approval_server


@pytest.fixture
async def v2_approval_page(v2_approval_server, browser):
    """Fresh Playwright page bound to the v2 approval server fixture."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{v2_approval_server}/?token={AUTH_TOKEN}")
    await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=10000,
    )
    yield page
    await context.close()


@pytest.mark.asyncio
async def test_v2_hides_routines_tab(v2_approval_page):
    """The legacy Routines tab should not be shown when ENGINE_V2 is enabled."""
    routines_tab = v2_approval_page.locator('.tab-bar button[data-tab="routines"]')
    await expect(routines_tab).to_be_hidden()
    await expect(v2_approval_page.locator('.tab-bar button[data-tab="missions"]')).to_be_visible()


@pytest.mark.asyncio
async def test_active_work_strip_survives_tab_switch(
    v2_approval_server,
    v2_approval_page,
):
    """Background v2 work stays visible after leaving the Chat tab."""
    r = await api_post(v2_approval_server, "/api/chat/thread/new")
    r.raise_for_status()
    thread_id = r.json()["id"]

    r = await api_post(
        v2_approval_server,
        "/api/chat/send",
        json={"content": "make approval post active-shell", "thread_id": thread_id},
        timeout=15,
    )
    r.raise_for_status()
    await _wait_for_approval(v2_approval_server, thread_id)

    strip = v2_approval_page.locator("#active-work-strip")
    await expect(strip).to_be_visible()

    await v2_approval_page.locator('.tab-bar button[data-tab="settings"]').click()
    await v2_approval_page.locator("#tab-settings.active").wait_for(state="visible", timeout=5000)
    await expect(strip).to_be_visible()
