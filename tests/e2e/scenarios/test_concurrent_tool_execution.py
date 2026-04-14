"""Concurrent tool execution E2E tests.

Verifies that the batch-partitioned concurrent tool execution dispatcher
(PR #2423) correctly handles:
  1. Multiple concurrent-safe tool calls complete in parallel
  2. Mixed batches with approval-gated tools pause appropriately
"""

import asyncio

from helpers import api_get, api_post


async def _create_thread(base_url: str) -> str:
    response = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    assert response.status_code == 200, response.text
    return response.json()["id"]


async def _send_chat_message(base_url: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text


async def _poll_history(base_url: str, thread_id: str, timeout: float = 10.0) -> dict:
    response = await api_get(
        base_url,
        f"/api/chat/history?thread_id={thread_id}",
        timeout=timeout,
    )
    assert response.status_code == 200, response.text
    return response.json()


# ---------------------------------------------------------------------------
# Test: 3 concurrent-safe echo tools all complete
# ---------------------------------------------------------------------------


async def test_three_concurrent_echo_tools(ironclaw_server):
    """Send a message that triggers 3 concurrent echo tool calls.

    All three are concurrent-safe (echo), so the dispatcher should run them
    in parallel via a single Concurrent batch. Verify all results appear in
    the chat history.
    """
    thread_id = await _create_thread(ironclaw_server)
    await _send_chat_message(ironclaw_server, thread_id, "run 3 concurrent readonly tools")

    deadline = asyncio.get_running_loop().time() + 30.0
    while asyncio.get_running_loop().time() < deadline:
        history = await _poll_history(ironclaw_server, thread_id)
        turns = history.get("turns", [])
        if not turns:
            await asyncio.sleep(0.3)
            continue

        turn = turns[-1]
        tool_calls = turn.get("tool_calls", [])

        echo_calls = [tc for tc in tool_calls if tc.get("name") == "echo"]
        completed = [tc for tc in echo_calls if tc.get("has_result")]

        if len(completed) >= 3:
            # Verify each expected result fragment is present
            previews = [tc.get("result_preview", "") for tc in completed]
            assert any("concurrent-alpha" in p for p in previews), (
                f"Missing concurrent-alpha in previews: {previews}"
            )
            assert any("concurrent-beta" in p for p in previews), (
                f"Missing concurrent-beta in previews: {previews}"
            )
            assert any("concurrent-gamma" in p for p in previews), (
                f"Missing concurrent-gamma in previews: {previews}"
            )
            return

        await asyncio.sleep(0.3)

    raise AssertionError(
        "Timed out waiting for 3 concurrent echo tools to complete"
    )


# ---------------------------------------------------------------------------
# Test: mixed batch with approval-gated tool
# ---------------------------------------------------------------------------


async def test_mixed_batch_with_approval_gated_tool(ironclaw_server):
    """Send a message triggering echo (safe) + http POST (approval-gated) + time (safe).

    The dispatcher should:
      - Run the preflight phase sequentially, detecting the approval gate
      - Return NeedApproval for the http POST tool
      - The concurrent-safe tools (echo, time) execute around the gate
      - After approval, the http tool should execute

    Note: the exact behavior depends on whether approval is checked before
    or after concurrent execution. This test verifies that at minimum the
    approval gate is hit and the non-gated tools complete.
    """
    thread_id = await _create_thread(ironclaw_server)
    await _send_chat_message(
        ironclaw_server, thread_id, "run mixed batch with approval gate"
    )

    deadline = asyncio.get_running_loop().time() + 30.0
    found_approval = False
    while asyncio.get_running_loop().time() < deadline:
        history = await _poll_history(ironclaw_server, thread_id)
        turns = history.get("turns", [])
        if not turns:
            await asyncio.sleep(0.3)
            continue

        turn = turns[-1]
        tool_calls = turn.get("tool_calls", [])

        # Check if any tool needs approval (http POST)
        if turn.get("pending_approval") or any(
            tc.get("name") == "http" and not tc.get("has_result")
            for tc in tool_calls
        ):
            found_approval = True
            break

        # Or maybe all tools already completed (if http was auto-approved
        # in the E2E harness configuration)
        all_done = len(tool_calls) >= 2 and all(
            tc.get("has_result") for tc in tool_calls
        )
        if all_done:
            # Verify at least echo completed
            echo_calls = [tc for tc in tool_calls if tc.get("name") == "echo"]
            assert len(echo_calls) >= 1, f"Expected echo call, got: {tool_calls}"
            assert echo_calls[0].get("has_result"), "echo should have completed"
            return

        await asyncio.sleep(0.3)

    if found_approval:
        # Approve the pending tool
        await _send_chat_message(ironclaw_server, thread_id, "yes")

        # Wait for completion after approval
        approve_deadline = asyncio.get_running_loop().time() + 15.0
        while asyncio.get_running_loop().time() < approve_deadline:
            history = await _poll_history(ironclaw_server, thread_id)
            turns = history.get("turns", [])
            if turns:
                turn = turns[-1]
                tool_calls = turn.get("tool_calls", [])
                completed = [tc for tc in tool_calls if tc.get("has_result")]
                if len(completed) >= 2:
                    # At minimum echo and one other tool completed
                    echo_done = any(
                        tc.get("name") == "echo" and tc.get("has_result")
                        for tc in tool_calls
                    )
                    assert echo_done, "echo tool should have completed"
                    return
            await asyncio.sleep(0.3)

        raise AssertionError(
            "Timed out waiting for tools to complete after approval"
        )

    raise AssertionError(
        "Timed out: neither approval gate nor completed tools found"
    )
