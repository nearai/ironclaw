"""Served Reborn WebUI v2 session/thread/message API matrix tests.

These scenarios exercise the browser-facing `/api/webchat/v2/*` API through a
real `ironclaw-reborn serve` process. They intentionally live outside
`test_reborn_webui_v2_smoke.py`, which is already owned by the normal Reborn
WebUI v2 CI smoke lane; this file is the QA-matrix executable conversion for
REBCLI-043 rows that were previously represented only by Rust contract
coverage.
"""

import httpx

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_webui_harness import (
    client_action_id,
    create_thread,
    fetch_timeline,
    reborn_bearer_headers,
    send_message,
    wait_for_assistant_message,
)

pytest_plugins = ["reborn_webui_harness"]


async def test_reborn_v2_session_and_thread_lifecycle_served(reborn_v2_server):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        session = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/session",
            timeout=15,
        )
        session.raise_for_status()
        session_body = session.json()
        assert session_body["tenant_id"] == "reborn-v2-e2e"
        assert session_body["user_id"] == "reborn-v2-e2e-user"
        assert "attachments" in session_body
        assert "features" in session_body

        first_thread_id = await create_thread(client, reborn_v2_server)
        second_thread_id = await create_thread(client, reborn_v2_server)
        assert first_thread_id != second_thread_id

        first_page = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads?limit=1",
            timeout=15,
        )
        first_page.raise_for_status()
        first_page_body = first_page.json()
        assert len(first_page_body["threads"]) == 1
        assert first_page_body["next_cursor"], first_page_body

        second_page = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads",
            params={"limit": 10, "cursor": first_page_body["next_cursor"]},
            timeout=15,
        )
        second_page.raise_for_status()
        second_page_ids = {
            thread["thread_id"] for thread in second_page.json().get("threads", [])
        }
        assert first_thread_id in second_page_ids or second_thread_id in second_page_ids

        delete_response = await client.delete(
            f"{reborn_v2_server}/api/webchat/v2/threads/{first_thread_id}",
            timeout=15,
        )
        delete_response.raise_for_status()

        after_delete = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads?limit=20",
            timeout=15,
        )
        after_delete.raise_for_status()
        remaining_ids = {
            thread["thread_id"] for thread in after_delete.json().get("threads", [])
        }
        assert first_thread_id not in remaining_ids


async def test_reborn_v2_message_submission_and_timeline_served(reborn_v2_server):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_server)

        await send_message(client, reborn_v2_server, thread_id, "what is 2+2?")
        assistant = await wait_for_assistant_message(client, reborn_v2_server, thread_id)
        assert "4" in assistant.get("content", "")

        timeline = await fetch_timeline(client, reborn_v2_server, thread_id)
        messages = timeline["messages"]
        assert any(
            message["kind"] == "user" and "2+2" in message.get("content", "")
            for message in messages
        )
        assert any(
            message["kind"] == "assistant"
            and message["status"] == "finalized"
            and "4" in message.get("content", "")
            for message in messages
        )

        limited = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/timeline",
            params={"limit": 1},
            timeout=15,
        )
        limited.raise_for_status()
        assert len(limited.json()["messages"]) == 1


async def test_reborn_v2_session_thread_api_rejects_bad_requests(reborn_v2_server):
    async with httpx.AsyncClient() as anonymous:
        unauthenticated = await anonymous.get(
            f"{reborn_v2_server}/api/webchat/v2/session",
            timeout=15,
        )
        assert unauthenticated.status_code == 401

    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        invalid_create = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/threads",
            json={"client_action_id": ""},
            timeout=15,
        )
        assert invalid_create.status_code == 400

        unknown_timeline = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/threads/not-a-thread/timeline",
            timeout=15,
        )
        assert unknown_timeline.status_code == 404

        thread_id = await create_thread(client, reborn_v2_server)
        empty_message = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/messages",
            json={"client_action_id": client_action_id(), "content": ""},
            timeout=15,
        )
        assert empty_message.status_code == 400
