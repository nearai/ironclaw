"""Served Reborn WebUI v2 streaming and run-control API tests.

These scenarios convert REBCLI-044 rows from Rust handler/support-substrate
contract proxies to caller-facing coverage through a real
`ironclaw-reborn serve` process. Browser approval-card UX remains covered by
the browser suites; this file focuses on served SSE and control routes.
"""

import json

import aiohttp
import httpx

from helpers import REBORN_V2_AUTH_TOKEN, sse_stream, wait_for_sse_line
from reborn_webui_harness import client_action_id, create_thread, reborn_bearer_headers

pytest_plugins = ["reborn_webui_harness"]


async def _submit_message(client: httpx.AsyncClient, base_url: str, thread_id: str) -> dict:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/messages",
        json={"client_action_id": client_action_id(), "content": "hello streaming"},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text
    return response.json()


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
