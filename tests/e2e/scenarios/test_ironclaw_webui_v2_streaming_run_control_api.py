"""Served IronClaw WebUI v2 streaming and run-control API tests.

These scenarios convert REBCLI-044 rows from Rust handler/support-substrate
contract proxies to caller-facing coverage through a real
`ironclaw serve` process. Browser approval-card UX remains covered by
the browser suites; this file focuses on served SSE and control routes.
"""

import asyncio
import json

import aiohttp
import httpx

from helpers import IRONCLAW_V2_AUTH_TOKEN, sse_stream, wait_for_sse_line
from ironclaw_webui_harness import (
    client_action_id,
    create_thread,
    fetch_timeline,
    ironclaw_bearer_headers,
)

pytest_plugins = ["ironclaw_webui_harness"]


async def _submit_message(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    content: str = "hello streaming",
) -> dict:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/messages",
        json={"client_action_id": client_action_id(), "content": content},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text
    return response.json()


async def _set_llm_faults(mock_llm_server: str, faults: list[dict]) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_faults",
            json={"faults": faults},
            timeout=10,
        )
        response.raise_for_status()


def _message_text(message: dict) -> str:
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for part in content:
            if isinstance(part, dict) and isinstance(part.get("text"), str):
                parts.append(part["text"])
        return " ".join(parts)
    return ""


def _request_has_user_marker(request: dict, marker: str) -> bool:
    for message in request.get("messages", []):
        if message.get("role") == "user" and marker.lower() in _message_text(message).lower():
            return True
    return False


async def _mock_llm_requests_matching(mock_llm_server: str, marker: str) -> list[dict]:
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{mock_llm_server}/__mock/chat_requests",
            timeout=10,
        )
        response.raise_for_status()
    return [
        request
        for request in response.json().get("requests", [])
        if _request_has_user_marker(request, marker)
    ]


async def _wait_for_mock_llm_request_count(
    mock_llm_server: str,
    marker: str,
    count: int,
    *,
    timeout: float = 30.0,
) -> list[dict]:
    deadline = asyncio.get_running_loop().time() + timeout
    last_requests: list[dict] = []
    while asyncio.get_running_loop().time() < deadline:
        last_requests = await _mock_llm_requests_matching(mock_llm_server, marker)
        if len(last_requests) >= count:
            return last_requests
        await asyncio.sleep(0.25)
    raise AssertionError(
        f"Timed out waiting for {count} mock LLM request(s) matching {marker!r}; "
        f"observed {len(last_requests)}"
    )


async def _wait_for_assistant_content(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    needle: str,
    *,
    timeout: float = 45.0,
) -> dict:
    deadline = asyncio.get_running_loop().time() + timeout
    last_timeline: dict = {}
    while asyncio.get_running_loop().time() < deadline:
        last_timeline = await fetch_timeline(client, base_url, thread_id)
        for message in last_timeline.get("messages", []):
            if (
                message.get("kind") == "assistant"
                and message.get("status") == "finalized"
                and needle.lower() in (message.get("content") or "").lower()
            ):
                return message
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for assistant content containing {needle!r}. "
        f"Last timeline: {last_timeline}"
    )


async def _wait_for_run_completed_sse_event(
    response,
    run_id: str,
    *,
    timeout: float = 60.0,
) -> dict:
    line = await wait_for_sse_line(
        response,
        predicate=lambda value: value.startswith("data:")
        and run_id in value
        and '"status":"completed"' in value,
        timeout=timeout,
    )
    event = json.loads(line.removeprefix("data:").strip())
    assert event.get("cursor"), event
    return event


async def _run_fault_scenario(
    ironclaw_v2_server: str,
    mock_llm_server: str,
    *,
    marker: str,
    actions: list[dict],
    expected_request_count: int,
) -> None:
    await _set_llm_faults(
        mock_llm_server,
        [{"match": marker, "actions": actions}],
    )

    headers = ironclaw_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, ironclaw_v2_server)
        async with sse_stream(
            ironclaw_v2_server,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=IRONCLAW_V2_AUTH_TOKEN,
            timeout=65,
        ) as events:
            assert events.status == 200
            submitted = await _submit_message(
                client,
                ironclaw_v2_server,
                thread_id,
                f"{marker}: what is 2+2?",
            )
            sse_event = await _wait_for_run_completed_sse_event(
                events,
                submitted["run_id"],
                timeout=60,
            )
            assistant = await _wait_for_assistant_content(
                client,
                ironclaw_v2_server,
                thread_id,
                "4",
                timeout=60,
            )

    requests = await _wait_for_mock_llm_request_count(
        mock_llm_server,
        marker,
        expected_request_count,
    )
    assert submitted["run_id"] in json.dumps(sse_event)
    assert assistant["content"] == "The answer is 4."
    assert all(
        request.get("stream") is True for request in requests[:expected_request_count]
    )


async def test_ironclaw_v2_sse_stream_accepts_bearer_served(
    ironclaw_v2_server,
):
    headers = ironclaw_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, ironclaw_v2_server)

    async with sse_stream(
        ironclaw_v2_server,
        path=f"/api/webchat/v2/threads/{thread_id}/events",
        token=IRONCLAW_V2_AUTH_TOKEN,
        timeout=45,
    ) as bearer_response:
        assert bearer_response.status == 200

        async with httpx.AsyncClient(headers=headers) as client:
            submitted = await _submit_message(client, ironclaw_v2_server, thread_id)

        line = await wait_for_sse_line(
            bearer_response,
            predicate=lambda value: value.startswith("data:")
            and '"type":"keep_alive"' not in value,
            timeout=45,
        )
        event = json.loads(line.removeprefix("data:").strip())
        assert event.get("cursor"), event
        event_json = json.dumps(event)
        assert thread_id in event_json
        assert submitted["run_id"] in event_json


async def test_ironclaw_v2_sse_auth_scope_and_capacity_served(ironclaw_v2_server):
    headers = ironclaw_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, ironclaw_v2_server)

    client_timeout = aiohttp.ClientTimeout(total=10, sock_read=10)
    async with aiohttp.ClientSession(timeout=client_timeout) as session:
        events_url = f"{ironclaw_v2_server}/api/webchat/v2/threads/{thread_id}/events"

        anonymous = await session.get(events_url, headers={"Accept": "text/event-stream"})
        try:
            assert anonymous.status == 401
        finally:
            anonymous.close()

        timeline_with_query_token = await session.get(
            f"{ironclaw_v2_server}/api/webchat/v2/threads/{thread_id}/timeline"
            f"?token={IRONCLAW_V2_AUTH_TOKEN}",
        )
        try:
            assert timeline_with_query_token.status == 401
        finally:
            timeline_with_query_token.close()

        streams = []
        try:
            for _ in range(3):
                response = await session.get(
                    f"{events_url}?token={IRONCLAW_V2_AUTH_TOKEN}",
                    headers={"Accept": "text/event-stream"},
                )
                assert response.status == 200
                streams.append(response)

            exhausted = await session.get(
                f"{events_url}?token={IRONCLAW_V2_AUTH_TOKEN}",
                headers={"Accept": "text/event-stream"},
            )
            try:
                assert exhausted.status == 429
                body = await exhausted.json()
                assert body["error"] == "rate_limited"
                assert body["retryable"] is True
            finally:
                exhausted.close()
        finally:
            for stream in streams:
                stream.close()


async def test_ironclaw_v2_cancel_and_gate_control_routes_served(ironclaw_v2_server):
    headers = ironclaw_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, ironclaw_v2_server)
        submitted = await _submit_message(client, ironclaw_v2_server, thread_id)
        run_id = submitted["run_id"]

        cancel = await client.post(
            f"{ironclaw_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel",
            json={
                "client_action_id": client_action_id(),
                "thread_id": "body-thread-must-not-win",
                "run_id": "body-run-must-not-win",
                "reason": "qa served cancel",
            },
            timeout=15,
        )
        if cancel.status_code == 200:
            cancel_body = cancel.json()
            assert cancel_body["run_id"] == run_id
            assert "status" in cancel_body
        else:
            assert cancel.status_code == 400
            cancel_body = cancel.json()
            assert cancel_body["error"] == "invalid_request"
            assert cancel_body.get("retryable") is False

        missing_gate = await client.post(
            f"{ironclaw_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}"
            "/gates/not-a-gate/resolve",
            json={
                "client_action_id": client_action_id(),
                "thread_id": "body-thread-must-not-win",
                "run_id": "body-run-must-not-win",
                "gate_ref": "body-gate-must-not-win",
                "resolution": "approved",
            },
            timeout=15,
        )
        assert missing_gate.status_code in {400, 404, 409}

    async with httpx.AsyncClient() as anonymous:
        unauthenticated_cancel = await anonymous.post(
            f"{ironclaw_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel",
            json={"client_action_id": client_action_id()},
            timeout=15,
        )
        assert unauthenticated_cancel.status_code == 401

        unauthenticated_gate = await anonymous.post(
            f"{ironclaw_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}"
            "/gates/not-a-gate/resolve",
            json={"client_action_id": client_action_id(), "resolution": "approved"},
            timeout=15,
        )
        assert unauthenticated_gate.status_code == 401


async def test_ironclaw_v2_retries_mock_llm_http_error_then_finalizes(
    ironclaw_v2_server,
    mock_llm_server,
):
    marker = "mock llm http retry e2e"
    await _run_fault_scenario(
        ironclaw_v2_server,
        mock_llm_server,
        marker=marker,
        actions=[
            {
                "type": "http_error",
                "status": 502,
                "message": "scripted transient provider failure",
            }
        ],
        expected_request_count=2,
    )


async def test_ironclaw_v2_retries_mock_llm_broken_sse_stream_then_finalizes(
    ironclaw_v2_server,
    mock_llm_server,
):
    marker = "mock llm broken sse retry e2e"
    await _run_fault_scenario(
        ironclaw_v2_server,
        mock_llm_server,
        marker=marker,
        actions=[{"type": "broken_stream_before_text"}],
        expected_request_count=2,
    )


async def test_ironclaw_v2_delayed_mock_llm_response_finalizes(
    ironclaw_v2_server,
    mock_llm_server,
):
    marker = "mock llm delayed response e2e"
    await _run_fault_scenario(
        ironclaw_v2_server,
        mock_llm_server,
        marker=marker,
        actions=[{"type": "delay", "seconds": 1.25}],
        expected_request_count=1,
    )


async def test_ironclaw_v2_cancel_delayed_mock_llm_inference_releases_thread(
    ironclaw_v2_server,
    mock_llm_server,
):
    marker = "mock llm cancel delayed inference e2e"
    await _set_llm_faults(
        mock_llm_server,
        [
            {
                "match": marker,
                "actions": [{"type": "delay", "seconds": 10.0}],
            }
        ],
    )

    headers = ironclaw_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, ironclaw_v2_server)
        submitted = await _submit_message(
            client,
            ironclaw_v2_server,
            thread_id,
            f"{marker}: hold this inference open",
        )
        run_id = submitted["run_id"]

        await _wait_for_mock_llm_request_count(mock_llm_server, marker, 1, timeout=15)

        cancel = await client.post(
            f"{ironclaw_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel",
            json={
                "client_action_id": client_action_id(),
                "reason": "user_requested",
            },
            timeout=15,
        )
        assert cancel.status_code == 200, cancel.text
        cancel_body = cancel.json()
        assert cancel_body["run_id"] == run_id
        assert cancel_body["status"] in {"CancelRequested", "Cancelled"}

        for _ in range(60):
            follow_up = await _submit_message(
                client,
                ironclaw_v2_server,
                thread_id,
                "post cancel follow-up: what is 2+2?",
            )
            if follow_up.get("outcome") in {"submitted", "already_submitted"}:
                break
            assert follow_up.get("outcome") == "rejected_busy", follow_up
            await asyncio.sleep(0.5)
        else:
            raise AssertionError("Thread stayed busy after cancelling a delayed inference")

        assistant = await _wait_for_assistant_content(
            client,
            ironclaw_v2_server,
            thread_id,
            "4",
            timeout=60,
        )

    assert assistant["content"] == "The answer is 4."
