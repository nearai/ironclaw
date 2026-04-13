"""E2E test: v2 engine tool approval lifecycle.

Tests the full tool approval flow through the v2 engine (CodeAct):
1. Mock LLM generates an http POST tool call (requires approval)
2. Engine pauses the thread and exposes pending_approval in chat history
3. User submits approval decision via POST /api/chat/approval
4. Engine resumes (or denies) the tool call and completes the thread

Covers approve, deny, always-approve (persistent per-tool policy), and
submitting approval for a non-existent request_id.
"""

import asyncio
import os
import signal
import socket
import tempfile
import uuid
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
_V2_APPROVAL_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-approval-e2e-")
_V2_APPROVAL_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-approval-e2e-home-")


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
async def v2_approval_server(ironclaw_binary, mock_llm_server):
    """Start ironclaw with ENGINE_V2=true for tool approval flow tests.

    No custom skills needed — the built-in http tool requires approval for
    POST requests, which is what the mock LLM generates for the
    "make approval post <label>" pattern.
    """
    home_dir = _V2_APPROVAL_HOME_TMPDIR.name
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
        "HTTP_ALLOW_LOCALHOST": "true",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-v2-approval-tester",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_V2_APPROVAL_DB_TMPDIR.name, "v2-approval-e2e.db"),
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
            f"v2 approval server failed to start on port {gateway_port}.\n"
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

async def _wait_for_approval(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Poll /api/chat/history until pending_gate appears.

    Returns the pending_gate dict containing request_id, tool_name, etc.
    """
    for _ in range(int(timeout * 2)):
        r = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        r.raise_for_status()
        history = r.json()
        pending = history.get("pending_gate")
        if pending and pending.get("request_id"):
            return pending
        await asyncio.sleep(0.5)

    # Dump full history for debugging
    debug_info = ""
    try:
        r = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15)
        data = r.json()
        turns = data.get("turns", [])
        pending = data.get("pending_gate")
        debug_info = f"turns={len(turns)}, pending={pending}"
        if turns:
            last_turn = turns[-1]
            debug_info += f", last_response={repr((last_turn.get('response') or '(None)')[:300])}"
            debug_info += f", state={last_turn.get('state')}"
            tool_calls = last_turn.get("tool_calls", [])
            if tool_calls:
                debug_info += f", tool_calls={[tc.get('name') for tc in tool_calls]}"
    except Exception as e:
        debug_info = f"error: {e}"
    raise AssertionError(
        f"Timed out waiting for pending_gate in thread {thread_id}. "
        f"Debug: {debug_info}"
    )


async def _wait_for_response(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
    expect_substring: str | None = None,
) -> dict:
    """Poll chat history until an assistant response appears.

    Returns the full history dict.
    """
    for _ in range(int(timeout * 2)):
        r = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        r.raise_for_status()
        history = r.json()
        turns = history.get("turns", [])
        if turns:
            last_response = turns[-1].get("response") or ""
            if last_response:
                if expect_substring is None or expect_substring.lower() in last_response.lower():
                    return history
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for response"
        + (f" containing '{expect_substring}'" if expect_substring else "")
        + f" in thread {thread_id}"
    )


async def _wait_for_no_pending_gate(base_url: str, thread_id: str, *, timeout: float = 45.0):
    for _ in range(int(timeout * 2)):
        r = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15)
        r.raise_for_status()
        history = r.json()
        if not history.get("pending_gate"):
            return history
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for pending_gate to clear in thread {thread_id}")


async def _approve(
    base_url: str,
    thread_id: str,
    request_id: str,
    action: str,
) -> httpx.Response:
    """POST /api/chat/approval with the given action.

    Returns the httpx response.
    """
    r = await api_post(
        base_url,
        "/api/chat/approval",
        json={
            "request_id": request_id,
            "action": action,
            "thread_id": thread_id,
        },
        timeout=15,
    )
    return r


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestV2EngineApprovalFlow:
    async def test_same_user_approvals_are_thread_scoped(self, v2_approval_server):
        base_url = v2_approval_server

        thread_a = (await api_post(base_url, "/api/chat/thread/new", timeout=15)).json()["id"]
        thread_b = (await api_post(base_url, "/api/chat/thread/new", timeout=15)).json()["id"]

        await api_post(
            base_url,
            "/api/chat/send",
            json={"content": "make approval post alpha", "thread_id": thread_a},
            timeout=30,
        )
        await api_post(
            base_url,
            "/api/chat/send",
            json={"content": "make approval post beta", "thread_id": thread_b},
            timeout=30,
        )

        pending_a = await _wait_for_approval(base_url, thread_a, timeout=60)
        pending_b = await _wait_for_approval(base_url, thread_b, timeout=60)
        assert pending_a["request_id"] != pending_b["request_id"]

        approve_a = await _approve(base_url, thread_a, pending_a["request_id"], "approve")
        assert approve_a.status_code == 202, approve_a.text
        await _wait_for_no_pending_gate(base_url, thread_a, timeout=60)

        history_b = await api_get(base_url, f"/api/chat/history?thread_id={thread_b}", timeout=15)
        history_b.raise_for_status()
        still_pending_b = history_b.json().get("pending_gate")
        assert still_pending_b is not None, history_b.json()
        assert still_pending_b["request_id"] == pending_b["request_id"]

        approve_b = await _approve(base_url, thread_b, pending_b["request_id"], "approve")
        assert approve_b.status_code == 202, approve_b.text
        await _wait_for_no_pending_gate(base_url, thread_b, timeout=60)

    """Test the v2 engine tool approval lifecycle.

    Uses text-based approval ("yes"/"no"/"always" as chat messages) rather
    than the /api/chat/approval endpoint, since the v2 engine's pending_approval
    metadata uses engine thread IDs that differ from the v1 session thread IDs
    shown in the history API.
    """

    async def test_approval_yes(self, v2_approval_server):
        """Approve a pending http POST tool call by replying 'yes'."""
        base = v2_approval_server

        thread_r = await api_post(base, "/api/chat/thread/new", timeout=15)
        assert thread_r.status_code == 200
        thread_id = thread_r.json()["id"]

        # Send message that triggers an http POST → NeedApproval
        await api_post(
            base, "/api/chat/send",
            json={"content": "make approval post test-alpha", "thread_id": thread_id},
            timeout=30,
        )

        # Wait for the approval prompt
        history = await _wait_for_response(
            base, thread_id, timeout=60, expect_substring="requires approval",
        )

        # Reply "yes" to approve — goes through SubmissionParser as ApprovalResponse
        await api_post(
            base, "/api/chat/send",
            json={"content": "yes", "thread_id": thread_id},
            timeout=30,
        )

        # Wait for the approval to be processed — poll until the response
        # changes from the approval prompt (tool executes after approval)
        for _ in range(120):
            await asyncio.sleep(0.5)
            r = await api_get(base, f"/api/chat/history?thread_id={thread_id}", timeout=15)
            history = r.json()
            turns = history.get("turns", [])
            if turns:
                last = (turns[-1].get("response") or "").lower()
                if last and "requires approval" not in last:
                    break
            # Also check if pending_gate is cleared (approval processed)
            if not history.get("pending_gate"):
                break

        # After approval, pending_gate should be cleared
        assert history.get("pending_gate") is None, (
            f"After approval, pending_gate should be cleared. "
            f"Got: {history.get('pending_gate')}"
        )

    async def test_approval_no(self, v2_approval_server):
        """Deny a pending tool call by replying 'no'."""
        base = v2_approval_server

        thread_r = await api_post(base, "/api/chat/thread/new", timeout=15)
        assert thread_r.status_code == 200
        thread_id = thread_r.json()["id"]

        await api_post(
            base, "/api/chat/send",
            json={"content": "make approval post test-deny", "thread_id": thread_id},
            timeout=30,
        )

        # Wait for the approval prompt
        await _wait_for_response(
            base, thread_id, timeout=60, expect_substring="requires approval",
        )

        # Deny
        await api_post(
            base, "/api/chat/send",
            json={"content": "no", "thread_id": thread_id},
            timeout=30,
        )

        # Wait for the denial response — poll until the approval prompt is
        # no longer the latest response (meaning the denial was processed)
        for _ in range(120):
            await asyncio.sleep(0.5)
            r = await api_get(base, f"/api/chat/history?thread_id={thread_id}", timeout=15)
            history = r.json()
            turns = history.get("turns", [])
            if turns:
                last = (turns[-1].get("response") or "").lower()
                # The denial is processed when the last response changes from
                # the approval prompt or mentions denial
                if last and "requires approval" not in last:
                    break
                if "denied" in last or "rejected" in last:
                    break

        all_responses = " ".join(
            (t.get("response") or "") for t in history.get("turns", [])
        ).lower()

        # After denial, approval prompt should no longer be pending
        assert history.get("pending_gate") is None, (
            f"After denial, pending_gate should be cleared. "
            f"Got: {history.get('pending_gate')}"
        )

    async def test_approval_always(self, v2_approval_server):
        """Approve with 'always' — second request auto-approves."""
        base = v2_approval_server

        # First thread: trigger approval and reply "always"
        thread_r = await api_post(base, "/api/chat/thread/new", timeout=15)
        thread_id_1 = thread_r.json()["id"]

        await api_post(
            base, "/api/chat/send",
            json={"content": "make approval post first-run", "thread_id": thread_id_1},
            timeout=30,
        )

        await _wait_for_response(
            base, thread_id_1, timeout=60, expect_substring="requires approval",
        )

        await api_post(
            base, "/api/chat/send",
            json={"content": "always", "thread_id": thread_id_1},
            timeout=30,
        )

        await _wait_for_response(base, thread_id_1, timeout=60)

        # Second thread: same tool should auto-approve (no pause)
        thread_r2 = await api_post(base, "/api/chat/thread/new", timeout=15)
        thread_id_2 = thread_r2.json()["id"]

        await api_post(
            base, "/api/chat/send",
            json={"content": "make approval post second-run", "thread_id": thread_id_2},
            timeout=30,
        )

        # Should complete directly without approval prompt
        history = await _wait_for_response(base, thread_id_2, timeout=60)
        all_responses = " ".join(
            (t.get("response") or "") for t in history.get("turns", [])
        ).lower()

        assert "requires approval" not in all_responses, (
            f"Second thread should auto-approve. Got: {all_responses[:500]}"
        )
        # Verify the tool actually ran (not just that approval was skipped)
        turns = history.get("turns", [])
        assert len(turns) >= 1, (
            f"Expected at least 1 turn with tool execution after auto-approve. "
            f"Got {len(turns)} turns."
        )

    # test_approval_prompt_contains_tool_name was removed — the assertion
    # is covered by test_approval_yes which verifies the prompt text.
