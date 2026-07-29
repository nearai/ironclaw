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


def _assert_text_redacted(secret: str, value: str, *, source: str) -> None:
    if secret in value:
        raise AssertionError(f"{source} exposed the raw credential")


async def _assert_sse_redacted_until(
    response,
    secret: str,
    outcome_reached: asyncio.Event,
) -> None:
    """Inspect every frame until the run is terminal and the stream goes quiet."""
    while True:
        try:
            raw = await asyncio.wait_for(response.content.readline(), timeout=0.25)
        except asyncio.TimeoutError:
            if outcome_reached.is_set():
                return
            continue
        if not raw:
            if outcome_reached.is_set():
                return
            raise AssertionError("SSE stream closed before the run reached a terminal outcome")
        line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
        _assert_text_redacted(secret, line, source="post-submit SSE frame")


async def _fetch_run_artifact(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    run_id: str,
) -> dict:
    response = await client.get(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/runs/{run_id}/artifact",
        timeout=15,
    )
    assert response.status_code == 200, response.text
    return response.json()


async def _wait_for_run_artifact_status(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    run_id: str,
    expected_status: str,
    *,
    timeout: float = 60.0,
) -> dict:
    deadline = asyncio.get_running_loop().time() + timeout
    last_artifact = None
    while asyncio.get_running_loop().time() < deadline:
        last_artifact = await _fetch_run_artifact(
            client,
            base_url,
            thread_id,
            run_id,
        )
        if last_artifact.get("run", {}).get("status") == expected_status:
            return last_artifact
        await asyncio.sleep(0.25)
    raise AssertionError(
        f"Run artifact did not reach {expected_status}; last={last_artifact}"
    )


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


async def test_reborn_v2_approval_gate_decline_has_no_successful_tool_result(
    reborn_v2_server,
):
    marker = f"approval-decline-{uuid.uuid4().hex[:8]}"
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
            run_id = submitted.json()["run_id"]

            gate_event = await _wait_for_sse_event(
                stream,
                "gate",
                timeout=60,
                match=lambda _event_type, payload: (
                    payload.get("prompt", {}).get("turn_run_id") == run_id
                ),
            )
            prompt = gate_event["prompt"]
            assert prompt["approval_context"]["tool_name"] == "builtin.echo"

            resolved = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
                f"/runs/{run_id}/gates/{quote(prompt['gate_ref'], safe='')}/resolve",
                json={
                    "client_action_id": client_action_id(),
                    "resolution": "declined",
                },
                timeout=15,
            )
            assert resolved.status_code == 200, resolved.text
            assert resolved.json()["outcome"] == "resumed", resolved.text

        artifact = await _wait_for_run_artifact_status(
            client,
            reborn_v2_server,
            thread_id,
            run_id,
            "Completed",
        )
        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_server,
            thread_id,
            timeout=60,
        )
        timeline = await fetch_timeline(client, reborn_v2_server, thread_id)

    assistant_content = assistant.get("content")
    assert isinstance(assistant_content, str), assistant
    assert "declined by user" in assistant_content.lower(), assistant
    assert artifact["run"]["status"] == "Completed", artifact

    references = _tool_result_references(timeline)
    assert references, timeline
    for reference in references:
        envelope = json.loads(reference["content"])
        observation = envelope["model_observation"]
        assert observation["status"] == "error", envelope
        assert observation["detail"]["failure_kind"] == "gate_declined", envelope
        assert envelope["result_ref"].startswith("result:provider-error-"), envelope


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

            run_id = prompt["turn_run_id"]
            outcome_reached = asyncio.Event()
            sse_redaction = asyncio.create_task(
                _assert_sse_redacted_until(stream, raw_token, outcome_reached)
            )
            try:
                token_submit = await client.post(
                    f"{reborn_v2_yolo_server}"
                    "/api/reborn/product-auth/manual-token/submit",
                    json={
                        "provider": "github",
                        "account_label": "Reborn E2E GitHub",
                        "token": raw_token,
                        "thread_id": thread_id,
                        "run_id": run_id,
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
                _assert_text_redacted(
                    raw_token,
                    token_submit.text,
                    source="manual-token response",
                )

                artifact = await _wait_for_run_artifact_status(
                    client,
                    reborn_v2_yolo_server,
                    thread_id,
                    run_id,
                    "Completed",
                    timeout=75,
                )
                assistant = await wait_for_assistant_message(
                    client,
                    reborn_v2_yolo_server,
                    thread_id,
                    timeout=75,
                )
                timeline = await fetch_timeline(
                    client,
                    reborn_v2_yolo_server,
                    thread_id,
                )
            finally:
                outcome_reached.set()
                await sse_redaction

    assert assistant.get("status") == "finalized", assistant
    assert _tool_result_references(timeline), timeline
    assert isinstance(artifact.get("logs", {}).get("entries"), list), artifact
    _assert_text_redacted(raw_token, json.dumps(timeline), source="timeline")
    _assert_text_redacted(raw_token, json.dumps(artifact), source="run artifact")
