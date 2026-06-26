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
                    "max_files_per_message": 4,
                    "max_bytes_per_file": 1048576,
                    "max_bytes_per_message": 4194304,
                },
            },
        )

    async def handle_threads(route):
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

    await page.goto(f"{reborn_v2_server}/v2/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)

    return {
        "context": context,
        "page": page,
        "timeline": timeline,
        "timeline_requests": timeline_requests,
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

        await page.locator("#gateway-sidebar button").filter(
            has_text="Other thread"
        ).first.click()
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(has_text="Other thread seed")
        ).to_be_visible(timeout=15000)

        await page.locator("#gateway-sidebar button").filter(
            has_text="Pending message regression"
        ).first.click()
        await expect(pending_message).to_have_count(1, timeout=15000)
        await expect(pending_message).to_contain_text("SSE reconnect race test 12345")
        assert len(harness["timeline_requests"]) > pending_timeline_requests
        assert harness["send_requests"][0]["content"] == "SSE reconnect race test 12345"
    finally:
        release_send.set()
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
