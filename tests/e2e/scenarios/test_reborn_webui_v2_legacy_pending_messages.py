"""Legacy pending-message behavior ported to Reborn WebChat v2."""

import asyncio
import json
from urllib.parse import unquote, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-legacy-pending"
OTHER_THREAD_ID = "thread-legacy-pending-other"
RUN_ID = "11111111-2222-3333-4444-555555555555"


def _user_record(message_id: str, content: str, sequence: int = 1) -> dict:
    return {
        "message_id": message_id,
        "kind": "user",
        "content": content,
        "sequence": sequence,
        "status": "accepted",
        "created_at": "2026-06-25T12:00:00Z",
        "turn_run_id": RUN_ID,
    }


def _assistant_record(message_id: str, content: str, sequence: int = 2) -> dict:
    return {
        "message_id": message_id,
        "kind": "assistant",
        "content": content,
        "sequence": sequence,
        "status": "finalized",
        "created_at": "2026-06-25T12:00:01Z",
        "turn_run_id": RUN_ID,
    }


def _submitted_response() -> dict:
    return {
        "thread_id": THREAD_ID,
        "accepted_message_ref": "msg:confirmed-user",
        "turn_id": "turn-legacy-pending",
        "run_id": RUN_ID,
        "status": "queued",
        "resolved_run_profile_id": "default",
        "resolved_run_profile_version": 1,
        "event_cursor": 1,
    }


async def _install_fake_event_source(page):
    await page.add_init_script(
        """
        (() => {
          const streams = [];
          class FakeEventSource extends EventTarget {
            constructor(url) {
              super();
              this.url = url;
              this.readyState = 0;
              streams.push(this);
              setTimeout(() => {
                this.readyState = 1;
                if (typeof this.onopen === "function") this.onopen(new Event("open"));
              }, 0);
            }
            close() {
              this.readyState = 2;
            }
          }
          window.EventSource = FakeEventSource;
          window.__emitV2Sse = (type, frame, id = "cursor-1") => {
            const stream = streams[streams.length - 1];
            if (!stream) throw new Error("no EventSource stream is open");
            const event = new MessageEvent(type, {
              data: JSON.stringify({ type, ...frame }),
              lastEventId: id,
            });
            stream.dispatchEvent(event);
          };
        })();
        """
    )


async def _wait_for_request_count(requests: list, count: int, *, timeout: float = 5.0) -> None:
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        if len(requests) > count:
            return
        await asyncio.sleep(0.05)
    raise AssertionError(f"Timed out waiting for request count > {count}; got {len(requests)}")


async def _open_mocked_pending_page(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    initial_thread_id=THREAD_ID,
    timeline_messages=None,
    timeline_by_thread=None,
    threads=None,
    send_handler,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await _install_fake_event_source(page)
    timeline = list(timeline_messages or [])
    timelines = {
        thread_id: list(messages)
        for thread_id, messages in (timeline_by_thread or {}).items()
    }
    timelines.setdefault(THREAD_ID, timeline)
    thread_records = list(
        threads
        or [
            {
                "thread_id": THREAD_ID,
                "title": "Pending message regression",
                "created_at": "2026-06-25T00:00:00Z",
                "updated_at": "2026-06-25T00:00:00Z",
            }
        ]
    )
    timeline_requests: list[dict] = []
    thread_requests: list[str] = []
    send_requests: list[dict] = []

    async def fulfill_json(route, body, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(body),
            headers={"Cache-Control": "no-store"},
        )

    async def handle_session(route):
        await fulfill_json(
            route,
            {
                "tenant_id": "reborn-v2-e2e",
                "user_id": USER_ID,
                "capabilities": {},
                "features": {"reborn_projects": False},
                "attachments": {
                    "accept": ["text/plain"],
                    "max_count": 4,
                    "max_file_bytes": 1048576,
                    "max_total_bytes": 4194304,
                },
            },
        )

    async def handle_threads(route):
        thread_requests.append(route.request.url)
        await fulfill_json(
            route,
            {
                "threads": thread_records,
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        parsed = urlparse(route.request.url)
        timeline_requests.append(dict(parsed._asdict()))
        thread_id = unquote(parsed.path.split("/threads/", 1)[1].split("/timeline", 1)[0])
        await fulfill_json(
            route,
            {"messages": timelines.get(thread_id, []), "next_cursor": None},
        )

    async def handle_send(route):
        payload = json.loads(route.request.post_data or "{}")
        send_requests.append(payload)
        await send_handler(route, payload, fulfill_json)

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads/*/timeline**", handle_timeline)
    await page.route(f"**/api/webchat/v2/threads/{THREAD_ID}/messages", handle_send)

    await page.goto(
        f"{reborn_v2_server}/v2/chat/{initial_thread_id}?token={REBORN_V2_AUTH_TOKEN}"
    )
    await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)

    return {
        "context": context,
        "page": page,
        "timeline": timeline,
        "threads": thread_records,
        "timeline_requests": timeline_requests,
        "thread_requests": thread_requests,
        "send_requests": send_requests,
    }


async def test_reborn_legacy_pending_message_visible_while_send_is_in_flight(
    reborn_v2_server, reborn_v2_browser
):
    release_send = asyncio.Event()

    async def handle_delayed_send(route, _payload, fulfill_json):
        await release_send.wait()
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        send_handler=handle_delayed_send,
    )
    try:
        page = harness["page"]
        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("Pending message test")
        await composer.press("Enter")

        await expect(page.locator(SEL_V2["msg_user"]).last).to_contain_text(
            "Pending message test", timeout=5000
        )
        assert harness["send_requests"][0]["content"] == "Pending message test"
        await expect(page.locator(SEL_V2["msg_assistant"])).to_have_count(0)
    finally:
        release_send.set()
        await harness["context"].close()


async def test_reborn_legacy_empty_landing_hidden_when_message_pending(
    reborn_v2_server, reborn_v2_browser
):
    release_send = asyncio.Event()

    async def handle_delayed_send(route, _payload, fulfill_json):
        await release_send.wait()
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        send_handler=handle_delayed_send,
    )
    try:
        page = harness["page"]
        await expect(
            page.get_by_text("Hello, what do you need help with?")
        ).to_be_visible(timeout=5000)

        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("Welcome card suppression test")
        await composer.press("Enter")

        await expect(
            page.locator(SEL_V2["msg_user"]).filter(
                has_text="Welcome card suppression test"
            )
        ).to_have_count(1, timeout=5000)
        await expect(
            page.get_by_text("Hello, what do you need help with?")
        ).to_have_count(0)
        assert harness["send_requests"][0]["content"] == "Welcome card suppression test"
    finally:
        release_send.set()
        await harness["context"].close()


async def test_reborn_legacy_pending_message_reconciles_with_confirmed_timeline(
    reborn_v2_server, reborn_v2_browser
):
    async def handle_successful_send(route, _payload, fulfill_json):
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        send_handler=handle_successful_send,
    )
    try:
        page = harness["page"]
        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("Duplicate check")
        await composer.press("Enter")
        await expect(page.locator(SEL_V2["msg_user"]).last).to_contain_text(
            "Duplicate check", timeout=5000
        )

        harness["timeline"][:] = [
            _user_record("confirmed-user", "Duplicate check"),
            _assistant_record("confirmed-assistant", "The duplicate check is complete."),
        ]
        before_reload_requests = len(harness["timeline_requests"])

        await page.evaluate(
            f"""
            () => window.__emitV2Sse("projection_update", {{
              state: {{
                items: [
                  {{ run_status: {{ run_id: {RUN_ID!r}, status: "completed" }} }}
                ]
              }}
            }})
            """
        )

        await _wait_for_request_count(harness["timeline_requests"], before_reload_requests)
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="Duplicate check")
        ).to_have_count(1, timeout=5000)
        await expect(page.locator(SEL_V2["msg_assistant"]).last).to_contain_text(
            "The duplicate check is complete.", timeout=5000
        )
    finally:
        await harness["context"].close()


async def test_reborn_legacy_pending_message_survives_thread_reload(
    reborn_v2_server, reborn_v2_browser
):
    release_send = asyncio.Event()

    async def handle_delayed_send(route, _payload, fulfill_json):
        await release_send.wait()
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        threads=[
            {
                "thread_id": THREAD_ID,
                "title": "Pending message regression",
                "created_at": "2026-06-25T00:00:00Z",
                "updated_at": "2026-06-25T00:00:00Z",
            },
            {
                "thread_id": OTHER_THREAD_ID,
                "title": "Other thread",
                "created_at": "2026-06-25T00:01:00Z",
                "updated_at": "2026-06-25T00:01:00Z",
            },
        ],
        timeline_by_thread={
            THREAD_ID: [],
            OTHER_THREAD_ID: [_user_record("other-user", "Other thread seed")],
        },
        send_handler=handle_delayed_send,
    )
    try:
        page = harness["page"]
        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("SSE reconnect race test 12345")
        await composer.press("Enter")

        pending_message = page.locator(SEL_V2["msg_user"]).filter(
            has_text="SSE reconnect race test 12345"
        )
        await expect(pending_message).to_have_count(1, timeout=5000)
        pending_timeline_requests = len(harness["timeline_requests"])

        await page.locator(SEL_V2["sidebar_button"]).filter(
            has_text="Other thread"
        ).first.click()
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="Other thread seed")
        ).to_be_visible(timeout=15000)

        await page.locator(SEL_V2["sidebar_button"]).filter(
            has_text="Pending message regression"
        ).first.click()
        await expect(pending_message).to_have_count(1, timeout=15000)
        await expect(pending_message).to_contain_text("SSE reconnect race test 12345")
        assert len(harness["timeline_requests"]) > pending_timeline_requests
        assert harness["send_requests"][0]["content"] == "SSE reconnect race test 12345"
    finally:
        release_send.set()
        await harness["context"].close()


async def test_reborn_legacy_pending_attachment_message_survives_thread_reload(
    reborn_v2_server, reborn_v2_browser
):
    """Port legacy in-progress attachment reload to Reborn pending messages."""
    release_send = asyncio.Event()

    async def handle_delayed_send(route, _payload, fulfill_json):
        await release_send.wait()
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        threads=[
            {
                "thread_id": THREAD_ID,
                "title": "Pending attachment regression",
                "created_at": "2026-06-25T00:00:00Z",
                "updated_at": "2026-06-25T00:00:00Z",
            },
            {
                "thread_id": OTHER_THREAD_ID,
                "title": "Other thread",
                "created_at": "2026-06-25T00:01:00Z",
                "updated_at": "2026-06-25T00:01:00Z",
            },
        ],
        timeline_by_thread={
            THREAD_ID: [],
            OTHER_THREAD_ID: [_user_record("other-user", "Other thread seed")],
        },
        send_handler=handle_delayed_send,
    )
    try:
        page = harness["page"]
        await expect(
            page.get_by_text("Hello, what do you need help with?")
        ).to_be_visible(timeout=15000)
        async with page.expect_file_chooser() as chooser_info:
            await page.get_by_label("Attach files").click()
        chooser = await chooser_info.value
        await chooser.set_files(
            [
                {
                    "name": "pending-note.txt",
                    "mimeType": "text/plain",
                    "buffer": b"Attachment survives Reborn pending reload.",
                }
            ]
        )
        await expect(page.locator("body")).to_contain_text(
            "pending-note.txt", timeout=15000
        )

        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("Pending attachment reload test")
        await composer.press("Enter")

        pending_message = page.locator(SEL_V2["msg_user"]).filter(
            has_text="Pending attachment reload test"
        )
        await expect(pending_message).to_have_count(1, timeout=5000)
        await expect(pending_message).to_contain_text("pending-note.txt")
        assert harness["send_requests"][0]["content"] == "Pending attachment reload test"
        assert harness["send_requests"][0]["attachments"][0]["filename"] == (
            "pending-note.txt"
        )

        await page.locator(SEL_V2["sidebar_button"]).filter(
            has_text="Other thread"
        ).first.click()
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="Other thread seed")
        ).to_be_visible(timeout=15000)

        await page.locator(SEL_V2["sidebar_button"]).filter(
            has_text="Pending attachment regression"
        ).first.click()
        await expect(pending_message).to_have_count(1, timeout=15000)
        await expect(pending_message).to_contain_text("Pending attachment reload test")
        await expect(pending_message).to_contain_text("pending-note.txt")
    finally:
        release_send.set()
        await harness["context"].close()


async def test_reborn_legacy_sidebar_refresh_keeps_active_thread_outside_summary_window(
    reborn_v2_server, reborn_v2_browser
):
    async def handle_successful_send(route, _payload, fulfill_json):
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        threads=[
            {
                "thread_id": THREAD_ID,
                "title": None,
                "created_at": "2026-06-25T00:00:00Z",
                "updated_at": "2026-06-25T00:00:00Z",
            },
            {
                "thread_id": OTHER_THREAD_ID,
                "title": "Newest summary thread",
                "created_at": "2026-06-25T00:01:00Z",
                "updated_at": "2026-06-25T00:01:00Z",
            },
        ],
        send_handler=handle_successful_send,
    )
    try:
        page = harness["page"]
        composer = page.locator(SEL_V2["chat_composer"])
        await expect(composer).to_be_visible(timeout=15000)
        assert await page.evaluate("() => location.pathname") == f"/v2/chat/{THREAD_ID}"

        harness["threads"][:] = [
            {
                "thread_id": OTHER_THREAD_ID,
                "title": "Newest summary thread",
                "created_at": "2026-06-25T00:01:00Z",
                "updated_at": "2026-06-25T00:01:00Z",
            }
        ]
        before_refresh_requests = len(harness["thread_requests"])

        await composer.fill("Summary refresh should keep this Reborn thread")
        await composer.press("Enter")

        await expect(
            page.locator(SEL_V2["msg_user"]).filter(
                has_text="Summary refresh should keep this Reborn thread"
            )
        ).to_have_count(1, timeout=5000)
        await _wait_for_request_count(
            harness["thread_requests"],
            before_refresh_requests,
        )

        assert len(harness["send_requests"]) == 1
        assert (
            harness["send_requests"][0]["content"]
            == "Summary refresh should keep this Reborn thread"
        )
        assert await page.evaluate("() => location.pathname") == f"/v2/chat/{THREAD_ID}"
        await expect(composer).to_be_visible(timeout=5000)
        await expect(
            page.locator(SEL_V2["sidebar"]).get_by_role("button").filter(
                has_text="Newest summary thread"
            )
        ).to_be_visible(timeout=5000)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_sidebar_running_indicator_clears_on_terminal_run(
    reborn_v2_server, reborn_v2_browser
):
    async def handle_successful_send(route, _payload, fulfill_json):
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        threads=[
            {
                "thread_id": THREAD_ID,
                "title": "Processing thread",
                "created_at": "2026-06-25T00:00:00Z",
                "updated_at": "2026-06-25T00:00:00Z",
            }
        ],
        send_handler=handle_successful_send,
    )
    try:
        page = harness["page"]
        sidebar_thread = page.locator(SEL_V2["sidebar"]).get_by_role("button").filter(
            has_text="Processing thread"
        ).first
        await expect(sidebar_thread).to_be_visible(timeout=5000)

        await page.evaluate(
            f"""
            () => window.__emitV2Sse("projection_update", {{
              state: {{
                items: [
                  {{ run_status: {{ run_id: {RUN_ID!r}, status: "running" }} }}
                ]
              }}
            }})
            """
        )
        await expect(sidebar_thread).to_contain_text("Running", timeout=5000)
        await expect(page.locator(SEL_V2["typing_indicator"])).to_be_visible(
            timeout=5000
        )

        before_terminal_requests = len(harness["timeline_requests"])
        await page.evaluate(
            f"""
            () => window.__emitV2Sse("projection_update", {{
              state: {{
                items: [
                  {{ run_status: {{ run_id: {RUN_ID!r}, status: "completed" }} }}
                ]
              }}
            }})
            """
        )
        await _wait_for_request_count(harness["timeline_requests"], before_terminal_requests)
        await expect(sidebar_thread).not_to_contain_text("Running", timeout=5000)
        await expect(page.locator(SEL_V2["typing_indicator"])).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_background_thread_shows_processing_indicator(
    reborn_v2_server, reborn_v2_browser
):
    """Port background-thread processing affordance to Reborn's thread summary state."""

    async def handle_successful_send(route, _payload, fulfill_json):
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        initial_thread_id=OTHER_THREAD_ID,
        threads=[
            {
                "thread_id": THREAD_ID,
                "title": "Background processing thread",
                "state": "Processing",
                "turn_count": 2,
                "created_at": "2026-06-25T00:00:00Z",
                "updated_at": "2026-06-25T00:02:00Z",
            },
            {
                "thread_id": OTHER_THREAD_ID,
                "title": "Active quiet thread",
                "turn_count": 1,
                "created_at": "2026-06-25T00:01:00Z",
                "updated_at": "2026-06-25T00:03:00Z",
            },
        ],
        timeline_by_thread={
            THREAD_ID: [_user_record("background-user", "Background thread seed")],
            OTHER_THREAD_ID: [_user_record("active-user", "Active thread seed")],
        },
        send_handler=handle_successful_send,
    )
    try:
        page = harness["page"]
        active_thread = page.locator(SEL_V2["sidebar"]).get_by_role("button").filter(
            has_text="Active quiet thread"
        ).first
        background_thread = page.locator(SEL_V2["sidebar"]).get_by_role(
            "button"
        ).filter(has_text="Background processing thread").first

        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="Active thread seed")
        ).to_be_visible(timeout=15000)
        await expect(active_thread).to_be_visible(timeout=5000)
        await expect(background_thread).to_be_visible(timeout=5000)
        await expect(background_thread.get_by_label("Running")).to_have_count(1)
        await expect(active_thread.get_by_label("Running")).to_have_count(0)
        await expect(page.locator(SEL_V2["typing_indicator"])).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_processing_indicator_does_not_leak_after_thread_switch(
    reborn_v2_server, reborn_v2_browser
):
    """Port stale in-progress indicator coverage to Reborn's thread switch path."""

    async def handle_successful_send(route, _payload, fulfill_json):
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        initial_thread_id=THREAD_ID,
        threads=[
            {
                "thread_id": THREAD_ID,
                "title": "Running source thread",
                "turn_count": 1,
                "created_at": "2026-06-25T00:00:00Z",
                "updated_at": "2026-06-25T00:02:00Z",
            },
            {
                "thread_id": OTHER_THREAD_ID,
                "title": "Quiet destination thread",
                "turn_count": 1,
                "created_at": "2026-06-25T00:01:00Z",
                "updated_at": "2026-06-25T00:03:00Z",
            },
        ],
        timeline_by_thread={
            THREAD_ID: [_user_record("running-seed", "Running thread seed")],
            OTHER_THREAD_ID: [_user_record("quiet-seed", "Quiet thread seed")],
        },
        send_handler=handle_successful_send,
    )
    try:
        page = harness["page"]
        sidebar = page.locator(SEL_V2["sidebar"])
        running_thread = sidebar.get_by_role("button").filter(
            has_text="Running source thread"
        ).first
        quiet_thread = sidebar.get_by_role("button").filter(
            has_text="Quiet destination thread"
        ).first

        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="Running thread seed")
        ).to_be_visible(timeout=15000)

        await page.evaluate(
            f"""
            () => window.__emitV2Sse("projection_update", {{
              state: {{
                items: [
                  {{ run_status: {{ run_id: {RUN_ID!r}, status: "running" }} }}
                ]
              }}
            }})
            """
        )
        await expect(page.locator(SEL_V2["typing_indicator"])).to_be_visible(
            timeout=5000
        )
        await expect(running_thread).to_contain_text("Running", timeout=5000)

        await quiet_thread.click()
        await page.wait_for_function(
            "(threadId) => location.pathname === `/v2/chat/${threadId}`",
            arg=OTHER_THREAD_ID,
            timeout=10000,
        )
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="Quiet thread seed")
        ).to_be_visible(timeout=15000)
        await expect(page.locator(SEL_V2["typing_indicator"])).to_have_count(0)
        await expect(quiet_thread.get_by_label("Running")).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_failed_send_marks_single_error_message(
    reborn_v2_server, reborn_v2_browser
):
    async def handle_failed_send(route, _payload, fulfill_json):
        await fulfill_json(
            route,
            {"error": "service_unavailable", "kind": "service_unavailable"},
            status=503,
        )

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        send_handler=handle_failed_send,
    )
    try:
        page = harness["page"]
        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("send-failure cleanup test")
        await composer.press("Enter")

        failed = page.locator(SEL_V2["msg_user"]).filter(
            has_text="send-failure cleanup test"
        )
        await expect(failed).to_have_count(1, timeout=5000)
        await expect(failed).to_contain_text("Service unavailable")
        await expect(failed.get_by_label("Retry message")).to_be_visible()
        assert len(harness["send_requests"]) == 1
    finally:
        await harness["context"].close()


async def test_reborn_legacy_failed_send_retry_resubmits_message(
    reborn_v2_server, reborn_v2_browser
):
    async def handle_fail_then_success(route, _payload, fulfill_json):
        if len(harness["send_requests"]) == 1:
            await fulfill_json(
                route,
                {"error": "service_unavailable", "kind": "service_unavailable"},
                status=503,
            )
            return
        await fulfill_json(route, _submitted_response(), status=202)

    harness = await _open_mocked_pending_page(
        reborn_v2_server,
        reborn_v2_browser,
        send_handler=handle_fail_then_success,
    )
    try:
        page = harness["page"]
        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("retry failed send test")
        await composer.press("Enter")

        failed = page.locator(SEL_V2["msg_user"]).filter(
            has_text="retry failed send test"
        )
        await expect(failed).to_have_count(1, timeout=5000)
        await expect(failed).to_contain_text("Service unavailable")

        await failed.get_by_label("Retry message").click()

        await expect(failed).to_have_count(1, timeout=5000)
        await expect(failed).not_to_contain_text("Service unavailable")
        await expect(failed.get_by_label("Retry message")).to_have_count(0)
        assert [request["content"] for request in harness["send_requests"]] == [
            "retry failed send test",
            "retry failed send test",
        ]
    finally:
        await harness["context"].close()
