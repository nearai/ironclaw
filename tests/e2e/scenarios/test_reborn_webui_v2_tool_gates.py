"""Served Reborn WebUI v2 smoke coverage for tools and run gates.

The canonical smoke module proves text-only turns. These tests exercise the
remaining live ``ironclaw serve`` path through the deterministic mock model:
tool dispatch, cancellation, approval resolution, and manual-token auth
resolution. No route or SSE frame is stubbed.

Tracks nearai/ironclaw#4633.
"""

import asyncio
import json
import uuid
from urllib.parse import quote

import httpx

from helpers import REBORN_V2_AUTH_TOKEN, sse_stream, wait_for_sse_line
from reborn_webui_harness import (
    client_action_id,
    create_thread,
    fetch_timeline,
    reborn_bearer_headers,
    reborn_v2_server,  # noqa: F401 - imported fixture
    reborn_v2_yolo_server,  # noqa: F401 - imported fixture
    send_message,
    wait_for_assistant_message,
)


async def _wait_for_sse_event(
    response,
    *event_types: str,
    timeout: float = 60.0,
    match=None,
) -> dict:
    """Return the first matching WebChat JSON payload from the canonical stream."""
    matched_payload = None

    def matches(line: str) -> bool:
        nonlocal matched_payload
        if not line.startswith("data:"):
            return False
        try:
            payload = json.loads(line.removeprefix("data:").strip())
        except json.JSONDecodeError:
            return False
        event_type = payload.get("type", "")
        if event_type not in event_types or (
            match is not None and not match(event_type, payload)
        ):
            return False
        matched_payload = payload
        return True

    await wait_for_sse_line(
        response,
        predicate=matches,
        timeout=timeout,
    )
    assert matched_payload is not None
    return matched_payload


async def _set_llm_delay(mock_llm_server: str, marker: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_faults",
            json={
                "faults": [
                    {
                        "match": marker,
                        "actions": [{"type": "delay", "seconds": 10.0}],
                    }
                ]
            },
            timeout=10,
        )
        response.raise_for_status()


async def _wait_for_mock_request(
    mock_llm_server: str,
    marker: str,
    *,
    timeout: float = 20.0,
) -> None:
    deadline = asyncio.get_running_loop().time() + timeout
    async with httpx.AsyncClient() as client:
        while asyncio.get_running_loop().time() < deadline:
            response = await client.get(
                f"{mock_llm_server}/__mock/chat_requests",
                timeout=10,
            )
            response.raise_for_status()
            for request in response.json().get("requests", []):
                if marker in json.dumps(request):
                    return
            await asyncio.sleep(0.25)
    raise AssertionError(f"Mock LLM never received request marker {marker!r}")


def _tool_result_references(timeline: dict) -> list[dict]:
    return [
        message
        for message in timeline.get("messages", [])
        if message.get("kind") == "tool_result_reference"
    ]


async def test_reborn_v2_tool_turn_records_result_and_final_reply(
    reborn_v2_yolo_server,
):
    marker = f"tool-turn-{uuid.uuid4().hex[:8]}"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            f"reborn builtin echo {marker}",
        )
        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=60,
        )
        timeline = await fetch_timeline(client, reborn_v2_yolo_server, thread_id)

    references = _tool_result_references(timeline)
    assert references, timeline
    assert any(reference.get("tool_result_ref") for reference in references), references
    assert assistant.get("status") == "finalized", assistant
    assistant_content = assistant.get("content")
    assert isinstance(assistant_content, str), assistant
    assert assistant_content.strip(), assistant


async def test_reborn_v2_cancel_in_flight_turn_ends_cancelled(
    reborn_v2_server,
    mock_llm_server,
):
    marker = f"cancel-in-flight-{uuid.uuid4().hex[:8]}"
    await _set_llm_delay(mock_llm_server, marker)

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_server)

        async with sse_stream(
            reborn_v2_server,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=REBORN_V2_AUTH_TOKEN,
            timeout=100,
        ) as stream:
            assert stream.status == 200
            submitted = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/messages",
                json={
                    "client_action_id": client_action_id(),
                    "content": f"{marker}: hold this response",
                },
                timeout=30,
            )
            assert submitted.status_code in (200, 202), submitted.text
            run_id = submitted.json()["run_id"]
            await _wait_for_mock_request(mock_llm_server, marker)

            cancelled = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
                f"/runs/{run_id}/cancel",
                json={
                    "client_action_id": client_action_id(),
                    "reason": "user_requested",
                },
                timeout=15,
            )
            assert cancelled.status_code == 200, cancelled.text
            assert cancelled.json()["run_id"] == run_id

            def is_cancelled(event_type: str, payload: dict) -> bool:
                if event_type == "cancelled":
                    response = payload.get("response") or {}
                    # Cancel responses serialize the TurnStatus enum variant,
                    # while projection run statuses use lowercase wire values.
                    return (
                        response.get("run_id") == run_id
                        and response.get("status") == "Cancelled"
                    )
                # RunStatus is externally tagged, so its run_id is nested here.
                for item in payload.get("state", {}).get("items", []):
                    status = item.get("run_status") or {}
                    if (
                        status.get("run_id") == run_id
                        and status.get("status") == "cancelled"
                    ):
                        return True
                return False

            await _wait_for_sse_event(
                stream,
                "cancelled",
                "projection_snapshot",
                "projection_update",
                timeout=45,
                match=is_cancelled,
            )


async def test_reborn_v2_approval_gate_resolves_and_resumes(
    reborn_v2_server,
):
    marker = f"approval-{uuid.uuid4().hex[:8]}"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        permission = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/settings/tools/builtin.echo",
            json={"state": "ask_each_time"},
            timeout=15,
        )
        assert permission.status_code == 200, permission.text
        thread_id = await create_thread(client, reborn_v2_server)

        async with sse_stream(
            reborn_v2_server,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=REBORN_V2_AUTH_TOKEN,
            timeout=90,
        ) as stream:
            assert stream.status == 200
            submitted = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/messages",
                json={
                    "client_action_id": client_action_id(),
                    "content": f"reborn builtin echo {marker}",
                },
                timeout=30,
            )
            assert submitted.status_code in (200, 202), submitted.text

            event = await _wait_for_sse_event(stream, "gate", timeout=60)
            prompt = event["prompt"]
            assert prompt["approval_context"]["tool_name"] == "builtin.echo"

            resolved = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
                f"/runs/{prompt['turn_run_id']}"
                f"/gates/{quote(prompt['gate_ref'], safe='')}/resolve",
                json={
                    "client_action_id": client_action_id(),
                    "resolution": "approved",
                    "always": False,
                },
                timeout=15,
            )
            assert resolved.status_code == 200, resolved.text
            assert resolved.json()["outcome"] == "resumed", resolved.text

        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_server,
            thread_id,
            timeout=60,
        )
        timeline = await fetch_timeline(client, reborn_v2_server, thread_id)

    assert assistant.get("status") == "finalized", assistant
    assert _tool_result_references(timeline), timeline


async def test_reborn_v2_manual_token_auth_gate_resolves_and_resumes(
    reborn_v2_yolo_server,
):
    raw_token = f"ghp_e2e_{uuid.uuid4().hex}"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)

        async with sse_stream(
            reborn_v2_yolo_server,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=REBORN_V2_AUTH_TOKEN,
            timeout=120,
        ) as stream:
            assert stream.status == 200
            submitted = await client.post(
                f"{reborn_v2_yolo_server}/api/webchat/v2/threads/{thread_id}/messages",
                json={
                    "client_action_id": client_action_id(),
                    "content": "reborn install github for auth gate",
                },
                timeout=30,
            )
            assert submitted.status_code in (200, 202), submitted.text

            event = await _wait_for_sse_event(stream, "auth_required", timeout=75)
            prompt = event["prompt"]
            assert prompt["provider"] == "github", prompt
            assert prompt["challenge_kind"] == "manual_token", prompt

            token_submit = await client.post(
                f"{reborn_v2_yolo_server}"
                "/api/reborn/product-auth/manual-token/submit",
                json={
                    "provider": "github",
                    "account_label": "Reborn E2E GitHub",
                    "token": raw_token,
                    "thread_id": thread_id,
                    "run_id": prompt["turn_run_id"],
                    "gate_ref": prompt["auth_request_ref"],
                },
                timeout=15,
            )
            assert token_submit.status_code == 200, token_submit.text
            token_body = token_submit.json()
            credential_ref = token_body.get("credential_ref")
            assert isinstance(credential_ref, str), token_body
            assert credential_ref.strip(), token_body
            assert token_body["continuation"]["type"] == "turn_gate_resume"
            assert raw_token not in token_submit.text

        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=75,
        )
        timeline = await fetch_timeline(client, reborn_v2_yolo_server, thread_id)

    assert assistant.get("status") == "finalized", assistant
    assert _tool_result_references(timeline), timeline
    assert raw_token not in json.dumps(timeline)
