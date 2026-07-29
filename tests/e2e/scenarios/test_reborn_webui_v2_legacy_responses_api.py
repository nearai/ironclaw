"""Legacy Responses API coverage ported to standalone Reborn."""

import asyncio
import time
import uuid

import httpx
import pytest

# Retry statuses that indicate a transient/cold-start condition worth resending.
# 429 = under load; 502/503/504 = the first request racing the backend/LLM
# warm-up (a cold-start 503 flaked this smoke).
_TRANSIENT_STATUSES = {429, 502, 503, 504}
_MAX_ATTEMPTS = 6
# Cap the WHOLE retry sequence well under the 120s E2E test timeout. Each attempt
# is given only the remaining budget as its timeout, so even a hung backend
# cannot blow past this deadline — the test fails fast instead of being killed by
# the outer pytest timeout with a less useful error.
_RETRY_DEADLINE_SECONDS = 100.0

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
    # One idempotency key per logical create, reused on every retry. A transient
    # 5xx can be returned AFTER the Responses handler has already accepted the run,
    # so a keyless resend would create a duplicate run; the Responses API dedupes
    # on the `Idempotency-Key` header (see `create_response` → `idempotency_key_
    # from_headers`), making the resend safe.
    headers = {"Idempotency-Key": str(uuid.uuid4())}
    response = None
    deadline = time.monotonic() + _RETRY_DEADLINE_SECONDS
    for attempt in range(_MAX_ATTEMPTS):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        response = await client.post(
            path,
            json={"model": "default", **payload},
            headers=headers,
            timeout=remaining,
        )
        # Break on a non-transient status; retrying would not change the outcome.
        if response.status_code not in _TRANSIENT_STATUSES:
            break
        # No point sleeping after the final attempt, or once the budget is spent.
        backoff = 1 + attempt * 0.5
        if attempt == _MAX_ATTEMPTS - 1 or time.monotonic() + backoff >= deadline:
            break
        await asyncio.sleep(backoff)
    assert response is not None
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["id"].startswith("resp_")
    assert body["object"] == "response"
    return body


# --- `_create_response` retry-semantics unit coverage (mocked client) ----------
# These exercise the helper's retry contract without a live server: transient
# statuses retry, non-transient statuses stop immediately, retries are capped and
# never sleep after the final attempt, a single idempotency key is reused across
# attempts, and each attempt is bounded by the remaining budget.


class _FakeResponse:
    def __init__(self, status_code):
        self.status_code = status_code
        self.text = f"status {status_code}"

    def json(self):
        return {"id": "resp_mock", "object": "response"}


class _RecordingClient:
    """Minimal `httpx.AsyncClient` stand-in that replays scripted statuses."""

    def __init__(self, statuses):
        self._statuses = list(statuses)
        self.posts = []

    async def post(self, path, json, headers, timeout):
        index = min(len(self.posts), len(self._statuses) - 1)
        self.posts.append({"headers": dict(headers), "timeout": timeout})
        return _FakeResponse(self._statuses[index])


@pytest.fixture()
def _recorded_sleeps(monkeypatch):
    sleeps = []

    async def _fake_sleep(seconds):
        sleeps.append(seconds)

    monkeypatch.setattr(asyncio, "sleep", _fake_sleep)
    return sleeps


async def test_create_response_retries_transient_then_succeeds(_recorded_sleeps):
    # Every transient status (503/502/504/429) must be retried — a sequence that
    # exercises all four fails if any is dropped from `_TRANSIENT_STATUSES`.
    client = _RecordingClient([503, 502, 504, 429, 200])
    body = await _create_response(client, input="hi")
    assert body["object"] == "response"
    assert len(client.posts) == 5  # four transient retries, then success
    assert len(_recorded_sleeps) == 4  # one backoff between each retry, none after 200
    # A single idempotency key is reused on every attempt so an accepted-then-5xx
    # run is deduped rather than duplicated.
    keys = {post["headers"]["Idempotency-Key"] for post in client.posts}
    assert len(keys) == 1


async def test_create_response_succeeds_without_retry_or_sleep(_recorded_sleeps):
    client = _RecordingClient([200])
    await _create_response(client, input="hi")
    assert len(client.posts) == 1
    assert _recorded_sleeps == []


async def test_create_response_does_not_retry_non_transient_status(_recorded_sleeps):
    client = _RecordingClient([400])
    with pytest.raises(AssertionError):  # helper asserts a 200
        await _create_response(client, input="hi")
    assert len(client.posts) == 1  # a 4xx is not retried
    assert _recorded_sleeps == []


async def test_create_response_caps_attempts_without_trailing_sleep(_recorded_sleeps):
    client = _RecordingClient([503] * (_MAX_ATTEMPTS + 2))
    with pytest.raises(AssertionError):  # never reaches a 200
        await _create_response(client, input="hi")
    assert len(client.posts) == _MAX_ATTEMPTS  # capped at the attempt ceiling
    assert len(_recorded_sleeps) == _MAX_ATTEMPTS - 1  # no sleep after the last try


async def test_create_response_bounds_each_attempt_by_remaining_budget(_recorded_sleeps):
    client = _RecordingClient([200])
    await _create_response(client, input="hi")
    timeout = client.posts[0]["timeout"]
    assert 0 < timeout <= _RETRY_DEADLINE_SECONDS  # never an unbounded per-call wait


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
