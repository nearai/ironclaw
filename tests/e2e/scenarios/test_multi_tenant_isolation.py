"""Multi-tenant isolation regressions for public gateway surfaces."""

import asyncio
import uuid

import httpx

from helpers import api_get, api_post, create_member_user, sse_stream


async def _create_thread(base_url: str, token: str) -> str:
    response = await api_post(
        base_url,
        "/api/chat/thread/new",
        token=token,
        timeout=15,
    )
    assert response.status_code == 200, response.text
    return response.json()["id"]


async def _send_chat(base_url: str, token: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        token=token,
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code == 202, response.text


async def _set_http_tool_approval(base_url: str, token: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.put(
            f"{base_url}/api/settings/tools/http",
            headers={"Authorization": f"Bearer {token}"},
            json={"state": "ask_each_time"},
            timeout=15,
        )
    assert response.status_code == 200, response.text


async def _wait_for_history(
    base_url: str,
    token: str,
    thread_id: str,
    *,
    expect_pending: bool | None = None,
    expected_user_input: str | None = None,
    timeout: float = 30.0,
) -> dict:
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            token=token,
            timeout=10,
        )
        assert response.status_code == 200, response.text
        history = response.json()
        turns = history.get("turns", [])
        pending = history.get("pending_gate")

        pending_ok = expect_pending is None or bool(pending) == expect_pending
        input_ok = expected_user_input is None or any(
            expected_user_input in (turn.get("user_input") or "") for turn in turns
        )
        if pending_ok and input_ok:
            return history

        await asyncio.sleep(0.25)

    raise AssertionError(
        "Timed out waiting for history: "
        f"thread_id={thread_id}, expect_pending={expect_pending}, "
        f"expected_user_input={expected_user_input!r}"
    )


async def _read_sse_lines_for(response, duration: float) -> list[str]:
    lines = []
    deadline = asyncio.get_running_loop().time() + duration
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            return lines
        try:
            line = await asyncio.wait_for(response.content.readline(), timeout=remaining)
        except asyncio.TimeoutError:
            return lines
        if not line:
            return lines
        lines.append(line.decode("utf-8", errors="replace").rstrip("\r\n"))


async def _two_member_users(base_url: str) -> tuple[dict[str, str], dict[str, str]]:
    suffix = uuid.uuid4().hex[:8]
    alice = await create_member_user(
        base_url,
        display_name=f"Alice Isolation {suffix}",
        email=f"alice-isolation-{suffix}@example.test",
    )
    bob = await create_member_user(
        base_url,
        display_name=f"Bob Isolation {suffix}",
        email=f"bob-isolation-{suffix}@example.test",
    )
    return alice, bob


async def test_chat_thread_history_and_list_are_user_scoped(ironclaw_server):
    """Bob must not list or read Alice's gateway thread by guessed id."""
    alice, bob = await _two_member_users(ironclaw_server)
    thread_id = await _create_thread(ironclaw_server, alice["token"])
    marker = f"alice-private-thread-{uuid.uuid4().hex}"

    await _send_chat(ironclaw_server, alice["token"], thread_id, marker)
    alice_history = await _wait_for_history(
        ironclaw_server,
        alice["token"],
        thread_id,
        expected_user_input=marker,
    )
    assert any(marker in (turn.get("user_input") or "") for turn in alice_history["turns"])

    bob_history = await api_get(
        ironclaw_server,
        f"/api/chat/history?thread_id={thread_id}",
        token=bob["token"],
        timeout=10,
    )
    assert bob_history.status_code == 404, bob_history.text

    bob_threads = await api_get(
        ironclaw_server,
        "/api/chat/threads",
        token=bob["token"],
        timeout=10,
    )
    assert bob_threads.status_code == 200, bob_threads.text
    bob_thread_ids = {
        item["id"] for item in bob_threads.json().get("threads", [])
    }
    assistant = bob_threads.json().get("assistant_thread")
    if assistant:
        bob_thread_ids.add(assistant["id"])
    assert thread_id not in bob_thread_ids


async def test_approval_gate_resolution_is_user_scoped(ironclaw_server):
    """Bob must not resolve Alice's pending approval gate by request id."""
    alice, bob = await _two_member_users(ironclaw_server)
    await _set_http_tool_approval(ironclaw_server, alice["token"])
    thread_id = await _create_thread(ironclaw_server, alice["token"])

    await _send_chat(
        ironclaw_server,
        alice["token"],
        thread_id,
        f"make approval post cross-user-{uuid.uuid4().hex[:8]}",
    )
    pending_history = await _wait_for_history(
        ironclaw_server,
        alice["token"],
        thread_id,
        expect_pending=True,
    )
    request_id = pending_history["pending_gate"]["request_id"]

    bob_approval = await api_post(
        ironclaw_server,
        "/api/chat/approval",
        token=bob["token"],
        json={
            "request_id": request_id,
            "action": "approve",
            "thread_id": thread_id,
        },
        timeout=15,
    )
    assert bob_approval.status_code in (403, 404, 409), bob_approval.text

    still_pending = await _wait_for_history(
        ironclaw_server,
        alice["token"],
        thread_id,
        expect_pending=True,
    )
    assert still_pending["pending_gate"]["request_id"] == request_id

    cleanup = await api_post(
        ironclaw_server,
        "/api/chat/approval",
        token=alice["token"],
        json={
            "request_id": request_id,
            "action": "deny",
            "thread_id": thread_id,
        },
        timeout=15,
    )
    assert cleanup.status_code == 202, cleanup.text
    await _wait_for_history(
        ironclaw_server,
        alice["token"],
        thread_id,
        expect_pending=False,
    )


async def test_approval_sse_event_is_user_scoped(ironclaw_server):
    """Alice's approval event must not appear on Bob's SSE stream."""
    alice, bob = await _two_member_users(ironclaw_server)
    await _set_http_tool_approval(ironclaw_server, alice["token"])
    thread_id = await _create_thread(ironclaw_server, alice["token"])

    async with sse_stream(
        ironclaw_server,
        token=bob["token"],
        timeout=10,
    ) as bob_events:
        assert bob_events.status == 200

        await _send_chat(
            ironclaw_server,
            alice["token"],
            thread_id,
            f"make approval post sse-isolation-{uuid.uuid4().hex[:8]}",
        )
        pending_history = await _wait_for_history(
            ironclaw_server,
            alice["token"],
            thread_id,
            expect_pending=True,
        )
        request_id = pending_history["pending_gate"]["request_id"]

        bob_lines = await _read_sse_lines_for(bob_events, 1.5)

    serialized = "\n".join(bob_lines)
    assert request_id not in serialized
    assert thread_id not in serialized
    assert "approval_needed" not in serialized
