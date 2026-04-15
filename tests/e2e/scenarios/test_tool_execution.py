"""Tool execution E2E tests via the chat history API."""

import asyncio

from helpers import api_get, api_post, send_chat_and_wait_for_terminal_message


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


async def _wait_for_turn(
    base_url: str,
    thread_id: str,
    *,
    response_fragment: str | None = None,
    tool_name: str | None = None,
    result_fragment: str | None = None,
    timeout: float = 30.0,
) -> dict:
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=10,
        )
        assert response.status_code == 200, response.text
        history = response.json()
        turns = history.get("turns", [])
        if turns:
            turn = turns[-1]
            response_ok = response_fragment is None or (
                turn.get("response") and response_fragment.lower() in turn["response"].lower()
            )
            tool_ok = tool_name is None
            if tool_name is not None:
                for tool_call in turn.get("tool_calls", []):
                    if tool_call.get("name") != tool_name:
                        continue
                    preview = (tool_call.get("result_preview") or "").lower()
                    tool_ok = tool_call.get("has_result") is True and (
                        result_fragment is None or result_fragment.lower() in preview
                    )
                    if tool_ok:
                        break
            if response_ok and tool_ok:
                return turn
        await asyncio.sleep(0.25)

    raise AssertionError(
        f"Timed out waiting for turn: response_fragment={response_fragment!r}, "
        f"tool_name={tool_name!r}, result_fragment={result_fragment!r}"
    )


async def test_builtin_echo_tool(ironclaw_server):
    """A chat request can execute the built-in echo tool and persist its result."""
    thread_id = await _create_thread(ironclaw_server)
    await _send_chat_message(ironclaw_server, thread_id, "echo hello world")

    turn = await _wait_for_turn(
        ironclaw_server,
        thread_id,
        tool_name="echo",
        result_fragment="hello world",
    )

    assert any(tc.get("name") == "echo" for tc in turn.get("tool_calls", [])), turn


async def test_builtin_time_tool(ironclaw_server):
    """A chat request can execute the built-in time tool and persist its result."""
    thread_id = await _create_thread(ironclaw_server)
    await _send_chat_message(ironclaw_server, thread_id, "what time is it")

    turn = await _wait_for_turn(
        ironclaw_server,
        thread_id,
        tool_name="time",
    )

    assert any(tc.get("name") == "time" for tc in turn.get("tool_calls", [])), turn


async def test_non_tool_message_still_works(ironclaw_server):
    """Messages that do not trigger tools still get a normal assistant response."""
    thread_id = await _create_thread(ironclaw_server)
    await _send_chat_message(ironclaw_server, thread_id, "What is 2+2?")

    turn = await _wait_for_turn(
        ironclaw_server,
        thread_id,
        response_fragment="4",
        timeout=15.0,
    )

    assert "4" in (turn.get("response") or ""), turn


async def test_gateway_message_tool_round_trip(page, ironclaw_server):
    """A browser chat turn can route a proactive message through the gateway."""
    result = await send_chat_and_wait_for_terminal_message(page, "gateway broadcast smoke")

    assert result["role"] == "assistant"
    assert "Sent message to gateway" in result["text"], result

    thread_id = await page.evaluate("() => currentThreadId")
    assert thread_id, "browser should retain the active thread id"

    response = await api_get(
        ironclaw_server,
        f"/api/chat/history?thread_id={thread_id}",
        timeout=15,
    )
    assert response.status_code == 200, response.text
    history = response.json()
    turns = history.get("turns", [])
    assert turns, history
    last_turn = turns[-1]
    assert any(tc.get("name") == "message" for tc in last_turn.get("tool_calls", [])), last_turn
    assert "Sent message to gateway" in (last_turn.get("response") or ""), last_turn
