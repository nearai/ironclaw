"""Legacy DOM resource-limit coverage ported to Reborn WebUI v2 history paging."""

import json
from urllib.parse import parse_qs, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    install_fake_v2_event_stream,
    USER_ID,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-legacy-dom-resource-limits"
FULL_PAGE_THREAD_ID = "thread-legacy-dom-full-page-response"
RECONNECT_THREAD_ID = "thread-legacy-dom-reconnect"
FULL_PAGE_SEND_THREAD_ID = "thread-legacy-dom-full-page-send"


def _timeline_message(sequence: int) -> dict:
    kind = "user" if sequence % 2 else "assistant"
    return {
        "message_id": f"dom-message-{sequence}",
        "kind": kind,
        "content": f"DOM page message {sequence}",
        "sequence": sequence,
        "status": "accepted" if kind == "user" else "finalized",
        "created_at": "2026-06-26T00:00:00Z",
    }


async def test_reborn_chat_history_dom_stays_page_bounded_until_user_loads_more(
    reborn_v2_server, reborn_v2_browser
):
    """Reborn replaces legacy DOM pruning with explicit 50-message timeline paging."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    timeline_queries: list[dict[str, list[str]]] = []

    await install_fake_v2_event_stream(page)

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
                        "title": "Legacy DOM resource port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        query = parse_qs(urlparse(route.request.url).query)
        timeline_queries.append(query)
        cursor = query.get("cursor", [None])[0]
        if cursor == "older-200":
            messages = [_timeline_message(i) for i in range(151, 201)]
            next_cursor = "older-150"
        else:
            messages = [_timeline_message(i) for i in range(201, 251)]
            next_cursor = "older-200"
        await fulfill_json(
            route,
            {
                "messages": messages,
                "next_cursor": next_cursor,
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(f"**/api/webchat/v2/threads/{THREAD_ID}/timeline**", handle_timeline)

    try:
        await page.goto(f"{reborn_v2_server}/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")

        bubbles = page.locator(f"{SEL_V2['msg_user']}, {SEL_V2['msg_assistant']}")
        await expect(bubbles).to_have_count(50, timeout=15000)
        assert timeline_queries == [{"limit": ["50"]}]
        await expect(page.locator(SEL_V2["message_list_load_older"])).to_be_visible()
        await expect(page.get_by_text("DOM page message 250")).to_be_visible()
        await expect(page.get_by_text("DOM page message 150")).to_have_count(0)

        await page.locator(SEL_V2["message_list_load_older"]).click()

        await expect(bubbles).to_have_count(100, timeout=15000)
        assert timeline_queries == [
            {"limit": ["50"]},
            {"limit": ["50"], "cursor": ["older-200"]},
        ]
        await expect(page.get_by_text("DOM page message 151")).to_be_visible()
        await expect(page.get_by_text("DOM page message 250")).to_be_visible()
        await expect(page.get_by_text("DOM page message 101")).to_have_count(0)
    finally:
        await context.close()


async def test_reborn_response_projection_survives_full_history_page(
    reborn_v2_server, reborn_v2_browser
):
    """Port legacy near-DOM-cap response integrity to Reborn projection updates."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    timeline_queries: list[dict[str, list[str]]] = []

    await install_fake_v2_event_stream(page)

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
                        "thread_id": FULL_PAGE_THREAD_ID,
                        "title": "Legacy near-cap response port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        query = parse_qs(urlparse(route.request.url).query)
        timeline_queries.append(query)
        await fulfill_json(
            route,
            {
                "messages": [_timeline_message(i) for i in range(201, 251)],
                "next_cursor": "older-200",
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(
        f"**/api/webchat/v2/threads/{FULL_PAGE_THREAD_ID}/timeline**",
        handle_timeline,
    )

    try:
        await page.goto(
            f"{reborn_v2_server}/chat/{FULL_PAGE_THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}"
        )

        bubbles = page.locator(f"{SEL_V2['msg_user']}, {SEL_V2['msg_assistant']}")
        await expect(bubbles).to_have_count(50, timeout=15000)
        assert timeline_queries == [{"limit": ["50"]}]
        await expect(page.get_by_text("DOM page message 250")).to_be_visible()
        await expect(page.get_by_text("DOM page message 150")).to_have_count(0)

        await page.evaluate(
            """
            () => window.__emitV2Sse("projection_update", {
              state: {
                items: [
                  {
                    text: {
                      id: "near-cap-final",
                      body: "Final response near DOM page cap"
                    }
                  }
                ]
              }
            })
            """
        )

        await expect(bubbles).to_have_count(51, timeout=5000)
        await expect(page.get_by_text("Final response near DOM page cap")).to_be_visible()
        await expect(page.get_by_text("DOM page message 150")).to_have_count(0)

        await page.evaluate(
            """
            () => window.__emitV2Sse("projection_update", {
              state: {
                items: [
                  {
                    text: {
                      id: "near-cap-final",
                      body: "Final response near DOM page cap with the final sentence intact."
                    }
                  }
                ]
              }
            }, "cursor-near-cap-2")
            """
        )

        await expect(bubbles).to_have_count(51, timeout=5000)
        await expect(
            page.get_by_text("Final response near DOM page cap with the final sentence intact.")
        ).to_be_visible()
        await expect(page.get_by_text("Final response near DOM page cap")).to_have_count(1)
    finally:
        await context.close()


async def test_reborn_user_send_survives_full_history_page_without_loading_older(
    reborn_v2_server, reborn_v2_browser
):
    """Port legacy near-DOM-cap user-send preservation to Reborn history paging."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    timeline_queries: list[dict[str, list[str]]] = []
    sent_messages: list[dict] = []

    await install_fake_v2_event_stream(page)

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
                        "thread_id": FULL_PAGE_SEND_THREAD_ID,
                        "title": "Legacy full-page send port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        query = parse_qs(urlparse(route.request.url).query)
        timeline_queries.append(query)
        await fulfill_json(
            route,
            {
                "messages": [_timeline_message(i) for i in range(201, 251)],
                "next_cursor": "older-200",
            },
        )

    async def handle_send(route):
        body = json.loads(route.request.post_data or "{}")
        sent_messages.append(body)
        await fulfill_json(
            route,
            {
                "thread_id": FULL_PAGE_SEND_THREAD_ID,
                "run_id": "run-full-page-send",
                "status": "queued",
                "accepted_message_ref": "message:full-page-send-user",
            },
        )

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(
        f"**/api/webchat/v2/threads/{FULL_PAGE_SEND_THREAD_ID}/timeline**",
        handle_timeline,
    )
    await page.route(
        f"**/api/webchat/v2/threads/{FULL_PAGE_SEND_THREAD_ID}/messages",
        handle_send,
    )

    try:
        await page.goto(
            f"{reborn_v2_server}/chat/{FULL_PAGE_SEND_THREAD_ID}"
            f"?token={REBORN_V2_AUTH_TOKEN}"
        )

        bubbles = page.locator(f"{SEL_V2['msg_user']}, {SEL_V2['msg_assistant']}")
        await expect(bubbles).to_have_count(50, timeout=15000)
        assert timeline_queries == [{"limit": ["50"]}]
        await expect(page.get_by_text("DOM page message 201")).to_be_visible()
        await expect(page.get_by_text("DOM page message 150")).to_have_count(0)

        composer = page.locator(SEL_V2["chat_composer"])
        await composer.fill("new message while history page is full")
        await composer.press("Enter")

        await expect(bubbles).to_have_count(51, timeout=5000)
        await expect(page.locator(SEL_V2["msg_user"]).last).to_contain_text(
            "new message while history page is full",
            timeout=5000,
        )
        assert sent_messages and sent_messages[-1]["content"] == (
            "new message while history page is full"
        )
        assert timeline_queries == [{"limit": ["50"]}]
        await expect(page.get_by_text("DOM page message 150")).to_have_count(0)
    finally:
        await context.close()


async def test_reborn_sse_reconnect_timer_clears_when_tab_hidden(
    reborn_v2_server, reborn_v2_browser
):
    """Port legacy reconnect timer cleanup to Reborn's visibility pause path.

    The native-EventSource era scheduled its own 2s reconnect timer; the
    fetch-based `event-source-plus` transport schedules retries internally.
    The equivalent contract: a failed stream leaves a pending reconnect, and
    hiding the tab cancels it (no new stream opens while hidden).
    """
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    await install_fake_v2_event_stream(page)

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
                        "thread_id": RECONNECT_THREAD_ID,
                        "title": "Legacy reconnect timer port",
                        "created_at": "2026-06-26T00:00:00Z",
                        "updated_at": "2026-06-26T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        await fulfill_json(route, {"messages": [], "next_cursor": None})

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads?**", handle_threads)
    await page.route(
        f"**/api/webchat/v2/threads/{RECONNECT_THREAD_ID}/timeline**",
        handle_timeline,
    )

    try:
        await page.goto(
            f"{reborn_v2_server}/chat/{RECONNECT_THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}"
        )
        await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
        await page.wait_for_function("() => window.__v2SseHasOpenStream?.() === true")

        # Fail the stream: the transport schedules a retry, which the fake
        # holds (no response yet). This is the pending-reconnect analog of the
        # legacy 2s reconnect timer.
        await page.evaluate("() => window.__failLatestV2Sse(0)")
        await page.wait_for_function("() => window.__v2SseHasHeldConnection?.() === true")

        await page.evaluate(
            """
            () => {
              Object.defineProperty(document, 'visibilityState', {
                configurable: true,
                get: () => 'hidden',
              });
              document.dispatchEvent(new Event('visibilitychange'));
            }
            """
        )

        # Hiding the tab aborts the pending reconnect: the held connection is
        # released and no new stream opens while hidden (the held retry already
        # recorded one URL; the count must not grow past it).
        await page.wait_for_function("() => window.__v2SseHasHeldConnection?.() === false")
        await page.wait_for_timeout(300)
        assert await page.evaluate("() => window.__v2SseUrls.length") == 2
    finally:
        await context.close()
