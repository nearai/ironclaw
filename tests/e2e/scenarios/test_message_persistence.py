"""E2E tests for user message persistence (#2409).

Verifies that user messages are persisted to the DB immediately at send
time (before the agent loop processes them), so they survive thread
switches and are never duplicated.
"""

import asyncio
import os
import signal
import socket
import tempfile
from pathlib import Path

import pytest

import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import (
    AUTH_TOKEN,
    SEL,
    api_get,
    api_post,
    send_chat_and_wait_for_terminal_message,
    wait_for_ready,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

async def _wait_for_turn_in_history(
    base_url: str,
    thread_id: str,
    *,
    predicate=None,
    timeout: float = 20.0,
) -> list:
    """Poll chat history until a turn matching `predicate` appears.

    Returns the full turns list on success.
    """
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        resp = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}")
        assert resp.status_code == 200, resp.text
        turns = resp.json()["turns"]
        if predicate is None and len(turns) > 0:
            return turns
        if predicate is not None and any(predicate(t) for t in turns):
            return turns
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for matching turn in history for thread {thread_id}"
    )


# ---------------------------------------------------------------------------
# v1 engine tests (use the default session-scoped ironclaw_server)
# ---------------------------------------------------------------------------


async def test_user_message_appears_in_history_immediately(page, ironclaw_server):
    """After POST /api/chat/send, the user message is in the DB before the
    agent loop responds — the core early-persist guarantee."""
    # Create an isolated thread
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_id = resp.json()["id"]

    # Send a message via API (not UI) to avoid waiting for the response
    resp = await api_post(
        ironclaw_server,
        "/api/chat/send",
        json={"content": "early persist check", "thread_id": thread_id},
    )
    assert resp.status_code == 202, resp.text

    # Immediately query history — the DB write is synchronous before 202
    resp = await api_get(
        ironclaw_server,
        f"/api/chat/history?thread_id={thread_id}",
    )
    assert resp.status_code == 200, resp.text
    turns = resp.json()["turns"]
    assert len(turns) >= 1, f"Expected at least 1 turn, got {turns}"
    assert turns[0]["user_input"] == "early persist check", turns[0]
    # The agent hasn't responded yet, so the turn should be in-progress
    assert turns[0]["state"] in ("Processing", "Failed"), turns[0]["state"]


async def test_message_survives_thread_switch(page, ironclaw_server):
    """Send on thread A, switch to B, switch back to A — message is retained."""
    # Create thread A via the UI new-thread button
    prev_thread = await page.evaluate("() => currentThreadId")
    await page.locator("#thread-new-btn").click()
    await page.wait_for_function(
        "(prev) => !!currentThreadId && currentThreadId !== prev",
        arg=prev_thread,
        timeout=10000,
    )
    thread_a = await page.evaluate("() => currentThreadId")

    # Send a message on thread A and wait for the assistant response
    result = await send_chat_and_wait_for_terminal_message(page, "hello")
    assert result["role"] == "assistant"

    # Wait for DB settlement
    await page.wait_for_timeout(2000)

    # Create thread B (switch away from A)
    await page.locator("#thread-new-btn").click()
    await page.wait_for_function(
        "(a) => !!currentThreadId && currentThreadId !== a",
        arg=thread_a,
        timeout=10000,
    )

    # Switch back to thread A
    await page.evaluate("(id) => switchThread(id)", thread_a)
    await page.wait_for_function(
        "(id) => currentThreadId === id",
        arg=thread_a,
        timeout=10000,
    )

    # Wait for the user message to reappear from the DB-backed history load
    await page.locator(SEL["message_user"]).filter(
        has_text="hello"
    ).wait_for(state="visible", timeout=15000)

    # Also verify via API
    resp = await api_get(
        ironclaw_server,
        f"/api/chat/history?thread_id={thread_a}",
    )
    assert resp.status_code == 200, resp.text
    turns = resp.json()["turns"]
    assert any("hello" in t["user_input"].lower() for t in turns), turns


async def test_no_duplicate_user_messages_after_processing(page, ironclaw_server):
    """After full agent processing, there is exactly one user message in the DB
    — the GATEWAY_PERSISTED_FLAG prevents double-write."""
    # Create an isolated thread
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_id = resp.json()["id"]

    # Switch the UI to this thread
    await page.evaluate("(id) => switchThread(id)", thread_id)
    await page.wait_for_function(
        "(id) => currentThreadId === id",
        arg=thread_id,
        timeout=10000,
    )

    # Send a message and wait for the full response
    result = await send_chat_and_wait_for_terminal_message(page, "What is 2+2?")
    assert result["role"] == "assistant"
    assert "4" in result["text"], result

    # Wait for DB settlement
    await page.wait_for_timeout(3000)

    # Switch away from the thread to force DB fallback on history query
    await page.locator("#thread-new-btn").click()
    await page.wait_for_function(
        "(tid) => !!currentThreadId && currentThreadId !== tid",
        arg=thread_id,
        timeout=10000,
    )

    # Query history via API — since thread is no longer active in-memory,
    # this reads from the database
    resp = await api_get(
        ironclaw_server,
        f"/api/chat/history?thread_id={thread_id}",
    )
    assert resp.status_code == 200, resp.text
    turns = resp.json()["turns"]

    # There must be exactly one user turn (not two from double-persist)
    user_turns = [t for t in turns if t.get("user_input")]
    assert len(user_turns) == 1, (
        f"Expected exactly 1 user turn, got {len(user_turns)}: {user_turns}"
    )
    assert "2+2" in user_turns[0]["user_input"] or "2 + 2" in user_turns[0]["user_input"]
    assert user_turns[0].get("response") and "4" in user_turns[0]["response"]
    assert user_turns[0]["state"] == "Completed", user_turns[0]["state"]


# ---------------------------------------------------------------------------
# v2 engine test (dedicated server with ENGINE_V2=true)
# ---------------------------------------------------------------------------

ROOT = Path(__file__).resolve().parent.parent.parent.parent
_V2_PERSIST_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-persist-e2e-")
_V2_PERSIST_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-persist-e2e-home-")


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


@pytest.fixture(scope="module")
async def v2_persistence_server(ironclaw_binary, mock_llm_server):
    """Start ironclaw with ENGINE_V2=true for message persistence tests."""
    home_dir = _V2_PERSIST_HOME_TMPDIR.name
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
        "RUST_LOG": "ironclaw=debug",
        "RUST_BACKTRACE": "1",
        "ENGINE_V2": "true",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-v2-persist-tester",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_V2_PERSIST_DB_TMPDIR.name, "v2-persist-e2e.db"),
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
            f"v2 persistence server failed to start on port {gateway_port}.\n"
            f"stderr: {stderr_bytes.decode('utf-8', errors='replace')}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


async def test_v2_no_duplicate_user_messages(v2_persistence_server):
    """v2 engine: the GATEWAY_PERSISTED_FLAG prevents double-write to v1 DB."""
    base_url = v2_persistence_server

    # Create a thread
    resp = await api_post(base_url, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_id = resp.json()["id"]

    # Send a message and wait for the agent to complete
    resp = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": "hello", "thread_id": thread_id},
    )
    assert resp.status_code == 202, resp.text

    # Wait for the turn to complete (assistant responds)
    turns = await _wait_for_turn_in_history(
        base_url,
        thread_id,
        predicate=lambda t: t.get("response") and len(t["response"]) > 0,
        timeout=30,
    )

    # Create a second thread to force the original out of in-memory cache
    resp2 = await api_post(base_url, "/api/chat/thread/new")
    assert resp2.status_code == 200, resp2.text

    # Small delay to let any async DB writes settle
    await asyncio.sleep(2)

    # Re-query history for the original thread (now DB-backed)
    resp = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}")
    assert resp.status_code == 200, resp.text
    turns = resp.json()["turns"]

    # Exactly one user turn — not two from double-persist
    user_turns = [t for t in turns if t.get("user_input")]
    assert len(user_turns) == 1, (
        f"Expected exactly 1 user turn, got {len(user_turns)}: {user_turns}"
    )
    assert "hello" in user_turns[0]["user_input"].lower()
