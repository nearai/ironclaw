"""Reborn WebUI v2 notification center E2E coverage."""

import json
import re
from urllib.parse import parse_qs, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-e2e-notification"


def _notification_threads_payload():
    return {
        "threads": [
            {
                "id": THREAD_ID,
                "thread_id": THREAD_ID,
                "title": "E2E scheduled report",
                "state": "needs_attention",
                "updated_at": "2026-06-30T08:10:01Z",
            }
        ],
        "next_cursor": None,
    }


async def _route_notification_threads(page):
    async def handler(route):
        parsed = urlparse(route.request.url)
        query = parse_qs(parsed.query)
        if parsed.path != "/api/webchat/v2/threads":
            await route.continue_()
            return
        payload = _notification_threads_payload()
        if query.get("needs_approval") != ["true"]:
            payload = {
                **payload,
                "threads": [
                    {**thread, "state": "idle"}
                    for thread in payload["threads"]
                ],
            }
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(payload),
        )

    await page.route("**/api/webchat/v2/threads*", handler)


async def _route_thread_delete_failure(page):
    async def handler(route):
        if route.request.method != "DELETE":
            await route.continue_()
            return
        await route.fulfill(
            status=503,
            content_type="application/json",
            body=json.dumps(
                {
                    "kind": "service_unavailable",
                    "message": "Thread deletion is temporarily unavailable.",
                }
            ),
        )

    await page.route(f"**/api/webchat/v2/threads/{THREAD_ID}", handler)


async def _open_v2(page, base_url, path="/v2/"):
    separator = "&" if "?" in path else "?"
    await page.goto(f"{base_url}{path}{separator}token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.locator(SEL_V2["notification_bell"])).to_be_visible(timeout=15000)


async def test_reborn_v2_notification_popover_opens_automation_thread(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await _route_notification_threads(page)
        await _open_v2(page, reborn_v2_server)

        await page.locator(SEL_V2["notification_bell"]).click()
        panel = page.locator(SEL_V2["notification_panel"])
        await expect(panel).to_be_visible(timeout=5000)
        await expect(panel).to_contain_text("E2E scheduled report")
        assert await panel.evaluate(
            "element => getComputedStyle(element).zIndex"
        ) == "9999"

        await page.locator(SEL_V2["notification_row"]).first.click()
        await expect(page).to_have_url(
            re.compile(rf".*/v2/chat/{THREAD_ID}(?:\?.*)?$"),
            timeout=5000,
        )
    finally:
        await context.close()


async def test_reborn_v2_notification_open_marks_read_without_hiding_pending_message(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await _route_notification_threads(page)
        await _open_v2(page, reborn_v2_server)

        await expect(page.locator(SEL_V2["notification_unread_dot"])).to_be_visible(
            timeout=5000
        )
        await page.locator(SEL_V2["notification_bell"]).click()
        await page.locator(SEL_V2["notification_row"]).first.click()
        await expect(page).to_have_url(
            re.compile(rf".*/v2/chat/{THREAD_ID}(?:\?.*)?$"),
            timeout=5000,
        )

        await expect(page.locator(SEL_V2["notification_unread_dot"])).to_have_count(0)
        await page.locator(SEL_V2["notification_bell"]).click()
        panel = page.locator(SEL_V2["notification_panel"])
        await expect(panel).to_be_visible(timeout=5000)
        await expect(panel).to_contain_text("E2E scheduled report")
        await expect(panel.locator(SEL_V2["notification_row"])).to_have_count(1)
    finally:
        await context.close()


async def test_reborn_v2_notification_drawer_and_header_actions_fit_mobile(
    reborn_v2_server,
    reborn_v2_browser,
):
    viewport = {"width": 390, "height": 740}
    context = await reborn_v2_browser.new_context(viewport=viewport)
    page = await context.new_page()
    try:
        await _route_notification_threads(page)
        await _open_v2(page, reborn_v2_server, "/v2/settings/language")

        for selector in (SEL_V2["header_logs_link"], SEL_V2["header_docs_link"]):
            action = page.locator(selector)
            await expect(action).to_be_visible()
            box = await action.bounding_box()
            assert box is not None
            assert box["width"] <= 40
            assert box["height"] <= 40

        await page.locator(SEL_V2["notification_bell"]).click()
        panel = page.locator(SEL_V2["notification_panel"])
        await expect(panel).to_be_visible(timeout=5000)
        box = await panel.bounding_box()
        assert box is not None
        assert box["x"] <= 1
        assert box["width"] >= viewport["width"] - 2
        assert box["y"] > viewport["height"] * 0.2
        assert box["y"] + box["height"] >= viewport["height"] - 2
    finally:
        await context.close()


async def test_reborn_v2_error_toast_pauses_dismisses_and_stays_above_notifications(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await page.clock.install()
        await _route_notification_threads(page)
        await _route_thread_delete_failure(page)
        await _open_v2(page, reborn_v2_server)

        delete_button = page.locator(
            SEL_V2["thread_delete_for"].format(id=THREAD_ID)
        )
        await expect(delete_button).to_be_visible(timeout=5000)
        await delete_button.click()
        await expect(page.locator(SEL_V2["confirm_dialog_confirm"])).to_be_visible()
        await page.locator(SEL_V2["confirm_dialog_confirm"]).click()

        toast = page.locator(SEL_V2["toast"])
        await expect(toast).to_be_visible(timeout=5000)
        await expect(toast).to_have_attribute("role", "alert")
        await expect(toast).to_have_attribute("aria-live", "assertive")
        await expect(page.locator(SEL_V2["toast_dismiss"])).to_be_visible()

        # Hover beyond the full eight-second error duration. The toast must
        # retain its remaining lifetime rather than expiring underneath the user.
        await toast.hover()
        await page.clock.fast_forward(8500)
        await expect(toast).to_be_visible()

        await page.locator(SEL_V2["notification_bell"]).click()
        panel = page.locator(SEL_V2["notification_panel"])
        await expect(panel).to_be_visible(timeout=5000)
        toast_z = await page.locator(SEL_V2["toast_viewport"]).evaluate(
            "element => Number(getComputedStyle(element).zIndex)"
        )
        panel_z = await panel.evaluate(
            "element => Number(getComputedStyle(element).zIndex)"
        )
        assert toast_z > panel_z, (toast_z, panel_z)

        await page.locator(SEL_V2["toast_dismiss"]).click()
        await page.clock.fast_forward(1000)
        await expect(toast).to_have_count(0, timeout=3000)
    finally:
        await context.close()
