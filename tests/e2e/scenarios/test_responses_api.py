"""Responses API integration tests — OpenAI client, streaming, context injection."""

import uuid

import httpx
import pytest
from openai import OpenAI

from helpers import AUTH_TOKEN


@pytest.fixture()
async def responses_user(ironclaw_server):
    """Create a test user for Responses API tests, yield (base_url, user_token), clean up."""
    email = f"resp-{uuid.uuid4().hex[:8]}@example.com"
    async with httpx.AsyncClient(
        base_url=ironclaw_server,
        headers={"Authorization": f"Bearer {AUTH_TOKEN}", "Content-Type": "application/json"},
        timeout=10,
    ) as admin:
        r = await admin.post("/api/admin/users", json={
            "display_name": "Responses Test User",
            "email": email,
            "role": "member",
        })
        assert r.status_code == 200
        data = r.json()
        user_id = data["id"]
        user_token = data["token"]

        yield ironclaw_server, user_token

        await admin.delete(f"/api/admin/users/{user_id}")


@pytest.fixture()
async def openai_client(responses_user):
    """OpenAI client pointed at the test IronClaw instance."""
    base_url, user_token = responses_user
    return OpenAI(api_key=user_token, base_url=f"{base_url}/v1")


# ---------------------------------------------------------------
# Non-streaming
# ---------------------------------------------------------------


async def test_non_streaming_text_input(openai_client):
    response = openai_client.responses.create(
        model="default",
        input="Say hello in exactly 3 words",
    )
    assert response.id.startswith("resp_")
    assert response.status == "completed"
    assert len(response.output) > 0


async def test_non_streaming_messages_input(openai_client):
    response = openai_client.responses.create(
        model="default",
        input=[{"role": "user", "content": "What is 2+2? Reply with just the number."}],
    )
    assert response.status == "completed"
    assert len(response.output) > 0


# ---------------------------------------------------------------
# Multi-turn
# ---------------------------------------------------------------


async def test_continue_conversation(openai_client):
    r1 = openai_client.responses.create(
        model="default",
        input="Say hello",
    )
    assert r1.status == "completed"

    r2 = openai_client.responses.create(
        model="default",
        input="Now say goodbye",
        previous_response_id=r1.id,
    )
    assert r2.status == "completed"
    assert r2.id != r1.id


# ---------------------------------------------------------------
# GET by ID
# ---------------------------------------------------------------


async def test_get_response_by_id(openai_client):
    response = openai_client.responses.create(
        model="default",
        input="Remember this: the sky is blue",
    )
    retrieved = openai_client.responses.retrieve(response.id)
    assert retrieved.id == response.id
    assert len(retrieved.output) > 0


# ---------------------------------------------------------------
# Streaming
# ---------------------------------------------------------------


async def test_streaming_events(openai_client):
    stream = openai_client.responses.create(
        model="default",
        input="Count from 1 to 3",
        stream=True,
    )
    events = []
    full_text = ""
    for event in stream:
        events.append(event.type)
        if event.type == "response.output_text.delta":
            full_text += event.delta

    assert "response.created" in events
    assert "response.completed" in events
    assert len(full_text) > 0


async def test_streaming_raw_sse(responses_user):
    base_url, user_token = responses_user
    async with httpx.AsyncClient(timeout=30) as client:
        async with client.stream(
            "POST",
            f"{base_url}/v1/responses",
            headers={
                "Authorization": f"Bearer {user_token}",
                "Content-Type": "application/json",
            },
            json={"input": "Say hi", "stream": True},
        ) as resp:
            assert resp.status_code == 200
            event_count = 0
            async for line in resp.aiter_lines():
                if line.startswith("event:"):
                    event_count += 1
            assert event_count > 0


# ---------------------------------------------------------------
# Context injection
# ---------------------------------------------------------------


async def test_context_injection_approval(responses_user):
    base_url, user_token = responses_user
    async with httpx.AsyncClient(timeout=120) as client:
        r = await client.post(
            f"{base_url}/v1/responses",
            headers={
                "Authorization": f"Bearer {user_token}",
                "Content-Type": "application/json",
            },
            json={
                "input": "Go ahead with the transfer",
                "x_context": {
                    "notification_response": {
                        "notification_id": "msg_456",
                        "action": "approved",
                        "original_signal": "convert_now",
                        "score": 72,
                    }
                },
                "stream": False,
            },
        )
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "completed"
        assert len(data["output"]) > 0


async def test_context_injection_rejection(responses_user):
    base_url, user_token = responses_user
    async with httpx.AsyncClient(timeout=120) as client:
        r = await client.post(
            f"{base_url}/v1/responses",
            headers={
                "Authorization": f"Bearer {user_token}",
                "Content-Type": "application/json",
            },
            json={
                "input": "Cancel it",
                "x_context": {
                    "notification_response": {
                        "notification_id": "msg_789",
                        "action": "rejected",
                    }
                },
                "stream": False,
            },
        )
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "completed"


# ---------------------------------------------------------------
# Error cases
# ---------------------------------------------------------------


async def test_error_no_auth(ironclaw_server):
    async with httpx.AsyncClient(timeout=10) as client:
        r = await client.post(
            f"{ironclaw_server}/v1/responses",
            headers={"Content-Type": "application/json"},
            json={"input": "hello"},
        )
        assert r.status_code == 401


async def test_error_empty_input(responses_user):
    base_url, user_token = responses_user
    async with httpx.AsyncClient(timeout=10) as client:
        r = await client.post(
            f"{base_url}/v1/responses",
            headers={
                "Authorization": f"Bearer {user_token}",
                "Content-Type": "application/json",
            },
            json={"input": ""},
        )
        assert r.status_code == 400
