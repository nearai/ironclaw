"""Served Reborn WebUI v2 streaming and run-control API tests.

These scenarios convert REBCLI-044 rows from Rust handler/support-substrate
contract proxies to caller-facing coverage through a real
`ironclaw-reborn serve` process. Browser approval-card UX remains covered by
the browser suites; this file focuses on served SSE and control routes.
"""

import asyncio
import json

import aiohttp
import httpx
import pytest

from helpers import REBORN_V2_AUTH_TOKEN, sse_stream, wait_for_sse_line
from reborn_webui_harness import (
    client_action_id,
    create_thread,
    fetch_timeline,
    reborn_bearer_headers,
)

pytest_plugins = ["reborn_webui_harness"]


async def _next_sse_event(response, *, timeout: float = 45) -> dict:
    """Read one complete SSE event block from a served response."""
    event_id = None
    event_name = None
    data_lines = []
    async with asyncio.timeout(timeout):
        while True:
            raw_line = await response.content.readline()
            if not raw_line:
                raise AssertionError("SSE stream closed before an event arrived")
            line = raw_line.decode("utf-8", errors="replace").rstrip("\r\n")
            if not line:
                if data_lines:
                    return {
                        "id": event_id,
                        "event": event_name,
                        "data": json.loads("\n".join(data_lines)),
                    }
                continue
            if line.startswith(":"):
                continue
            field, separator, value = line.partition(":")
            if not separator:
                value = ""
            elif value.startswith(" "):
                value = value[1:]
            if field == "id":
                event_id = value
            elif field == "event":
                event_name = value
            elif field == "data":
                data_lines.append(value)


def _contains_field(value, field: str, expected: str) -> bool:
    if isinstance(value, dict):
        if value.get(field) == expected:
            return True
        return any(_contains_field(item, field, expected) for item in value.values())
    if isinstance(value, list):
        return any(_contains_field(item, field, expected) for item in value)
    return False


def _event_matches_run_status(event: dict, run_id: str, status: str) -> bool:
    data = event["data"]
    return _contains_field(data, "run_id", run_id) and _contains_field(
        data, "status", status
    )


def _sse_payload_signature(event: dict) -> str:
    """Compare logical frames while allowing a replay cursor to be re-based."""
    payload = dict(event["data"])
    payload.pop("cursor", None)
    if payload.get("type") in {"projection_snapshot", "projection_update"}:
        payload["type"] = "projection_state"
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


async def _collect_sse_until_run_status(
    response,
    run_id: str,
    status: str,
    *,
    timeout: float = 60,
) -> list[dict]:
    events = []
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    while True:
        remaining = deadline - loop.time()
        if remaining <= 0:
            break
        try:
            event = await _next_sse_event(response, timeout=remaining)
        except TimeoutError:
            break
        events.append(event)
        if _event_matches_run_status(event, run_id, status):
            return events
    recent = [
        {
            "event": event["event"],
            "type": event["data"].get("type"),
            "has_cursor": event["id"] is not None,
        }
        for event in events[-3:]
    ]
    raise AssertionError(
        f"Timed out waiting for run {run_id} to reach {status}; "
        f"observed={len(events)}, recent={recent}"
    )


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
    reborn_v2_server: str,
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

    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        async with sse_stream(
            reborn_v2_server,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=REBORN_V2_AUTH_TOKEN,
            timeout=65,
        ) as events:
            assert events.status == 200
            submitted = await _submit_message(
                client,
                reborn_v2_server,
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
                reborn_v2_server,
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


async def test_reborn_v2_sse_stream_accepts_bearer_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_server)

    async with sse_stream(
        reborn_v2_server,
        path=f"/api/webchat/v2/threads/{thread_id}/events",
        token=REBORN_V2_AUTH_TOKEN,
        timeout=45,
    ) as bearer_response:
        assert bearer_response.status == 200

        async with httpx.AsyncClient(headers=headers) as client:
            submitted = await _submit_message(client, reborn_v2_server, thread_id)

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


async def test_reborn_v2_sse_auth_scope_and_capacity_served(reborn_v2_server):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_server)

    client_timeout = aiohttp.ClientTimeout(total=10, sock_read=10)
    async with aiohttp.ClientSession(timeout=client_timeout) as session:
        events_url = f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/events"

        anonymous = await session.get(events_url, headers={"Accept": "text/event-stream"})
        try:
            assert anonymous.status == 401
        finally:
            anonymous.close()

        timeline_with_query_token = await session.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline"
            f"?token={REBORN_V2_AUTH_TOKEN}",
        )
        try:
            assert timeline_with_query_token.status == 401
        finally:
            timeline_with_query_token.close()

        streams = []
        try:
            for _ in range(3):
                response = await session.get(
                    f"{events_url}?token={REBORN_V2_AUTH_TOKEN}",
                    headers={"Accept": "text/event-stream"},
                )
                assert response.status == 200
                streams.append(response)

            exhausted = await session.get(
                f"{events_url}?token={REBORN_V2_AUTH_TOKEN}",
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


async def test_reborn_v2_sse_reconnect_resumes_without_gap_or_duplicate_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        events_path = f"/api/webchat/v2/threads/{thread_id}/events"

        async with sse_stream(
            reborn_v2_server,
            path=events_path,
            token=REBORN_V2_AUTH_TOKEN,
            timeout=65,
        ) as initial_stream:
            assert initial_stream.status == 200
            submitted = await _submit_message(
                client,
                reborn_v2_server,
                thread_id,
                "served Last-Event-ID replay: what is 2+2?",
            )
            initial_events = await _collect_sse_until_run_status(
                initial_stream,
                submitted["run_id"],
                "completed",
            )

        initial_cursor_events = [event for event in initial_events if event["id"]]
        initial_ids = [event["id"] for event in initial_cursor_events]
        assert len(initial_ids) >= 2, initial_events
        assert len(initial_ids) == len(set(initial_ids)), initial_ids
        replay_from = initial_ids[0]
        expected_terminal_payload = _sse_payload_signature(initial_events[-1])

        async with sse_stream(
            reborn_v2_server,
            path=events_path,
            token=REBORN_V2_AUTH_TOKEN,
            headers={"Last-Event-ID": replay_from},
            timeout=45,
        ) as resumed_stream:
            assert resumed_stream.status == 200
            replayed_events = await _collect_sse_until_run_status(
                resumed_stream,
                submitted["run_id"],
                "completed",
                timeout=40,
            )

        replayed_cursor_events = [event for event in replayed_events if event["id"]]
        replayed_ids = [event["id"] for event in replayed_cursor_events]
        assert len(replayed_ids) == len(set(replayed_ids)), replayed_ids
        assert replay_from not in replayed_ids
        # Replay may compact live updates into a snapshot. The logical terminal
        # projection must still be identical even when the frame kind changes.
        assert _sse_payload_signature(replayed_events[-1]) == expected_terminal_payload


async def test_reborn_v2_websocket_origin_projection_and_shared_capacity_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_server)

    ws_base = reborn_v2_server.replace("http://", "ws://", 1).replace(
        "https://", "wss://", 1
    )
    ws_url = f"{ws_base}/api/webchat/v2/threads/{thread_id}/ws"
    events_url = f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/events"
    timeout = aiohttp.ClientTimeout(total=60, sock_read=45)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        with pytest.raises(aiohttp.WSServerHandshakeError) as rejected:
            await session.ws_connect(
                ws_url,
                headers=headers,
                origin="https://attacker.invalid",
            )
        assert rejected.value.status == 403

        websocket = None
        streams = []
        try:
            websocket = await session.ws_connect(
                ws_url,
                headers=headers,
                origin=reborn_v2_server,
            )
            for _ in range(2):
                response = await session.get(
                    events_url,
                    headers={
                        **headers,
                        "Accept": "text/event-stream",
                    },
                )
                assert response.status == 200
                streams.append(response)

            exhausted = await session.get(
                events_url,
                headers={
                    **headers,
                    "Accept": "text/event-stream",
                },
            )
            try:
                assert exhausted.status == 429
                exhausted_body = await exhausted.json()
                assert exhausted_body["error"] == "rate_limited"
                assert exhausted_body["retryable"] is True
            finally:
                exhausted.close()

            async with httpx.AsyncClient(headers=headers) as client:
                submitted = await _submit_message(
                    client,
                    reborn_v2_server,
                    thread_id,
                    "served WebSocket projection: what is 2+2?",
                )

            async with asyncio.timeout(45):
                while True:
                    message = await websocket.receive()
                    assert message.type == aiohttp.WSMsgType.TEXT, message
                    frame = json.loads(message.data)
                    if _contains_field(frame, "run_id", submitted["run_id"]):
                        assert frame.get("projection_cursor"), frame
                        break
        finally:
            for stream in streams:
                stream.close()
            if websocket is not None:
                await websocket.close()


async def test_reborn_v2_cancel_and_gate_control_routes_served(reborn_v2_server):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        submitted = await _submit_message(client, reborn_v2_server, thread_id)
        run_id = submitted["run_id"]

        cancel = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel",
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
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}"
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
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel",
            json={"client_action_id": client_action_id()},
            timeout=15,
        )
        assert unauthenticated_cancel.status_code == 401

        unauthenticated_gate = await anonymous.post(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}"
            "/gates/not-a-gate/resolve",
            json={"client_action_id": client_action_id(), "resolution": "approved"},
            timeout=15,
        )
        assert unauthenticated_gate.status_code == 401


async def test_reborn_v2_retries_mock_llm_http_error_then_finalizes(
    reborn_v2_server,
    mock_llm_server,
):
    marker = "mock llm http retry e2e"
    await _run_fault_scenario(
        reborn_v2_server,
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


async def test_reborn_v2_retries_mock_llm_broken_sse_stream_then_finalizes(
    reborn_v2_server,
    mock_llm_server,
):
    marker = "mock llm broken sse retry e2e"
    await _run_fault_scenario(
        reborn_v2_server,
        mock_llm_server,
        marker=marker,
        actions=[{"type": "broken_stream_before_text"}],
        expected_request_count=2,
    )


async def test_reborn_v2_delayed_mock_llm_response_finalizes(
    reborn_v2_server,
    mock_llm_server,
):
    marker = "mock llm delayed response e2e"
    await _run_fault_scenario(
        reborn_v2_server,
        mock_llm_server,
        marker=marker,
        actions=[{"type": "delay", "seconds": 1.25}],
        expected_request_count=1,
    )


async def test_reborn_v2_cancel_delayed_mock_llm_inference_releases_thread(
    reborn_v2_server,
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

    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        submitted = await _submit_message(
            client,
            reborn_v2_server,
            thread_id,
            f"{marker}: hold this inference open",
        )
        run_id = submitted["run_id"]

        await _wait_for_mock_llm_request_count(mock_llm_server, marker, 1, timeout=15)

        cancel = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel",
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
                reborn_v2_server,
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
            reborn_v2_server,
            thread_id,
            "4",
            timeout=60,
        )

    assert assistant["content"] == "The answer is 4."


async def test_reborn_v2_protocol_error_and_limit_boundaries_served(
    reborn_v2_server,
):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        malformed_thread = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/"
            "__ironclaw_reserved/timeline",
            timeout=15,
        )
        assert malformed_thread.status_code == 400
        assert malformed_thread.json() == {
            "error": "invalid_request",
            "kind": "validation",
            "retryable": False,
            "field": "thread_id",
            "validation_code": "invalid_id",
        }

        missing_action_id = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/threads",
            json={},
            timeout=15,
        )
        assert missing_action_id.status_code == 400
        missing_body = missing_action_id.json()
        assert missing_body["error"] == "invalid_request"
        assert missing_body["field"] == "client_action_id"
        assert missing_body["validation_code"] == "missing_field"

        oversized_body = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/threads",
            json={
                "client_action_id": client_action_id(),
                "padding": "x" * (16 * 1024),
            },
            timeout=15,
        )
        assert oversized_body.status_code == 413
        assert oversized_body.text == "Request body exceeds the route's body limit."

        retry_url = (
            f"{reborn_v2_server}/api/webchat/v2/threads/thread-rate-limit/"
            "runs/00000000-0000-0000-0000-000000000000/retry"
        )
        for _ in range(60):
            accepted_by_limiter = await client.post(
                retry_url,
                json={"client_action_id": client_action_id()},
                timeout=15,
            )
            assert accepted_by_limiter.status_code != 429
            await asyncio.sleep(0.05)

        rate_limited = await client.post(
            retry_url,
            json={"client_action_id": client_action_id()},
            timeout=15,
        )
        assert rate_limited.status_code == 429
        assert rate_limited.text == "Rate limit exceeded. Try again shortly."
