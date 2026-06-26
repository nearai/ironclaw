"""Legacy DOM resource-limit coverage ported to Reborn WebUI v2 history paging."""

import json
from urllib.parse import parse_qs, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


THREAD_ID = "thread-legacy-dom-resource-limits"


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

    await page.add_init_script(
        """
        (() => {
          class FakeEventSource extends EventTarget {
            constructor(url) {
              super();
              this.url = url;
              this.readyState = 0;
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
        await page.goto(f"{reborn_v2_server}/v2/chat/{THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}")

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
