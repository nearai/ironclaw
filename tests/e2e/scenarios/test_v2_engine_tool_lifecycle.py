"""E2E test: v2 engine tool execution lifecycle.

Tests the full tool execution path through the v2 engine -- the basic
contract that was previously only covered for the v1 engine
(test_tool_execution.py). This is the fundamental gap identified in the
#2193 audit: zero engine-level e2e tests for the tool call -> result ->
response path.

Covers:
1. Single tool call (echo) completes through v2
2. Single tool call (time) completes through v2
3. Text-only message (no tools) completes through v2
4. Parallel tool calls (echo + time dispatched simultaneously)
5. Multi-step tool chain (echo -> result -> time -> result -> text)
6. Multi-turn tool usage (tool call in turn 1, another in turn 2)
"""

import asyncio
import os
import signal
import socket
import tempfile
from pathlib import Path

import httpx
import pytest

import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import api_get, api_post, AUTH_TOKEN, wait_for_ready


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

ROOT = Path(__file__).resolve().parent.parent.parent.parent
_V2_TOOL_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-tool-e2e-")
_V2_TOOL_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-tool-e2e-home-")


def _forward_coverage_env(env: dict):
    """Forward LLVM coverage env vars from outer environment."""
    for key in os.environ:
        if key.startswith(("CARGO_LLVM_COV", "LLVM_", "CARGO_ENCODED_RUSTFLAGS",
                           "CARGO_INCREMENTAL")):
            env[key] = os.environ[key]


async def _stop_process(proc, sig=signal.SIGINT, timeout=5):
    """Send signal and wait for process to exit."""
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
async def v2_tool_server(ironclaw_binary, mock_llm_server):
    """Start ironclaw with ENGINE_V2=true for tool lifecycle tests."""
    home_dir = _V2_TOOL_HOME_TMPDIR.name
    os.makedirs(os.path.join(home_dir, ".ironclaw"), exist_ok=True)

    socks = []
    for _ in range(2):
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.bind(("127.0.0.1", 0))
        socks.append(s)
    gateway_port = socks[0].getsockname()[1]
    http_port = socks[1].getsockname()[1]
    for s in socks:
        s.close()

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "ENGINE_V2": "true",
        "AGENT_AUTO_APPROVE_TOOLS": "true",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-v2-tool-tester",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        # Dummy key: mock LLM ignores it, but openai_compatible config requires auth.
        "LLM_API_KEY": "mock-api-key",
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_V2_TOOL_DB_TMPDIR.name, "v2-tool-e2e.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "false",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "false",
        "ONBOARD_COMPLETED": "true",
    }
    _forward_coverage_env(env)

    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary, "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )

    base_url = f"http://127.0.0.1:{gateway_port}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=60)
        yield base_url
    except TimeoutError:
        if proc.returncode is None:
            await _stop_process(proc, timeout=2)
        stderr_bytes = b""
        if proc.stderr:
            try:
                stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
            except asyncio.TimeoutError:
                pass
        pytest.fail(
            f"v2 tool lifecycle server failed to start on port {gateway_port}.\n"
            f"stderr: {stderr_bytes.decode('utf-8', errors='replace')}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

async def _create_thread(base_url: str) -> str:
    r = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    assert r.status_code == 200, r.text
    return r.json()["id"]


async def _send(base_url: str, thread_id: str, content: str) -> None:
    r = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert r.status_code in (200, 202), r.text


async def _wait_for_response(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
    expect_substring: str | None = None,
    min_turns: int = 1,
) -> dict:
    """Poll chat history until an assistant response matching criteria appears."""
    last_history = None
    for _ in range(int(timeout * 2)):
        r = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        r.raise_for_status()
        history = r.json()
        last_history = history
        turns = history.get("turns", [])

        if len(turns) < min_turns:
            await asyncio.sleep(0.5)
            continue

        last = turns[-1]
        response = last.get("response", "")

        if not response:
            await asyncio.sleep(0.5)
            continue

        if expect_substring is None or expect_substring.lower() in response.lower():
            return history

        await asyncio.sleep(0.5)

    # Include last known state in the error for debugging
    debug_info = ""
    if last_history:
        turns = last_history.get("turns", [])
        if turns:
            last = turns[-1]
            debug_info = f"\nLast response: {last.get('response', '')[:200]!r}"

    raise AssertionError(
        f"Timed out waiting for response in thread {thread_id}: "
        f"expect_substring={expect_substring!r}, min_turns={min_turns}"
        + debug_info
    )


# ---------------------------------------------------------------------------
# Tests: single tool calls
# ---------------------------------------------------------------------------

class TestV2EngineSingleTool:
    """Verify that single tool calls complete through the v2 engine."""

    async def test_echo_tool(self, v2_tool_server):
        """echo tool call -> result -> text response through v2 orchestrator."""
        thread_id = await _create_thread(v2_tool_server)
        await _send(v2_tool_server, thread_id, "echo hello from v2")

        history = await _wait_for_response(
            v2_tool_server,
            thread_id,
            expect_substring="hello from v2",
            timeout=30,
        )

        turn = history["turns"][-1]
        assert "hello from v2" in turn["response"].lower()

        # Verify tool_calls are persisted to chat history
        tool_calls = turn.get("tool_calls", [])
        assert len(tool_calls) >= 1, (
            f"Expected tool_calls in v2 history, got: {tool_calls}"
        )
        assert tool_calls[0]["name"] == "echo"
        assert tool_calls[0]["has_result"] is True
        # Verify result_preview contains actual tool output, not just non-null
        preview = tool_calls[0].get("result_preview", "")
        assert "hello from v2" in preview.lower(), (
            f"Expected echo output in result_preview, got: {preview[:200]}"
        )

    async def test_time_tool(self, v2_tool_server):
        """time tool call -> result -> text response through v2 orchestrator."""
        thread_id = await _create_thread(v2_tool_server)
        await _send(v2_tool_server, thread_id, "what time is it")

        # The time tool returns a timestamp; the mock LLM summarizes it
        history = await _wait_for_response(
            v2_tool_server,
            thread_id,
            expect_substring="tool returned",
            timeout=30,
        )

        response = history["turns"][-1]["response"]
        assert "returned" in response.lower(), (
            f"Expected time tool result in response, got: {response[:200]}"
        )

        # Verify tool_calls persistence
        tool_calls = history["turns"][-1].get("tool_calls", [])
        assert len(tool_calls) >= 1, (
            f"Expected tool_calls for time tool, got: {tool_calls}"
        )
        assert tool_calls[0]["name"] == "time"
        assert tool_calls[0]["has_result"] is True

    async def test_text_only(self, v2_tool_server):
        """Non-tool message completes through v2 without tool calls."""
        thread_id = await _create_thread(v2_tool_server)
        await _send(v2_tool_server, thread_id, "What is 2+2?")

        history = await _wait_for_response(
            v2_tool_server,
            thread_id,
            expect_substring="4",
            timeout=20,
        )

        response = history["turns"][-1]["response"]
        assert "4" in response


# ---------------------------------------------------------------------------
# Tests: parallel and multi-step
# ---------------------------------------------------------------------------

class TestV2EngineMultiTool:
    """Verify multi-tool patterns through the v2 engine."""

    async def test_parallel_tool_calls(self, v2_tool_server):
        """Two tools dispatched in one LLM response both complete."""
        thread_id = await _create_thread(v2_tool_server)
        await _send(v2_tool_server, thread_id, "parallel echo and time")

        # The mock LLM returns both tools, engine runs them, mock
        # summarizes "Dispatched 2 tools: ...".
        history = await _wait_for_response(
            v2_tool_server,
            thread_id,
            expect_substring="dispatched 2 tools",
            timeout=45,
        )

        response = history["turns"][-1]["response"]
        assert "parallel-test" in response, (
            f"Expected echo result in parallel response, got: {response[:300]}"
        )

        # Verify both tool calls are persisted
        tool_calls = history["turns"][-1].get("tool_calls", [])
        assert len(tool_calls) >= 2, (
            f"Expected at least 2 tool_calls for parallel dispatch, got: {tool_calls}"
        )
        tc_names = {tc["name"] for tc in tool_calls}
        assert "echo" in tc_names, f"Expected 'echo' in tool_calls, got names: {tc_names}"
        assert "time" in tc_names, f"Expected 'time' in tool_calls, got names: {tc_names}"
        assert all(tc["has_result"] for tc in tool_calls), (
            f"Expected all tool_calls to have results, got: {tool_calls}"
        )

    async def test_multi_step_chain(self, v2_tool_server):
        """Multi-step: echo -> result -> time -> result -> text completion.

        The mock LLM returns echo first, waits for result, then returns
        time, waits for result, then returns completion text. This
        exercises the v2 engine's ability to handle sequential tool
        chains without entering an infinite loop (the #2402 pattern).
        """
        thread_id = await _create_thread(v2_tool_server)
        await _send(v2_tool_server, thread_id, "multi step echo then time")

        history = await _wait_for_response(
            v2_tool_server,
            thread_id,
            expect_substring="multi-step complete",
            timeout=60,
        )

        response = history["turns"][-1]["response"]
        assert "multi-step complete" in response.lower(), (
            f"Expected multi-step completion, got: {response[:200]}"
        )

        # Verify both sequential tool calls are persisted
        tool_calls = history["turns"][-1].get("tool_calls", [])
        assert len(tool_calls) >= 2, (
            f"Expected at least 2 tool_calls for multi-step chain, got: {tool_calls}"
        )
        # Echo runs first, then time -- both should have results
        assert all(tc["has_result"] for tc in tool_calls), (
            f"Expected all tool_calls to have results, got: {tool_calls}"
        )


# ---------------------------------------------------------------------------
# Tests: multi-turn
# ---------------------------------------------------------------------------

class TestV2EngineMultiTurn:
    """Verify tool usage across multiple conversation turns."""

    async def test_tool_then_text_then_tool(self, v2_tool_server):
        """Turn 1: echo tool. Turn 2: text question. Turn 3: time tool.

        Verifies the v2 engine maintains conversation state and can
        alternate between tool-using and text-only turns.
        """
        thread_id = await _create_thread(v2_tool_server)

        # Turn 1: echo
        await _send(v2_tool_server, thread_id, "echo first turn")
        await _wait_for_response(
            v2_tool_server,
            thread_id,
            expect_substring="first turn",
            timeout=30,
        )

        # Turn 2: text-only
        await _send(v2_tool_server, thread_id, "What is 2+2?")
        await _wait_for_response(
            v2_tool_server,
            thread_id,
            expect_substring="4",
            min_turns=2,
            timeout=30,
        )

        # Turn 3: time
        await _send(v2_tool_server, thread_id, "what time is it")
        history = await _wait_for_response(
            v2_tool_server,
            thread_id,
            expect_substring="tool returned",
            min_turns=3,
            timeout=30,
        )

        assert len(history["turns"]) >= 3, (
            f"Expected at least 3 turns, got {len(history['turns'])}"
        )

        # Verify tool_calls persisted for tool turns, absent for text turn
        turns = history["turns"]

        # Turn 1: echo -- should have tool_calls
        t1_calls = turns[0].get("tool_calls", [])
        assert len(t1_calls) >= 1, (
            f"Turn 1 (echo) should have tool_calls, got: {t1_calls}"
        )
        assert t1_calls[0]["name"] == "echo"
        assert t1_calls[0]["has_result"] is True

        # Turn 2: text-only -- should have no tool_calls
        t2_calls = turns[1].get("tool_calls", [])
        assert len(t2_calls) == 0, (
            f"Turn 2 (text) should have no tool_calls, got: {t2_calls}"
        )

        # Turn 3: time -- should have tool_calls
        t3_calls = turns[2].get("tool_calls", [])
        assert len(t3_calls) >= 1, (
            f"Turn 3 (time) should have tool_calls, got: {t3_calls}"
        )
        assert t3_calls[0]["name"] == "time"
        assert t3_calls[0]["has_result"] is True
