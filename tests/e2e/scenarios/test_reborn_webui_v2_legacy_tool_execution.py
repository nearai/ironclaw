"""Legacy tool-execution API checks ported to standalone Reborn WebUI v2."""

import asyncio
import json
from urllib.parse import unquote, urlparse

import aiohttp
import httpx
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    USER_ID,
    capability_preview_payload as _preview_payload,
    create_thread,
    fetch_timeline,
    reborn_bearer_headers,
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_loop_limited_yolo_server,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
    reborn_v2_yolo_server,  # noqa: F401 - imported fixture
    send_message,
    wait_for_assistant_message,
    wait_for_capability_preview as _wait_for_capability_preview,
)


EMPTY_REPLY_THREAD_ID = "thread-legacy-empty-reply"
EMPTY_REPLY_RUN_ID = "22222222-3333-4444-5555-666666666666"


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


async def _wait_for_request_count(
    requests: list,
    count: int,
    *,
    timeout: float = 5.0,
) -> None:
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        if len(requests) > count:
            return
        await asyncio.sleep(0.05)
    raise AssertionError(
        f"Timed out waiting for request count > {count}; got {len(requests)}"
    )


async def _wait_for_failed_run_projection(
    response, *, timeout: float = 45.0
) -> tuple[dict, int]:
    current_event = None
    data_lines: list[str] = []
    seen_frames: list[dict] = []
    completed_loop_echo_invocations: set[str] = set()
    deadline = asyncio.get_running_loop().time() + timeout

    async def next_line() -> str:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise asyncio.TimeoutError
        raw = await asyncio.wait_for(response.content.readline(), timeout=remaining)
        if raw == b"":
            raise AssertionError(
                f"SSE stream closed before failed-run projection. Seen frames: {seen_frames}"
            )
        return raw.decode("utf-8", errors="replace").rstrip("\r\n")

    while asyncio.get_running_loop().time() < deadline:
        try:
            line = await next_line()
        except asyncio.TimeoutError:
            break
        if not line:
            if current_event and data_lines:
                frame = json.loads("\n".join(data_lines))
                seen_frames.append({"event": current_event, "frame": frame})
                if current_event == "capability_activity":
                    activity = frame.get("activity") or {}
                    if (
                        activity.get("capability_id") == "builtin.echo"
                        and activity.get("status") == "completed"
                    ):
                        if invocation_id := activity.get("invocation_id"):
                            completed_loop_echo_invocations.add(invocation_id)
                if current_event in ("projection_snapshot", "projection_update"):
                    for item in frame.get("state", {}).get("items", []):
                        activity = item.get("capability_activity") or {}
                        if (
                            activity.get("capability_id") == "builtin.echo"
                            and activity.get("status") == "completed"
                        ):
                            if invocation_id := activity.get("invocation_id"):
                                completed_loop_echo_invocations.add(invocation_id)
                        run_status = item.get("run_status")
                        if not run_status:
                            continue
                        if run_status.get("status") == "failed":
                            return run_status, len(completed_loop_echo_invocations)
                if current_event == "failed":
                    return frame, len(completed_loop_echo_invocations)
            current_event = None
            data_lines = []
            continue
        if line.startswith(":"):
            continue
        if line.startswith("event:"):
            current_event = line.removeprefix("event:").strip()
            continue
        if line.startswith("data:"):
            data_lines.append(line.removeprefix("data:").strip())

    raise AssertionError(
        f"Timed out waiting for failed-run projection. Seen frames: {seen_frames}"
    )


def _empty_reply_user_record() -> dict:
    return {
        "message_id": "legacy-empty-reply-user",
        "kind": "user",
        "content": "issue 1780 empty reply",
        "sequence": 1,
        "status": "accepted",
        "created_at": "2026-06-25T12:00:00Z",
        "turn_run_id": EMPTY_REPLY_RUN_ID,
    }


async def _open_mocked_empty_reply_page(reborn_v2_server, reborn_v2_browser):
    context = await reborn_v2_browser.new_context(
        viewport={"width": 1280, "height": 720}
    )
    page = await context.new_page()
    await _install_fake_event_source(page)
    timeline_requests: list[dict] = []

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
        await fulfill_json(
            route,
            {
                "threads": [
                    {
                        "thread_id": EMPTY_REPLY_THREAD_ID,
                        "title": "Empty reply recovery",
                        "created_at": "2026-06-25T00:00:00Z",
                        "updated_at": "2026-06-25T00:00:00Z",
                    }
                ],
                "next_cursor": None,
            },
        )

    async def handle_timeline(route):
        parsed = urlparse(route.request.url)
        timeline_requests.append(dict(parsed._asdict()))
        thread_id = unquote(
            parsed.path.split("/threads/", 1)[1].split("/timeline", 1)[0]
        )
        messages = (
            [_empty_reply_user_record()]
            if thread_id == EMPTY_REPLY_THREAD_ID
            else []
        )
        await fulfill_json(route, {"messages": messages, "next_cursor": None})

    await page.route("**/api/webchat/v2/session", handle_session)
    await page.route("**/api/webchat/v2/threads", handle_threads)
    await page.route("**/api/webchat/v2/threads/*/timeline**", handle_timeline)

    await page.goto(
        f"{reborn_v2_server}/chat/{EMPTY_REPLY_THREAD_ID}?token={REBORN_V2_AUTH_TOKEN}"
    )
    await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
    await expect(page.locator(SEL_V2["msg_user"]).last).to_contain_text(
        "issue 1780 empty reply", timeout=5000
    )

    return {
        "context": context,
        "page": page,
        "timeline_requests": timeline_requests,
    }


async def test_reborn_legacy_builtin_echo_tool_executes(reborn_v2_yolo_server):
    """Port of legacy builtin echo execution to Reborn's namespaced capability."""
    marker = "reborn tool execution echo 1429"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            f"reborn builtin echo {marker}",
        )

        preview = await _wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.echo",
            output_fragment=marker,
        )

    assert preview["status"] == "completed", preview
    assert marker in (preview.get("output_preview") or ""), preview


async def test_reborn_legacy_builtin_time_tool_executes(reborn_v2_yolo_server):
    """Port of legacy builtin time execution to Reborn's namespaced capability."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "reborn builtin time",
        )

        preview = await _wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.time",
            output_fragment="utc_iso",
        )

    assert preview["status"] == "completed", preview
    assert "utc_iso" in (preview.get("output_preview") or ""), preview


async def test_reborn_legacy_non_tool_message_still_works(reborn_v2_yolo_server):
    """Port of legacy non-tool chat regression to Reborn's v2 timeline."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(client, reborn_v2_yolo_server, thread_id, "What is 2+2?")

        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=30,
        )
        timeline = await fetch_timeline(client, reborn_v2_yolo_server, thread_id)

    assert "4" in (assistant.get("content") or ""), assistant
    assert [
        message
        for message in timeline.get("messages", [])
        if message.get("kind") == "capability_display_preview"
    ] == []


async def test_reborn_legacy_parallel_tool_calls_complete(reborn_v2_yolo_server):
    """Port legacy parallel echo+time dispatch to Reborn capability previews."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "reborn parallel echo and time",
        )

        echo_preview = await _wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.echo",
            output_fragment="parallel-test",
        )
        time_preview = await _wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.time",
            output_fragment="utc_iso",
        )
        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=45,
        )

    content = assistant.get("content") or ""
    assert echo_preview["status"] == "completed", echo_preview
    assert time_preview["status"] == "completed", time_preview
    assert "Dispatched 2 tools" in content, assistant
    assert "parallel-test" in content, assistant


async def test_reborn_legacy_multi_step_tool_chain_completes(reborn_v2_yolo_server):
    """Port legacy echo-then-time chain to Reborn's planned loop path."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "multi step echo then time",
        )

        echo_preview = await _wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.echo",
            output_fragment="step-one",
        )
        time_preview = await _wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.time",
            output_fragment="utc_iso",
        )
        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=60,
        )
        timeline = await fetch_timeline(client, reborn_v2_yolo_server, thread_id)

    previews = [
        preview
        for message in timeline.get("messages", [])
        if (preview := _preview_payload(message)) is not None
        and preview.get("capability_id") in {"builtin.echo", "builtin.time"}
    ]
    assert echo_preview["status"] == "completed", echo_preview
    assert time_preview["status"] == "completed", time_preview
    assert [preview["capability_id"] for preview in previews] == [
        "builtin.echo",
        "builtin.time",
    ], previews
    assert "multi-step complete" in (assistant.get("content") or "").lower(), assistant


async def test_reborn_legacy_tool_failure_recovers_with_final_response(
    reborn_v2_yolo_server,
):
    """Port issue-1780 failed-tool recovery to Reborn's v2 turn path."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "issue 1780 tool failure",
        )

        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=45,
        )
        content = (assistant.get("content") or "").lower()

    assert "tool returned" in content, assistant
    assert "broken-operation" in content or "error" in content, assistant


async def test_reborn_legacy_truncated_tool_call_recovers_without_activity_card(
    reborn_v2_yolo_server,
):
    """Port issue-1780 truncated tool-call recovery to Reborn's v2 turn path."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "issue 1780 truncated tool call",
        )

        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=45,
        )
        timeline = await fetch_timeline(client, reborn_v2_yolo_server, thread_id)

    assert (
        "response was truncated" in (assistant.get("content") or "").lower()
    ), assistant
    assert [
        message
        for message in timeline.get("messages", [])
        if message.get("kind") == "capability_display_preview"
    ] == []


async def test_reborn_legacy_empty_reply_failure_projection_is_visible(
    reborn_v2_server, reborn_v2_browser
):
    """Port legacy empty-reply recovery to Reborn's visible failed-run bubble."""
    harness = await _open_mocked_empty_reply_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        before_terminal_requests = len(harness["timeline_requests"])

        await page.evaluate(
            f"""
            () => window.__emitV2Sse("projection_update", {{
              state: {{
                items: [
                  {{
                    run_status: {{
                      run_id: {EMPTY_REPLY_RUN_ID!r},
                      status: "failed",
                      failure_category: "invalid_output",
                      failure_summary: "The run failed because the model returned an empty assistant response."
                    }}
                  }}
                ]
              }}
            }})
            """
        )

        error_message = page.locator(SEL_V2["msg_error"]).filter(
            has_text="empty assistant response"
        )
        await expect(error_message).to_be_visible(timeout=5000)
        await _wait_for_request_count(
            harness["timeline_requests"], before_terminal_requests
        )
        await expect(error_message).to_be_visible(timeout=5000)
        await expect(page.locator(SEL_V2["chat_composer"])).to_be_enabled()
        await expect(
            page.locator(SEL_V2["msg_assistant"]).filter(has_text="empty assistant response")
        ).to_have_count(0)
    finally:
        await harness["context"].close()


async def test_reborn_legacy_looping_tool_calls_stop_at_low_iteration_boundary(
    reborn_v2_loop_limited_yolo_server,
):
    """Port issue-1780 looping tool-call termination to Reborn's v2 turn path."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_loop_limited_yolo_server)

        timeout = aiohttp.ClientTimeout(total=60, sock_read=60)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            events_url = (
                f"{reborn_v2_loop_limited_yolo_server}"
                f"/api/webchat/v2/threads/{thread_id}/events"
            )
            async with session.get(
                events_url,
                params={"token": REBORN_V2_AUTH_TOKEN},
                headers={"Accept": "text/event-stream"},
            ) as response:
                assert response.status == 200, await response.text()
                await send_message(
                    client,
                    reborn_v2_loop_limited_yolo_server,
                    thread_id,
                    "issue 1780 loop forever",
                )
                run_status, completed_loop_echoes = (
                    await _wait_for_failed_run_projection(response)
                )

    assert run_status.get("failure_category") == "iteration_limit", run_status
    assert completed_loop_echoes == 1
