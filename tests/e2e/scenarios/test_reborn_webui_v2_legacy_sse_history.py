"""Legacy SSE reconnect and history persistence coverage ported to Reborn v2."""

from contextlib import AsyncExitStack
import json
from urllib.parse import parse_qs, urlparse

import httpx
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2, sse_stream, wait_for_sse_comment
from reborn_webui_harness import (
    USER_ID,
    create_thread as _create_thread,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
    send_and_settle as _send_and_settle,
    send_message as _send_message,
)


THREAD_ID = "thread-legacy-sse-history"
THREAD_A_ID = "thread-legacy-sse-a"
THREAD_B_ID = "thread-legacy-sse-b"
STALE_REPLAY_THREAD_ID = "thread-legacy-sse-stale-replay"
LATE_TEXT_PROJECTION_THREAD_ID = "thread-legacy-late-text-projection"


async def test_reborn_legacy_message_persists_across_page_reload(
    reborn_v2_server, reborn_v2_browser
):
    """Port of the legacy reload persistence check to the v2 timeline."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)
        await _send_and_settle(
            client,
            reborn_v2_server,
            thread_id,
            "What is 2+2?",
            expected=1,
        )

    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await page.goto(f"{reborn_v2_server}/v2/chat/{thread_id}?token={REBORN_V2_AUTH_TOKEN}")
        await expect(page.locator(SEL_V2["msg_user"]).filter(has_text="What is 2+2?")).to_be_visible(
            timeout=15000
        )
        await expect(page.locator(SEL_V2["msg_assistant"]).filter(has_text="4")).to_be_visible(
            timeout=15000
        )

        await page.reload()
        await expect(page.locator(SEL_V2["msg_user"]).filter(has_text="What is 2+2?")).to_be_visible(
            timeout=15000
        )
        await expect(page.locator(SEL_V2["msg_assistant"]).filter(has_text="4")).to_be_visible(
            timeout=15000
        )
    finally:
        await context.close()


async def test_reborn_legacy_sse_resume_reuses_last_cursor_without_history_reload(
    reborn_v2_server, reborn_v2_browser
):
    """A hidden-tab SSE pause resumes with after_cursor and leaves history DOM intact."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    timeline_requests: list[str] = []

    await page.add_init_script(
        """
        (() => {
          const streams = [];
          window.__v2SseUrls = [];
          class FakeEventSource extends EventTarget {
            constructor(url) {
              super();
              this.url = url;
              this.readyState = 0;
              streams.push(this);
              window.__v2SseUrls.push(url);
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

    async def fulfill_json(route, body):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(body),
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
                "threads": [
                    {
                        "thread_id": THREAD_ID,
                        "title": "Legacy SSE history port",
                        "created_at": "2026-06-25T00:00:00Z",
                        "updated_at": "2026-06-25T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        timeline_requests.append(route.request.url)
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "seed-user",
                        "kind": "user",
                        "content": "SSE reconnect should preserve this message",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-25T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{THREAD_ID}/timeline**", handle_timeline)

    try:
        await page.goto(f"{reborn_v2_server}/v2/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")
        user_message = page.locator(SEL_V2["msg_user"]).first
        await expect(user_message).to_contain_text(
            "SSE reconnect should preserve this message", timeout=15000
        )
        await page.wait_for_function("() => window.__v2SseUrls.length === 1", timeout=5000)

        await page.evaluate("() => window.__emitV2Sse('keep_alive', {}, 'cursor-42')")
        await page.evaluate(
            """
            () => {
              const msg = document.querySelector('[data-testid="msg-user"]');
              if (msg) msg.setAttribute('data-e2e-preserved', 'yes');
              Object.defineProperty(document, 'visibilityState', {
                configurable: true,
                get: () => 'hidden',
              });
              document.dispatchEvent(new Event('visibilitychange'));
            }
            """
        )
        await expect(page.locator('[data-e2e-preserved="yes"]')).to_have_count(1)
        timeline_count_before_resume = len(timeline_requests)

        await page.evaluate(
            """
            () => {
              Object.defineProperty(document, 'visibilityState', {
                configurable: true,
                get: () => 'visible',
              });
              document.dispatchEvent(new Event('visibilitychange'));
            }
            """
        )
        await page.wait_for_function("() => window.__v2SseUrls.length === 2", timeout=5000)
        await page.wait_for_timeout(500)

        resumed_url = await page.evaluate("() => window.__v2SseUrls[1]")
        query = parse_qs(urlparse(resumed_url).query)
        assert query.get("token") == [REBORN_V2_AUTH_TOKEN]
        assert query.get("after_cursor") == ["cursor-42"]
        assert len(timeline_requests) == timeline_count_before_resume, timeline_requests
        await expect(page.locator('[data-e2e-preserved="yes"]')).to_have_count(1)
    finally:
        await context.close()


async def test_reborn_legacy_sse_error_reconnect_resumes_after_last_cursor(
    reborn_v2_server, reborn_v2_browser
):
    """Port legacy EventSource reconnect to Reborn's after_cursor resume path."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    await page.add_init_script(
        """
        (() => {
          const streams = [];
          window.__v2SseUrls = [];
          class FakeEventSource extends EventTarget {
            constructor(url) {
              super();
              this.url = url;
              this.readyState = 0;
              streams.push(this);
              window.__v2SseUrls.push(url);
              queueMicrotask(() => {
                this.readyState = 1;
                if (typeof this.onopen === "function") this.onopen(new Event("open"));
              });
            }
            close() {
              this.readyState = 2;
            }
          }
          window.EventSource = FakeEventSource;
          window.__emitV2Sse = (type, frame, id = "cursor-before-error") => {
            const stream = streams[streams.length - 1];
            if (!stream) throw new Error("no EventSource stream is open");
            const event = new MessageEvent(type, {
              data: JSON.stringify({ type, ...frame }),
              lastEventId: id,
            });
            stream.dispatchEvent(event);
          };
          window.__failLatestV2Sse = () => {
            const stream = streams[streams.length - 1];
            if (!stream) throw new Error("no EventSource stream is open");
            if (typeof stream.onerror !== "function") {
              throw new Error("EventSource has no error handler");
            }
            stream.onerror(new Event("error"));
          };
        })();
        """
    )

    async def fulfill_json(route, body):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(body),
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
                "threads": [
                    {
                        "thread_id": THREAD_ID,
                        "title": "Legacy SSE error reconnect port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "error-reconnect-user",
                        "kind": "user",
                        "content": "Reconnect should resume after this cursor",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{THREAD_ID}/timeline**", handle_timeline)

    try:
        await page.goto(f"{reborn_v2_server}/v2/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")
        await expect(
            page.locator(SEL_V2["msg_user"]).filter(
                has_text="Reconnect should resume after this cursor"
            )
        ).to_be_visible(timeout=15000)
        await page.wait_for_function("() => window.__v2SseUrls.length === 1", timeout=5000)

        await page.evaluate(
            "() => window.__emitV2Sse('keep_alive', {}, 'cursor-before-error')"
        )
        await page.evaluate("() => window.__failLatestV2Sse()")
        await page.wait_for_function("() => window.__v2SseUrls.length === 2", timeout=5000)

        reconnected_url = await page.evaluate("() => window.__v2SseUrls[1]")
        query = parse_qs(urlparse(reconnected_url).query)
        assert query.get("token") == [REBORN_V2_AUTH_TOKEN]
        assert query.get("after_cursor") == ["cursor-before-error"]
    finally:
        await context.close()


async def test_reborn_legacy_usage_event_does_not_render_message_badge(
    reborn_v2_server, reborn_v2_browser
):
    """Port of legacy turn_cost-event rendering guard to Reborn's event stream."""
    thread_id = "thread-legacy-usage-event"
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

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
          window.__emitV2MessageSse = (frame, id = "cursor-usage") => {
            const stream = streams[streams.length - 1];
            if (!stream) throw new Error("no EventSource stream is open");
            stream.dispatchEvent(new MessageEvent("message", {
              data: JSON.stringify(frame),
              lastEventId: id,
            }));
          };
        })();
        """
    )

    async def fulfill_json(route, body):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(body),
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
                "threads": [
                    {
                        "thread_id": thread_id,
                        "title": "Usage event rendering port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "seed-assistant",
                        "kind": "assistant",
                        "content": "No footer please",
                        "sequence": 1,
                        "status": "completed",
                        "created_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{thread_id}/timeline**", handle_timeline)

    try:
        await page.goto(f"{reborn_v2_server}/v2/chat/{thread_id}?token={REBORN_V2_AUTH_TOKEN}")
        assistant = page.locator(SEL_V2["msg_assistant"]).first
        await expect(assistant).to_contain_text("No footer please", timeout=15000)
        await expect(page.locator(".turn-cost-badge")).to_have_count(0)

        await page.evaluate(
            """
            () => window.__emitV2MessageSse({
              type: "turn_cost",
              thread_id: "thread-legacy-usage-event",
              input_tokens: 632101,
              output_tokens: 0,
              cost_usd: "$1.6296"
            })
            """
        )

        await page.wait_for_timeout(250)
        await expect(page.locator(SEL_V2["msg_assistant"])).to_have_count(1)
        await expect(page.locator(".turn-cost-badge")).to_have_count(0)
        await expect(assistant).not_to_contain_text("632,101 tokens")
        await expect(assistant).not_to_contain_text("$1.6296")
    finally:
        await context.close()


async def test_reborn_legacy_stale_replay_timeline_refresh_dedupes_messages(
    reborn_v2_server, reborn_v2_browser
):
    """Port stale Last-Event-ID duplicate protection to Reborn timeline refresh."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    timeline_requests: list[str] = []

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
          window.__emitV2Sse = (type, frame, id = "cursor-stale-replay") => {
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

    async def fulfill_json(route, body):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(body),
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
                "threads": [
                    {
                        "thread_id": STALE_REPLAY_THREAD_ID,
                        "title": "Legacy stale replay port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    user_record = {
        "message_id": "stale-replay-user",
        "kind": "user",
        "content": "Question before stale replay",
        "sequence": 1,
        "status": "accepted",
        "created_at": "2026-06-26T00:00:00Z",
        "turn_run_id": "run-stale-replay",
    }
    assistant_record = {
        "message_id": "stale-replay-assistant",
        "kind": "assistant",
        "content": "Answer after stale replay",
        "sequence": 2,
        "status": "finalized",
        "created_at": "2026-06-26T00:00:01Z",
        "turn_run_id": "run-stale-replay",
    }

    async def handle_timeline(route):
        timeline_requests.append(route.request.url)
        messages = [user_record]
        if len(timeline_requests) > 1:
            messages = [user_record, user_record, assistant_record, assistant_record]
        await fulfill_json(route, {"messages": messages, "next_cursor": None})

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(
        f"**/api/webchat/v2/threads/{STALE_REPLAY_THREAD_ID}/timeline**",
        handle_timeline,
    )

    try:
        await page.goto(
            f"{reborn_v2_server}/v2/chat/{STALE_REPLAY_THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}"
        )
        user_message = page.locator(SEL_V2["msg_user"]).filter(
            has_text="Question before stale replay"
        )
        await expect(user_message).to_have_count(1, timeout=15000)
        before_terminal_requests = len(timeline_requests)

        await page.evaluate(
            """
            () => window.__emitV2Sse("projection_update", {
              state: {
                items: [
                  {
                    run_status: {
                      run_id: "run-stale-replay",
                      status: "completed"
                    }
                  }
                ]
              }
            }, "cursor-stale-replay-old")
            """
        )
        assistant_message = page.locator(SEL_V2["msg_assistant"]).filter(
            has_text="Answer after stale replay"
        )
        await expect(assistant_message).to_have_count(1, timeout=5000)
        await expect(user_message).to_have_count(1, timeout=5000)
        assert len(timeline_requests) > before_terminal_requests
    finally:
        await context.close()


async def test_reborn_legacy_late_projection_text_dedupes_history_response(
    reborn_v2_server, reborn_v2_browser
):
    """Port legacy late response-event dedupe to Reborn projection text items."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

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
              queueMicrotask(() => {
                this.readyState = 1;
                if (typeof this.onopen === "function") this.onopen(new Event("open"));
              });
            }
            close() {
              this.readyState = 2;
            }
          }
          window.EventSource = FakeEventSource;
          window.__emitV2Sse = (type, frame, id = "cursor-late-text") => {
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

    async def fulfill_json(route, body):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(body),
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
                "threads": [
                    {
                        "thread_id": LATE_TEXT_PROJECTION_THREAD_ID,
                        "title": "Legacy late text projection port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "late-projection-user",
                        "kind": "user",
                        "content": "What is 2+2?",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-26T00:00:00Z",
                        "turn_run_id": "run-late-projection",
                    },
                    {
                        "message_id": "late-projection-assistant",
                        "kind": "assistant",
                        "content": "The answer is 4.",
                        "sequence": 2,
                        "status": "finalized",
                        "created_at": "2026-06-26T00:00:01Z",
                        "turn_run_id": "run-late-projection",
                    },
                ],
                "next_cursor": None,
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(
        f"**/api/webchat/v2/threads/{LATE_TEXT_PROJECTION_THREAD_ID}/timeline**",
        handle_timeline,
    )

    try:
        await page.goto(
            f"{reborn_v2_server}/v2/chat/{LATE_TEXT_PROJECTION_THREAD_ID}"
            f"?token={REBORN_V2_AUTH_TOKEN}"
        )
        answer = page.locator(SEL_V2["msg_assistant"]).filter(
            has_text="The answer is 4."
        )
        await expect(answer).to_have_count(1, timeout=15000)

        await page.evaluate(
            """
            () => window.__emitV2Sse("projection_update", {
              state: {
                items: [
                  {
                    text: {
                      id: "late-projection-assistant",
                      body: "The answer is 4."
                    }
                  }
                ]
              }
            })
            """
        )

        await expect(answer).to_have_count(1, timeout=5000)
        await expect(page.locator(SEL_V2["msg_assistant"])).to_have_count(1)
    finally:
        await context.close()


async def test_reborn_legacy_sse_thread_switch_drops_prior_thread_cursor(
    reborn_v2_server, reborn_v2_browser
):
    """Switching threads must not replay one thread's SSE cursor on another."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    await page.add_init_script(
        """
        (() => {
          const streams = [];
          window.__v2SseUrls = [];
          class FakeEventSource extends EventTarget {
            constructor(url) {
              super();
              this.url = url;
              this.readyState = 0;
              streams.push(this);
              window.__v2SseUrls.push(url);
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

    async def fulfill_json(route, body):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(body),
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
                "threads": [
                    {
                        "thread_id": THREAD_A_ID,
                        "title": "Thread A",
                        "created_at": "2026-06-25T00:00:00Z",
                        "updated_at": "2026-06-25T00:00:00Z",
                    },
                    {
                        "thread_id": THREAD_B_ID,
                        "title": "Thread B",
                        "created_at": "2026-06-25T00:01:00Z",
                        "updated_at": "2026-06-25T00:01:00Z",
                    },
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline_a(route):
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "thread-a-user",
                        "kind": "user",
                        "content": "Message from thread A",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-25T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline_b(route):
        await fulfill_json(
            route,
            {
                "messages": [
                    {
                        "message_id": "thread-b-user",
                        "kind": "user",
                        "content": "Message from thread B",
                        "sequence": 1,
                        "status": "accepted",
                        "created_at": "2026-06-25T00:01:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{THREAD_A_ID}/timeline**", handle_timeline_a)
    await page.route(f"**/api/webchat/v2/threads/{THREAD_B_ID}/timeline**", handle_timeline_b)

    try:
        await page.goto(f"{reborn_v2_server}/v2/chat/{THREAD_A_ID}?token={REBORN_V2_AUTH_TOKEN}")
        thread_a_message = page.locator(SEL_V2["msg_user"]).filter(has_text="Message from thread A")
        await expect(thread_a_message).to_be_visible(timeout=15000)
        await page.wait_for_function("() => window.__v2SseUrls.length === 1", timeout=5000)

        await page.evaluate("() => window.__emitV2Sse('keep_alive', {}, 'cursor-thread-a')")
        await page.locator(SEL_V2["sidebar_button"]).filter(has_text="Thread B").first.click()
        thread_b_message = page.locator(SEL_V2["msg_user"]).filter(has_text="Message from thread B")
        await expect(thread_b_message).to_be_visible(timeout=15000)
        await page.wait_for_function("() => window.__v2SseUrls.length === 2", timeout=5000)

        switched_url = await page.evaluate("() => window.__v2SseUrls[1]")
        parsed = urlparse(switched_url)
        query = parse_qs(parsed.query)
        assert parsed.path.endswith(f"/api/webchat/v2/threads/{THREAD_B_ID}/events")
        assert query.get("token") == [REBORN_V2_AUTH_TOKEN]
        assert "after_cursor" not in query
    finally:
        await context.close()


async def test_reborn_legacy_multiple_tabs_receive_same_response(
    reborn_v2_server, reborn_v2_browser
):
    """Port of legacy multi-tab SSE delivery to Reborn's per-thread event stream."""
    prompt = "hello multi tab reborn response check"
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page_a = await context.new_page()
    page_b = await context.new_page()
    try:
        url = f"{reborn_v2_server}/v2/chat/{thread_id}?token={REBORN_V2_AUTH_TOKEN}"
        await page_a.goto(url)
        await page_b.goto(url)
        await expect(page_a.locator(SEL_V2["chat_composer"])).to_be_visible(
            timeout=15000
        )
        await expect(page_b.locator(SEL_V2["chat_composer"])).to_be_visible(
            timeout=15000
        )

        async with httpx.AsyncClient(headers=headers) as client:
            await _send_message(client, reborn_v2_server, thread_id, prompt)

        for page in (page_a, page_b):
            await expect(page.locator(SEL_V2["msg_user"]).filter(has_text=prompt)).to_be_visible(
                timeout=30000
            )
            await expect(page.locator(SEL_V2["msg_assistant"]).filter(has_text="Hello")).to_be_visible(
                timeout=45000
            )
    finally:
        await context.close()


async def test_reborn_legacy_excess_sse_connections_are_rate_limited(
    reborn_v2_server,
):
    """Port the legacy SSE connection cap to Reborn's per-caller stream limit."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

    events_path = f"/api/webchat/v2/threads/{thread_id}/events"
    params = {"token": REBORN_V2_AUTH_TOKEN}

    async with AsyncExitStack() as stack:
        for _ in range(3):
            response = await stack.enter_async_context(
                sse_stream(
                    reborn_v2_server,
                    events_path,
                    token=REBORN_V2_AUTH_TOKEN,
                    params=params,
                    timeout=15,
                )
            )
            assert response.status == 200, await response.text()
            assert response.headers.get("content-type", "").startswith(
                "text/event-stream"
            )

        async with sse_stream(
            reborn_v2_server,
            events_path,
            token=REBORN_V2_AUTH_TOKEN,
            params=params,
            timeout=15,
        ) as rejected:
            body = await rejected.json(content_type=None)
            assert rejected.status == 429, body
            assert body["error"] == "rate_limited"
            assert body["kind"] == "busy"
            assert body["retryable"] is True


async def test_reborn_legacy_sse_keepalive_comments_arrive(reborn_v2_server):
    """Port the legacy idle SSE keepalive check to Reborn's v2 event stream."""
    headers = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _create_thread(client, reborn_v2_server)

    events_path = f"/api/webchat/v2/threads/{thread_id}/events"
    async with sse_stream(
        reborn_v2_server,
        events_path,
        token=REBORN_V2_AUTH_TOKEN,
        params={"token": REBORN_V2_AUTH_TOKEN},
        timeout=25,
    ) as response:
        assert response.status == 200, await response.text()
        keepalive = await wait_for_sse_comment(response, timeout=22)
        assert keepalive.startswith(":")
