"""E2E test: v2 engine error handling and safety limits.

Tests the orchestrator's handling of edge cases that protect against runaway
execution and guide the LLM back on track:

1. **Max iterations** -- The mock LLM always returns an echo tool call for the
   "loop forever" trigger, so the orchestrator runs until it hits the default
   max_iterations cap (30). The thread should complete with a response that
   mentions the iteration limit.

2. **Tool intent nudge** -- The mock LLM returns a text response that expresses
   intent to use a tool ("Let me search for that information now.") without
   actually emitting a tool call. The orchestrator detects this and sends a
   nudge message ("You expressed intent ..."), after which the mock LLM returns
   a valid completion ("I found the information you requested.").

NOTE: A consecutive-errors test is intentionally omitted. The consecutive error
path requires the LLM to generate Python code blocks (```repl) that fail at
runtime, which is not feasible with the current mock LLM (it returns either
canned text or tool calls, never CodeAct code blocks).
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
_V2_ERR_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-err-e2e-")
_V2_ERR_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-err-e2e-home-")


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
async def v2_error_server(ironclaw_binary, mock_llm_server):
    """Start ironclaw with ENGINE_V2=true for error handling tests."""
    home_dir = _V2_ERR_HOME_TMPDIR.name
    os.makedirs(os.path.join(home_dir, ".ironclaw"), exist_ok=True)

    # Find two free ports
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
        "RUST_LOG": "ironclaw=debug",
        "RUST_BACKTRACE": "1",
        "ENGINE_V2": "true",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-v2-error-tester",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_V2_ERR_DB_TMPDIR.name, "v2-err-e2e.db"),
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
            f"v2 error-handling server failed to start on port {gateway_port}.\n"
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

async def _wait_for_response(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
    expect_substring: str | None = None,
    auto_approve: bool = False,
) -> dict:
    """Poll chat history until an assistant response appears.

    If ``auto_approve`` is True, any pending approval encountered while
    polling will be approved automatically. This is needed for the max
    iterations test where the echo tool may require approval on each
    iteration.
    """
    for _ in range(int(timeout * 2)):
        r = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        r.raise_for_status()
        history = r.json()

        # Auto-approve pending approvals so the loop doesn't stall
        if auto_approve:
            pending = history.get("pending_approval")
            if pending:
                request_id = pending.get("request_id", "")
                if request_id:
                    await api_post(
                        base_url,
                        "/api/chat/approval",
                        json={
                            "request_id": request_id,
                            "action": "approve",
                            "thread_id": thread_id,
                        },
                        timeout=10,
                    )

        turns = history.get("turns", [])
        if turns:
            last_response = turns[-1].get("response", "")
            if last_response:
                if expect_substring is None or expect_substring.lower() in last_response.lower():
                    return history

        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for response"
        + (f" containing '{expect_substring}'" if expect_substring else "")
        + f" in thread {thread_id}"
    )


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestV2EngineMaxIterations:
    """Verify the orchestrator enforces the max iterations safety limit."""

    async def test_max_iterations(self, v2_error_server):
        """Sending "loop forever" causes the mock LLM to always return an echo
        tool call. The orchestrator should stop after max_iterations (30) and
        produce a response mentioning the iteration limit."""
        # Create a fresh thread
        thread_r = await api_post(
            v2_error_server, "/api/chat/thread/new", timeout=15,
        )
        assert thread_r.status_code == 200
        thread_id = thread_r.json()["id"]

        # Send the trigger message
        send_r = await api_post(
            v2_error_server,
            "/api/chat/send",
            json={
                "content": "loop forever",
                "thread_id": thread_id,
            },
            timeout=30,
        )
        assert send_r.status_code in (200, 202)

        # Wait for the orchestrator to hit the iteration cap.
        # 30 iterations with tool calls take time — use a generous timeout.
        # The final response should mention the iteration limit.
        history = await _wait_for_response(
            v2_error_server,
            thread_id,
            timeout=120,
            expect_substring="iteration",
            auto_approve=True,
        )

        # Verify the response references the iteration cap
        all_responses = " ".join(
            t.get("response", "") for t in history.get("turns", [])
        ).lower()
        assert "iteration" in all_responses, (
            f"Expected response to mention 'iteration' limit, got: "
            f"{all_responses[:500]}"
        )


class TestV2EngineToolIntentNudge:
    """Verify the orchestrator nudges the LLM when it expresses tool intent
    without actually calling a tool, and that the nudge leads to completion."""

    async def test_tool_intent_nudge(self, v2_error_server):
        """Send "search intent" → LLM responds with text expressing intent
        ("Let me search for that information now.") but no tool call →
        orchestrator sends nudge → LLM sees nudge ("You expressed intent...")
        and responds with "I found the information you requested." →
        thread completes with "found" in the response."""
        # Create a fresh thread
        thread_r = await api_post(
            v2_error_server, "/api/chat/thread/new", timeout=15,
        )
        assert thread_r.status_code == 200
        thread_id = thread_r.json()["id"]

        # Send the trigger message
        send_r = await api_post(
            v2_error_server,
            "/api/chat/send",
            json={
                "content": "search intent",
                "thread_id": thread_id,
            },
            timeout=30,
        )
        assert send_r.status_code in (200, 202)

        # The orchestrator should detect the tool intent in the first LLM
        # response, send a nudge, and the mock LLM will respond to the nudge
        # with "I found the information you requested."
        history = await _wait_for_response(
            v2_error_server,
            thread_id,
            timeout=45,
            expect_substring="found",
        )

        # Verify the final response contains the expected content
        all_responses = " ".join(
            t.get("response", "") for t in history.get("turns", [])
        ).lower()
        assert "found" in all_responses, (
            f"Expected response to contain 'found' after nudge recovery, got: "
            f"{all_responses[:500]}"
        )
