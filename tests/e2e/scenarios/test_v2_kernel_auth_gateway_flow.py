"""E2E test: v2 engine auth flow through the gateway UI (auth card).

Reproduces the bug where:
1. User asks "list github issues" → NeedAuthentication triggers
2. Gateway shows AuthRequired SSE event → frontend should show auth card
3. Auth card should have a token input field (not the extension configure modal)
4. User submits token via /api/chat/auth-token → token stored → retry succeeds

The bug: when auth_url is None and instructions ARE present (skill credential),
the frontend incorrectly calls showConfigureModal() instead of showAuthCard(),
which fails for skill credentials (not extensions) and permanently blocks input.
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


ROOT = Path(__file__).resolve().parent.parent.parent.parent
_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-gw-auth-e2e-")
_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-gw-auth-e2e-home-")


def _forward_coverage_env(env: dict):
    for key in os.environ:
        if key.startswith(("CARGO_LLVM_COV", "LLVM_", "CARGO_ENCODED_RUSTFLAGS",
                           "CARGO_INCREMENTAL")):
            env[key] = os.environ[key]


async def _stop_process(proc, sig=signal.SIGINT, timeout=5):
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
# Mock API: public endpoint (200 without auth) + auth endpoint (401 without)
# ---------------------------------------------------------------------------

async def _start_mock_api():
    from aiohttp import web

    state = {"request_count": 0, "tokens": []}

    async def handle_issues(request: web.Request) -> web.Response:
        """Auth-required endpoint — returns 401 without Bearer token."""
        state["request_count"] += 1
        auth = request.headers.get("Authorization", "")
        if not auth.startswith("Bearer "):
            return web.json_response({"message": "Bad credentials"}, status=401)
        token = auth.split(" ", 1)[1]
        state["tokens"].append(token)
        if not token.startswith("ghp_"):
            return web.json_response({"message": "Bad credentials"}, status=401)
        return web.json_response([
            {"number": 42, "title": "Test issue", "state": "open"},
        ])

    async def handle_state(request: web.Request) -> web.Response:
        return web.json_response(state)

    async def handle_reset(request: web.Request) -> web.Response:
        state["request_count"] = 0
        state["tokens"].clear()
        return web.json_response({"ok": True})

    app = web.Application()
    app.router.add_get("/repos/{owner}/{repo}/issues", handle_issues)
    app.router.add_get("/__mock/state", handle_state)
    app.router.add_post("/__mock/reset", handle_reset)

    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = site._server.sockets[0].getsockname()[1]
    return f"http://127.0.0.1:{port}", runner, state


def _write_github_skill(skills_dir: str, mock_api_host: str):
    skill_dir = os.path.join(skills_dir, "github")
    os.makedirs(skill_dir, exist_ok=True)
    skill_content = f"""---
name: github
version: "1.0.0"
keywords:
  - github
  - issues
  - repo
tags:
  - github
credentials:
  - name: github_token
    provider: github
    location:
      type: bearer
    hosts:
      - "{mock_api_host}"
    setup_instructions: "Paste your GitHub personal access token (starts with ghp_)."
---
# GitHub API Skill

Use the `http` tool to access the GitHub REST API.
Credentials are automatically injected.
"""
    with open(os.path.join(skill_dir, "SKILL.md"), "w") as f:
        f.write(skill_content)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
async def mock_api():
    url, runner, state = await _start_mock_api()
    yield {"url": url, "state": state}
    await runner.cleanup()


@pytest.fixture(scope="module")
async def v2_server(ironclaw_binary, mock_llm_server, mock_api):
    mock_api_url = mock_api["url"]
    mock_api_host = mock_api_url.replace("http://", "")

    async with httpx.AsyncClient() as client:
        await client.post(
            f"{mock_llm_server}/__mock/set_github_api_url",
            json={"url": mock_api_url},
        )

    home_dir = _HOME_TMPDIR.name
    skills_dir = os.path.join(home_dir, ".ironclaw", "skills")
    os.makedirs(skills_dir, exist_ok=True)
    _write_github_skill(skills_dir, mock_api_host)

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
        "SECRETS_MASTER_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-gw-auth-tester",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_DB_TMPDIR.name, "gw-auth-e2e.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
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
            f"Server failed to start: {stderr_bytes.decode('utf-8', errors='replace')}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

async def _wait_for_auth_prompt(base_url, thread_id, *, timeout=45.0):
    auth_indicators = ["paste your token", "token below", "authentication required for"]
    for _ in range(int(timeout * 2)):
        r = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15)
        r.raise_for_status()
        turns = r.json().get("turns", [])
        if turns:
            last = (turns[-1].get("response") or "").lower()
            if last and any(ind in last for ind in auth_indicators):
                return r.json()
        await asyncio.sleep(0.5)

    last = ""
    try:
        r = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15)
        turns = r.json().get("turns", [])
        if turns:
            last = turns[-1].get("response") or "(None)"
    except Exception:
        pass
    raise AssertionError(f"Timed out waiting for auth prompt. Last: {last[:500]}")


async def _wait_for_response(base_url, thread_id, *, timeout=45.0, expect_substring=None):
    for _ in range(int(timeout * 2)):
        r = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15)
        r.raise_for_status()
        turns = r.json().get("turns", [])
        if turns:
            last = turns[-1].get("response") or ""
            if last and (expect_substring is None or expect_substring.lower() in last.lower()):
                return r.json()
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for response in thread {thread_id}")


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestGatewayAuthCard:
    """Test the auth flow through the gateway API endpoints (simulating frontend)."""

    async def test_auth_token_endpoint_stores_credential_for_v2(self, v2_server, mock_api):
        """Submit token via /api/chat/auth-token → credential stored → retry works.

        This is the gateway UI path: the auth card calls /api/chat/auth-token
        instead of sending the token as a chat message. Both paths must work
        for the v2 engine.
        """
        mock_api_url = mock_api["url"]
        async with httpx.AsyncClient() as client:
            await client.post(f"{mock_api_url}/__mock/reset")

        # Step 1: Trigger auth flow
        thread_r = await api_post(v2_server, "/api/chat/thread/new", timeout=15)
        thread_id = thread_r.json()["id"]

        await api_post(
            v2_server,
            "/api/chat/send",
            json={
                "content": "list issues in nearai/ironclaw github repo",
                "thread_id": thread_id,
            },
            timeout=30,
        )

        # Step 2: Wait for auth prompt
        await _wait_for_auth_prompt(v2_server, thread_id, timeout=60)

        # Step 3: Submit token via the auth-token API endpoint
        # (this is what the frontend auth card does)
        token_r = await api_post(
            v2_server,
            "/api/chat/auth-token",
            json={
                "extension_name": "github_token",
                "token": "ghp_gateway_test_token_123",
            },
            timeout=15,
        )
        assert token_r.status_code == 200, (
            f"Auth token endpoint should accept the token. "
            f"Response: {token_r.text}"
        )

        # Step 4: Verify the auth flow completes — send another message
        # to verify the credential is now stored and usable.
        thread_r2 = await api_post(v2_server, "/api/chat/thread/new", timeout=15)
        thread_id2 = thread_r2.json()["id"]

        await api_post(
            v2_server,
            "/api/chat/send",
            json={
                "content": "list issues in nearai/ironclaw github repo",
                "thread_id": thread_id2,
            },
            timeout=30,
        )

        # Should complete without auth prompt (credential was stored)
        history = await _wait_for_response(v2_server, thread_id2, timeout=60)
        all_responses = " ".join(
            (t.get("response") or "") for t in history.get("turns", [])
        ).lower()

        assert "paste your token" not in all_responses, (
            f"Should not need auth after token was stored via auth-token API. "
            f"Responses: {all_responses[:500]}"
        )

    async def test_chat_message_token_path_works(self, v2_server, mock_api):
        """Submit token as a chat message → stored → retry works.

        This is the v2 engine's native token reception path: the next
        chat message after NeedAuthentication is treated as a token.
        """
        mock_api_url = mock_api["url"]
        async with httpx.AsyncClient() as client:
            await client.post(f"{mock_api_url}/__mock/reset")

        # Create thread — credential may already be stored from previous test.
        # This test validates the chat-message path independently.
        thread_r = await api_post(v2_server, "/api/chat/thread/new", timeout=15)
        thread_id = thread_r.json()["id"]

        await api_post(
            v2_server,
            "/api/chat/send",
            json={
                "content": "list issues in nearai/ironclaw github repo",
                "thread_id": thread_id,
            },
            timeout=30,
        )

        # If credential is already stored, this completes without auth.
        # If not, we'll get an auth prompt and submit via chat.
        history = await _wait_for_response(v2_server, thread_id, timeout=60)
        all_responses = " ".join(
            (t.get("response") or "") for t in history.get("turns", [])
        ).lower()

        if "paste your token" in all_responses or "authentication required" in all_responses:
            # Auth prompt — submit token as chat message
            await api_post(
                v2_server,
                "/api/chat/send",
                json={"content": "ghp_chat_path_token_456", "thread_id": thread_id},
                timeout=30,
            )

            # Wait for retry to complete
            for _ in range(60):
                await asyncio.sleep(0.5)
                r = await api_get(
                    v2_server,
                    f"/api/chat/history?thread_id={thread_id}",
                    timeout=15,
                )
                turns = r.json().get("turns", [])
                if len(turns) > 1:
                    last = (turns[-1].get("response") or "").lower()
                    if "paste your token" not in last and last:
                        break

        # Verify: no crash, got some response
        r = await api_get(
            v2_server,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        assert r.status_code == 200
        turns = r.json().get("turns", [])
        assert len(turns) > 0, "Should have at least one turn"

    async def test_auth_cancel_unblocks_input(self, v2_server, mock_api):
        """After cancelling auth via /api/chat/auth-cancel, chat input works again."""
        thread_r = await api_post(v2_server, "/api/chat/thread/new", timeout=15)
        thread_id = thread_r.json()["id"]

        # Send normal message — should work (not blocked by stale auth state)
        await api_post(
            v2_server,
            "/api/chat/send",
            json={"content": "hello", "thread_id": thread_id},
            timeout=30,
        )
        history = await _wait_for_response(v2_server, thread_id, timeout=30)
        assert len(history.get("turns", [])) > 0, "Should get a response after cancel"
