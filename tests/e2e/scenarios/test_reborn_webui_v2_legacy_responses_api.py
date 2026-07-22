"""Legacy Responses API coverage ported to standalone Reborn."""

import asyncio

import httpx
import pytest

from reborn_webui_harness import (
    close_reborn_server,
    reborn_bearer_headers,
    start_reborn_webui_v2_server,
)

@pytest.fixture(scope="module")
async def reborn_openai_compat_server(
    ironclaw_reborn_openai_compat_binary,
    mock_llm_server,
    tmp_path_factory,
):
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-openai-compat-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_openai_compat_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        log_prefix="reborn-openai-compat",
    )
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture()
async def reborn_responses_client(reborn_openai_compat_server):
    async with httpx.AsyncClient(
        base_url=reborn_openai_compat_server,
        headers={**reborn_bearer_headers(), "Content-Type": "application/json"},
        timeout=120,
    ) as client:
        yield client


def _response_output_text(response: dict) -> str:
    parts: list[str] = []
    for item in response.get("output") or []:
        content = item.get("content")
        if isinstance(content, list):
            for part in content:
                if isinstance(part, dict) and isinstance(part.get("text"), str):
                    parts.append(part["text"])
        elif isinstance(content, str):
            parts.append(content)
    return "\n".join(parts)


async def _create_response(client: httpx.AsyncClient, path="/v1/responses", **payload):
    response = None
    for attempt in range(6):
        response = await client.post(path, json={"model": "default", **payload})
        if response.status_code != 429 and not _is_retryable_service_unavailable(response):
            break
        await asyncio.sleep(1 + attempt * 0.5)
    assert response is not None
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["id"].startswith("resp_")
    assert body["object"] == "response"
    return body


def _is_retryable_service_unavailable(response: httpx.Response) -> bool:
    if response.status_code != 503:
        return False
    try:
        body = response.json()
    except ValueError:
        return False
    if not isinstance(body, dict):
        return False
    error = body.get("error")
    if isinstance(error, dict) and error.get("code") == "service_unavailable":
        return True
    return error == "service_unavailable" or body.get("kind") == "service_unavailable"


async def test_reborn_legacy_responses_non_streaming_text_input(
    reborn_responses_client,
):
    response = await _create_response(
        reborn_responses_client,
        input="Say hello in exactly 3 words",
    )

    assert response["status"] == "completed"
    assert response["model"] == "default"
    assert _response_output_text(response).strip()


async def test_reborn_legacy_responses_untyped_message_input_alias(
    reborn_responses_client,
):
    response = await _create_response(
        reborn_responses_client,
        path="/api/v1/responses",
        input=[
            {
                "role": "user",
                "content": "What is 2+2? Reply with just the number.",
            }
        ],
    )

    assert response["status"] == "completed"
    assert _response_output_text(response).strip()


async def test_reborn_legacy_responses_continue_and_retrieve(
    reborn_responses_client,
):
    first = await _create_response(reborn_responses_client, input="Say hello")
    second = await _create_response(
        reborn_responses_client,
        input="Now say goodbye",
        previous_response_id=first["id"],
    )

    assert second["status"] == "completed"
    assert second["id"] != first["id"]

    retrieved = await reborn_responses_client.get(f"/api/v1/responses/{second['id']}")
    assert retrieved.status_code == 200, retrieved.text
    retrieved_body = retrieved.json()
    assert retrieved_body["id"] == second["id"]
    assert _response_output_text(retrieved_body).strip()


async def test_reborn_legacy_responses_streaming_raw_sse(reborn_responses_client):
    async with reborn_responses_client.stream(
        "POST",
        "/v1/responses",
        json={"model": "default", "input": "Say hi", "stream": True},
    ) as response:
        assert response.status_code == 200
        events: list[str] = []
        async for line in response.aiter_lines():
            if line.startswith("event:"):
                events.append(line.removeprefix("event:").strip())
            if "response.completed" in events:
                break

    assert events
    assert "response.created" in events
    assert "response.completed" in events


async def test_reborn_legacy_responses_context_injection_approval(
    reborn_responses_client,
):
    response = await _create_response(
        reborn_responses_client,
        input="Go ahead with the transfer",
        x_context={
            "notification_response": {
                "notification_id": "msg_456",
                "action": "approved",
                "original_signal": "convert_now",
                "score": 72,
            }
        },
        stream=False,
    )

    assert response["status"] == "completed"
    assert _response_output_text(response).strip()


async def test_reborn_legacy_responses_context_injection_rejection(
    reborn_responses_client,
):
    response = await _create_response(
        reborn_responses_client,
        input="Cancel it",
        x_context={
            "notification_response": {
                "notification_id": "msg_789",
                "action": "rejected",
            }
        },
        stream=False,
    )

    assert response["status"] == "completed"


async def test_reborn_legacy_responses_rejects_missing_auth(
    reborn_openai_compat_server,
):
    async with httpx.AsyncClient(timeout=10) as client:
        response = await client.post(
            f"{reborn_openai_compat_server}/v1/responses",
            headers={"Content-Type": "application/json"},
            json={"model": "default", "input": "hello"},
        )

    assert response.status_code == 401


async def test_reborn_legacy_responses_rejects_empty_input_items(
    reborn_responses_client,
):
    response = await reborn_responses_client.post(
        "/v1/responses",
        json={"model": "default", "input": []},
    )

    assert response.status_code == 400
    body = response.json()
    assert body["error"]["param"] == "input"


async def test_reborn_legacy_responses_rejects_empty_text_input(
    reborn_responses_client,
):
    response = await reborn_responses_client.post(
        "/v1/responses",
        json={"model": "default", "input": ""},
    )

    assert response.status_code == 400
    body = response.json()
    assert body["error"]["param"] == "input"
