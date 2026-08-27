"""Reborn WebUI v2 notification center E2E coverage."""

import asyncio
import json
import re
import uuid
from urllib.parse import quote, urlparse

import httpx
import pytest
from playwright.async_api import expect

from helpers import (
    REBORN_V2_AUTH_TOKEN,
    SEL_V2,
    sse_stream,
    wait_for_sse_line,
)
from notification_e2e_helpers import (
    create_notification_automation,
    delete_notification_automation,
    run_notification_automation,
    wait_for_notification,
)
from reborn_webui_harness import (
    client_action_id,
    install_fake_v2_event_stream,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_bearer_headers,
    reborn_v2_server,  # noqa: F401 - imported fixture
    reborn_v2_yolo_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-e2e-notification"
NOTIFICATION_ID = "notification-e2e-auth"
COMPLETION_NOTIFICATION_ID = "notification-e2e-completed"
RUN_ID = "run-e2e-notification"


async def _wait_for_sse_event(
    response,
    *event_types: str,
    timeout: float = 60.0,
    run_id: str | None = None,
) -> dict:
    matched_payload = None

    def matches(line: str) -> bool:
        nonlocal matched_payload
        if not line.startswith("data:"):
            return False
        try:
            payload = json.loads(line.removeprefix("data:").strip())
        except json.JSONDecodeError:
            return False
        if payload.get("type") not in event_types:
            return False
        if run_id is not None and payload.get("prompt", {}).get("turn_run_id") != run_id:
            return False
        matched_payload = payload
        return True

    await wait_for_sse_line(response, predicate=matches, timeout=timeout)
    assert matched_payload is not None
    return matched_payload


async def _set_tool_permission(
    client: httpx.AsyncClient,
    base_url: str,
    capability_id: str,
    state: str,
) -> None:
    response = await client.post(
        f"{base_url}/api/webchat/v2/settings/tools/{capability_id}",
        json={"state": state},
        timeout=15,
    )
    assert response.status_code == 200, response.text


async def _read_tool_permission(
    client: httpx.AsyncClient,
    base_url: str,
    capability_id: str,
) -> tuple[str, str]:
    response = await client.get(
        f"{base_url}/api/webchat/v2/settings/tools",
        timeout=15,
    )
    assert response.status_code == 200, response.text
    key = f"tool.{capability_id}"
    entry = next(
        (item for item in response.json().get("entries", []) if item.get("key") == key),
        None,
    )
    assert entry is not None, response.text
    value = entry.get("value") or {}
    state = value.get("state")
    source = value.get("effective_source")
    assert state in {"always_allow", "ask_each_time", "disabled"}, entry
    assert source in {"default", "global", "override"}, entry
    return state, source


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
        # What the thread actually showed at the instant the read arrived. The
        # DOM can only be inspected after the response resolves, by which time a
        # correct and an incorrect implementation look identical — so the answer
        # has to be captured while the request is still in flight.
        "rendered_when_read": None,
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
            if state["rendered_when_read"] is None:
                state["rendered_when_read"] = await page.evaluate(
                    """
                    (selector) => [...document.querySelectorAll(selector)]
                      .map((node) => node.textContent || "")
                      .join("\\n")
                    """,
                    SEL_V2["msg_assistant"],
                )
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


async def test_reborn_v2_notification_chunk_shows_loading_shell(
    reborn_v2_server,
    reborn_v2_browser,
):
    viewport = {"width": 1280, "height": 720}
    context = await reborn_v2_browser.new_context(viewport=viewport)
    page = await context.new_page()
    release_chunk = asyncio.Event()

    async def delay_notification_chunk(route):
        await release_chunk.wait()
        await route.continue_()

    try:
        await page.route("**/assets/notification-panel-*.js", delay_notification_chunk)
        await _route_notification_inbox(page)
        await _open_v2(page, reborn_v2_server)

        bell = page.locator(SEL_V2["notification_bell"])
        await bell.click()
        loading = page.locator(SEL_V2["notification_panel_loading"])
        await expect(loading).to_be_visible(timeout=5000)
        await expect(loading).to_have_attribute("role", "status")
        await expect(loading).to_have_attribute("aria-busy", "true")

        box = await loading.bounding_box()
        assert box is not None
        assert box["x"] > viewport["width"] / 2
        assert box["y"] >= 60
        assert box["height"] > 150

        await page.keyboard.press("Escape")
        await expect(loading).to_have_count(0)
        await expect(bell).to_have_attribute("aria-expanded", "false")
        assert await bell.evaluate("element => element === document.activeElement")

        await bell.click()
        await expect(loading).to_be_visible(timeout=5000)
        release_chunk.set()
        await expect(page.locator(SEL_V2["notification_panel"])).to_be_visible(
            timeout=5000
        )
        await expect(loading).to_have_count(0)
    finally:
        release_chunk.set()
        await context.close()


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

        # A reload drops the React Query cache and asks the server again. The
        # record must remain read because Inbox state, not browser storage,
        # owns the lifecycle.
        await page.reload()
        await expect(page.locator(SEL_V2["notification_unread_dot"])).to_have_count(0)
        await page.locator(SEL_V2["notification_bell"]).click()
        reloaded_panel = page.locator(SEL_V2["notification_panel"])
        await expect(reloaded_panel).to_contain_text("Authentication required")
        await expect(reloaded_panel.locator(SEL_V2["notification_row"])).to_have_count(1)
    finally:
        await context.close()


@pytest.mark.parametrize(
    ("kind", "expected_text"),
    [
        ("approval_required", "Approval required"),
        ("run_failed", "Run failed"),
    ],
)
async def test_reborn_v2_notification_inbox_presents_actionable_and_failed_runs(
    reborn_v2_server,
    reborn_v2_browser,
    kind,
    expected_text,
):
    """The generic Inbox presents producer kinds without a legacy thread feed."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await _route_notification_inbox(page, kind=kind)
        await _open_v2(page, reborn_v2_server)

        await page.locator(SEL_V2["notification_bell"]).click()
        panel = page.locator(SEL_V2["notification_panel"])
        await expect(panel).to_contain_text(expected_text)
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
        unrelated_reply = page.locator(SEL_V2["msg_assistant"]).filter(
            has_text="An unrelated run answered."
        )
        await unrelated_reply.first.wait_for(state="visible", timeout=5000)
        assert await unrelated_reply.count() == 1
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

        matching_reply = page.locator(SEL_V2["msg_assistant"]).filter(
            has_text="The scheduled report is ready."
        )
        await matching_reply.first.wait_for(state="visible", timeout=5000)
        assert await matching_reply.count() == 1
        assert state["read_at"] is not None
        # The point of deferring the acknowledgement is that the answer is on
        # screen before the notification is settled. Asserting the DOM after the
        # response has resolved cannot tell the two orderings apart.
        assert state["rendered_when_read"] is not None
        assert "The scheduled report is ready." in state["rendered_when_read"], (
            "the read request was sent before the matching final reply rendered: "
            f"{state['rendered_when_read']!r}"
        )
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


async def test_reborn_v2_scheduled_approval_notification_resolves_with_gate(
    reborn_v2_server,
):
    """A real scheduled approval gate owns one durable Inbox lifecycle."""
    label = uuid.uuid4().hex[:10]
    automation_id = None
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        prior_state, prior_source = await _read_tool_permission(
            client,
            reborn_v2_server,
            "builtin.echo",
        )
        restore_state = "default" if prior_source in {"default", "global"} else prior_state
        await _set_tool_permission(
            client,
            reborn_v2_server,
            "builtin.echo",
            "ask_each_time",
        )
        try:
            automation = await create_notification_automation(
                client,
                reborn_v2_server,
                "approval",
                label,
            )
            automation_id = automation["automation_id"]
            run = await run_notification_automation(
                client,
                reborn_v2_server,
                automation_id,
            )
            run_id = run["run_id"]
            thread_id = run["thread_id"]

            async with sse_stream(
                reborn_v2_server,
                path=f"/api/webchat/v2/threads/{thread_id}/events",
                token=REBORN_V2_AUTH_TOKEN,
                timeout=90,
            ) as stream:
                assert stream.status == 200
                event = await _wait_for_sse_event(
                    stream,
                    "gate",
                    timeout=60,
                    run_id=run_id,
                )
                prompt = event["prompt"]
                assert prompt["approval_context"]["tool_name"] == "builtin.echo"
                notification = await wait_for_notification(
                    client,
                    reborn_v2_server,
                    kind="approval_required",
                    run_id=run_id,
                    resolved=False,
                )

                resolved = await client.post(
                    f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
                    f"/runs/{run_id}/gates/{quote(prompt['gate_ref'], safe='')}/resolve",
                    json={
                        "client_action_id": client_action_id(),
                        "resolution": "approved",
                        "always": False,
                    },
                    timeout=15,
                )
                assert resolved.status_code == 200, resolved.text

            settled = await wait_for_notification(
                client,
                reborn_v2_server,
                kind="approval_required",
                run_id=run_id,
                resolved=True,
            )
            assert settled["id"] == notification["id"]
        finally:
            try:
                await _set_tool_permission(
                    client,
                    reborn_v2_server,
                    "builtin.echo",
                    restore_state,
                )
            finally:
                await delete_notification_automation(
                    client,
                    reborn_v2_server,
                    automation_id,
                )


async def test_reborn_v2_scheduled_auth_notification_resolves_with_manual_token(
    reborn_v2_yolo_server,
):
    """A real scheduled auth gate resolves the same durable Inbox row."""
    label = uuid.uuid4().hex[:10]
    raw_token = f"ghp_notification_e2e_{uuid.uuid4().hex}"
    automation_id = None
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        try:
            automation = await create_notification_automation(
                client,
                reborn_v2_yolo_server,
                "authentication",
                label,
            )
            automation_id = automation["automation_id"]
            run = await run_notification_automation(
                client,
                reborn_v2_yolo_server,
                automation_id,
            )
            run_id = run["run_id"]
            thread_id = run["thread_id"]

            async with sse_stream(
                reborn_v2_yolo_server,
                path=f"/api/webchat/v2/threads/{thread_id}/events",
                token=REBORN_V2_AUTH_TOKEN,
                timeout=120,
            ) as stream:
                assert stream.status == 200
                event = await _wait_for_sse_event(
                    stream,
                    "auth_required",
                    timeout=75,
                    run_id=run_id,
                )
                prompt = event["prompt"]
                assert prompt["provider"] == "github"
                notification = await wait_for_notification(
                    client,
                    reborn_v2_yolo_server,
                    kind="authentication_required",
                    run_id=run_id,
                    resolved=False,
                )

                submitted = await client.post(
                    f"{reborn_v2_yolo_server}/api/reborn/product-auth/manual-token/submit",
                    json={
                        "provider": "github",
                        "account_label": "Notification E2E GitHub",
                        "token": raw_token,
                        "thread_id": thread_id,
                        "run_id": run_id,
                        "gate_ref": prompt["auth_request_ref"],
                    },
                    timeout=15,
                )
                assert submitted.status_code == 200, submitted.text
                assert raw_token not in submitted.text

            settled = await wait_for_notification(
                client,
                reborn_v2_yolo_server,
                kind="authentication_required",
                run_id=run_id,
                resolved=True,
            )
            assert settled["id"] == notification["id"]
        finally:
            await delete_notification_automation(
                client,
                reborn_v2_yolo_server,
                automation_id,
            )


async def test_reborn_v2_scheduled_outcome_notifications_and_read_persistence(
    reborn_v2_server,
):
    """Authoritative scheduled completion/failure outcomes persist in Inbox."""
    completed_id = None
    failed_id = None
    completed_label = uuid.uuid4().hex[:10]
    failed_label = uuid.uuid4().hex[:10]
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        try:
            completed = await create_notification_automation(
                client,
                reborn_v2_server,
                "completed",
                completed_label,
            )
            completed_id = completed["automation_id"]
            completed_run = await run_notification_automation(
                client,
                reborn_v2_server,
                completed_id,
            )
            completed_notification = await wait_for_notification(
                client,
                reborn_v2_server,
                kind="run_completed",
                run_id=completed_run["run_id"],
            )

            marked = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/notifications/"
                f"{completed_notification['id']}/read",
                timeout=15,
            )
            assert marked.status_code == 200, marked.text

            async with httpx.AsyncClient(headers=reborn_bearer_headers()) as reloaded:
                persisted = await wait_for_notification(
                    reloaded,
                    reborn_v2_server,
                    kind="run_completed",
                    run_id=completed_run["run_id"],
                )
            assert persisted["read_at"] is not None

            failed = await create_notification_automation(
                client,
                reborn_v2_server,
                "failed",
                failed_label,
            )
            failed_id = failed["automation_id"]
            failed_run = await run_notification_automation(
                client,
                reborn_v2_server,
                failed_id,
            )
            failed_notification = await wait_for_notification(
                client,
                reborn_v2_server,
                kind="run_failed",
                run_id=failed_run["run_id"],
                timeout=120,
            )
            assert failed_notification["severity"] == "error"
        finally:
            await delete_notification_automation(
                client,
                reborn_v2_server,
                failed_id,
            )
            await delete_notification_automation(
                client,
                reborn_v2_server,
                completed_id,
            )
