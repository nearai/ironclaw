"""Reborn WebUI v2 notification center E2E coverage."""

import json
import re
from urllib.parse import urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    install_fake_v2_event_stream,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-e2e-notification"
NOTIFICATION_ID = "notification-e2e-auth"
COMPLETION_NOTIFICATION_ID = "notification-e2e-completed"
RUN_ID = "run-e2e-notification"


def _notification_threads_payload():
    return {
        "threads": [
            {
                "id": THREAD_ID,
                "thread_id": THREAD_ID,
                "title": "E2E scheduled report",
                "state": "idle",
                "updated_at": "2026-06-30T08:10:01Z",
            }
        ],
        "next_cursor": None,
    }


def _notification_payload(
    read_at=None,
    *,
    notification_id=NOTIFICATION_ID,
    kind="authentication_required",
    turn_run_id=None,
):
    return {
        "notifications": [
            {
                "id": notification_id,
                "kind": kind,
                "severity": "warning",
                "action": {"kind": "open_thread", "thread_id": THREAD_ID},
                "thread_id": THREAD_ID,
                "turn_run_id": turn_run_id,
                "created_at": "2026-06-30T08:10:01Z",
                "updated_at": read_at or "2026-06-30T08:10:01Z",
                "read_at": read_at,
                "resolved_at": None,
            }
        ],
        "next_cursor": None,
        "unread_count": 0 if read_at else 1,
    }


async def _route_notification_inbox(
    page,
    *,
    notification_id=NOTIFICATION_ID,
    kind="authentication_required",
    turn_run_id=None,
):
    state = {
        "read_at": None,
    }

    async def threads_handler(route):
        parsed = urlparse(route.request.url)
        if parsed.path != "/api/webchat/v2/threads" or route.request.method != "GET":
            await route.continue_()
            return
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(_notification_threads_payload()),
        )

    async def notifications_handler(route):
        parsed = urlparse(route.request.url)
        if route.request.method == "GET" and parsed.path == "/api/webchat/v2/notifications":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps(
                    _notification_payload(
                        state["read_at"],
                        notification_id=notification_id,
                        kind=kind,
                        turn_run_id=turn_run_id,
                    )
                ),
            )
            return
        if route.request.method == "POST" and parsed.path in {
            f"/api/webchat/v2/notifications/{notification_id}/read",
            "/api/webchat/v2/notifications/read-all",
        }:
            state["read_at"] = "2026-06-30T08:11:00Z"
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"updated": True}),
            )
            return
        await route.continue_()

    await page.route("**/api/webchat/v2/threads*", threads_handler)
    await page.route("**/api/webchat/v2/notifications*", notifications_handler)
    await page.route("**/api/webchat/v2/notifications/**", notifications_handler)
    return state


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


async def _open_v2(page, base_url, path="/"):
    separator = "&" if "?" in path else "?"
    await page.goto(f"{base_url}{path}{separator}token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.locator(SEL_V2["notification_bell"])).to_be_visible(timeout=15000)


async def test_reborn_v2_notification_popover_opens_server_inbox_thread(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await _route_notification_inbox(page)
        await _open_v2(page, reborn_v2_server)

        await page.locator(SEL_V2["notification_bell"]).click()
        panel = page.locator(SEL_V2["notification_panel"])
        await expect(panel).to_be_visible(timeout=5000)
        await expect(panel).to_contain_text("Authentication required")
        assert await panel.evaluate(
            "element => getComputedStyle(element).zIndex"
        ) == "9999"

        await page.locator(SEL_V2["notification_row"]).first.click()
        await expect(page).to_have_url(
            re.compile(rf".*/chat/{THREAD_ID}(?:\?.*)?$"),
            timeout=5000,
        )
    finally:
        await context.close()


async def test_reborn_v2_notification_open_persists_read_without_hiding_message(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        state = await _route_notification_inbox(page)
        await _open_v2(page, reborn_v2_server)

        await expect(page.locator(SEL_V2["notification_unread_dot"])).to_be_visible(
            timeout=5000
        )
        await page.locator(SEL_V2["notification_bell"]).click()
        async with page.expect_response(
            lambda response: response.request.method == "POST"
            and urlparse(response.url).path
            == f"/api/webchat/v2/notifications/{NOTIFICATION_ID}/read"
            and response.status == 200
        ):
            await page.locator(SEL_V2["notification_row"]).first.click()
        await expect(page).to_have_url(
            re.compile(rf".*/chat/{THREAD_ID}(?:\?.*)?$"),
            timeout=5000,
        )
        assert state["read_at"] is not None

        await expect(page.locator(SEL_V2["notification_unread_dot"])).to_have_count(0)
        await page.locator(SEL_V2["notification_bell"]).click()
        panel = page.locator(SEL_V2["notification_panel"])
        await expect(panel).to_be_visible(timeout=5000)
        await expect(panel).to_contain_text("Authentication required")
        await expect(panel.locator(SEL_V2["notification_row"])).to_have_count(1)
    finally:
        await context.close()


async def test_reborn_v2_completion_waits_for_matching_final_reply_render(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await install_fake_v2_event_stream(page)
        state = await _route_notification_inbox(
            page,
            notification_id=COMPLETION_NOTIFICATION_ID,
            kind="run_completed",
            turn_run_id=RUN_ID,
        )

        async def timeline_handler(route):
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"messages": [], "next_cursor": None}),
            )

        await page.route(
            f"**/api/webchat/v2/threads/{THREAD_ID}/timeline**",
            timeline_handler,
        )
        await _open_v2(page, reborn_v2_server)

        await page.locator(SEL_V2["notification_bell"]).click()
        await page.locator(SEL_V2["notification_row"]).first.click()
        await expect(page).to_have_url(
            re.compile(rf".*/chat/{THREAD_ID}(?:\?.*)?$"),
            timeout=5000,
        )
        await page.wait_for_function("() => window.__v2SseHasOpenStream?.() === true")
        await expect(page.locator(SEL_V2["msg_assistant"])).to_have_count(0)
        assert state["read_at"] is None

        # A final reply from a different run must not acknowledge this record:
        # the notification describes one run's outcome, and marking it read on
        # any final reply in the thread would lose that evidence.
        await page.evaluate(
            """
            ([runId, text]) => window.__emitV2Sse("final_reply", {
              reply: {
                turn_run_id: runId,
                text,
                generated_at: "2026-06-30T08:10:00Z"
              }
            }, "cursor-notification-other")
            """,
            ["run-unrelated", "An unrelated run answered."],
        )
        await expect(page.locator(SEL_V2["msg_assistant"])).to_contain_text(
            "An unrelated run answered.",
            timeout=5000,
        )
        assert state["read_at"] is None

        async with page.expect_response(
            lambda response: response.request.method == "POST"
            and urlparse(response.url).path
            == f"/api/webchat/v2/notifications/{COMPLETION_NOTIFICATION_ID}/read"
            and response.status == 200
        ):
            await page.evaluate(
                """
                ([runId, text]) => window.__emitV2Sse("final_reply", {
                  reply: {
                    turn_run_id: runId,
                    text,
                    generated_at: "2026-06-30T08:11:00Z"
                  }
                }, "cursor-notification-final")
                """,
                [RUN_ID, "The scheduled report is ready."],
            )

        await expect(page.locator(SEL_V2["msg_assistant"])).to_contain_text(
            "The scheduled report is ready.",
            timeout=5000,
        )
        assert state["read_at"] is not None
    finally:
        await context.close()


async def test_reborn_v2_notification_mark_all_read_updates_server_inbox(
    reborn_v2_server,
    reborn_v2_browser,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        state = await _route_notification_inbox(page)
        await _open_v2(page, reborn_v2_server)

        await page.locator(SEL_V2["notification_bell"]).click()
        async with page.expect_response(
            lambda response: response.request.method == "POST"
            and urlparse(response.url).path
            == "/api/webchat/v2/notifications/read-all"
            and response.status == 200
        ):
            await page.locator(SEL_V2["notification_mark_all_read"]).click()
        assert state["read_at"] is not None
        await expect(page.locator(SEL_V2["notification_unread_dot"])).to_have_count(0)
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
        await _route_notification_inbox(page)
        await _open_v2(page, reborn_v2_server, "/settings/language")

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
        await _route_notification_inbox(page)
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

        # A failed deletion keeps the confirmation dialog open so the user can
        # retry. Close it before exercising controls behind the modal backdrop.
        await page.locator(SEL_V2["confirm_dialog_cancel"]).click()
        await expect(page.locator(SEL_V2["confirm_dialog_confirm"])).to_have_count(0)

        # Hover beyond the full eight-second error duration. The toast must
        # retain its remaining lifetime rather than expiring underneath the user.
        await toast.hover()
        await page.clock.fast_forward(8500)
        await expect(toast).to_be_visible()

        # The dialog was already dismissed above, so the bell is reachable.
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
