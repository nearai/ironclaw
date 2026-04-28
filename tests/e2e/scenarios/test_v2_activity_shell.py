"""Playwright coverage for the v2 activity shell."""

import pytest
from playwright.async_api import expect

from helpers import AUTH_TOKEN, SEL, api_post

from .test_v2_engine_approval_flow import _wait_for_approval, v2_approval_server


@pytest.fixture
async def v2_approval_page(v2_approval_server, browser):
    """Fresh Playwright page bound to the v2 approval server fixture."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{v2_approval_server}/?token={AUTH_TOKEN}")
    await page.wait_for_selector(SEL["auth_screen"], state="hidden", timeout=15000)
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=10000,
    )
    yield page
    await context.close()


async def test_v2_hides_routines_tab(v2_approval_page):
    """The legacy Routines tab should not be shown when ENGINE_V2 is enabled."""
    routines_tab = v2_approval_page.locator(SEL["tab_button"].format(tab="routines"))
    missions_tab = v2_approval_page.locator(SEL["tab_button"].format(tab="missions"))
    await expect(routines_tab).to_be_hidden()
    await expect(missions_tab).to_be_visible()


async def test_v2_missions_tab_replaces_removed_activity_strip(
    v2_approval_server,
    v2_approval_page,
):
    """Background v2 work should be accessed via the Missions tab, not a pill strip."""
    thread_r = await api_post(v2_approval_server, "/api/chat/thread/new", timeout=15)
    thread_r.raise_for_status()
    thread_id = thread_r.json()["id"]

    send_r = await api_post(
        v2_approval_server,
        "/api/chat/send",
        json={"content": "make approval post active-shell", "thread_id": thread_id},
        timeout=30,
    )
    send_r.raise_for_status()
    await _wait_for_approval(v2_approval_server, thread_id)

    await expect(v2_approval_page.locator(SEL["active_work_strip"])).to_have_count(0)

    missions_tab = v2_approval_page.locator(SEL["tab_button"].format(tab="missions"))
    missions_panel = v2_approval_page.locator(SEL["tab_panel"].format(tab="missions"))
    summary = v2_approval_page.locator(SEL["missions_summary"])

    async with v2_approval_page.expect_response(
        lambda response: response.request.method == "GET"
        and response.url.endswith("/api/engine/missions/summary")
    ) as missions_summary_response:
        await missions_tab.click()

    response = await missions_summary_response.value
    assert response.ok, f"missions summary request failed: {response.status} {response.url}"

    await missions_panel.wait_for(state="visible", timeout=5000)
    await v2_approval_page.wait_for_function(
        "selector => {"
        "  const el = document.querySelector(selector);"
        "  return !!el && el.textContent.trim().length > 0;"
        "}",
        arg=SEL["missions_summary"],
        timeout=5000,
    )
    await expect(summary).to_be_visible()


