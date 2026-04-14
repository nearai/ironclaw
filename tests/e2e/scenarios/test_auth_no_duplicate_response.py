"""E2E regression test: auth_required SSE event fires without a duplicate response event.

Bug fix regression: previously, when a tool triggered auth_required, the gateway sent
BOTH an auth_required SSE event AND a response SSE event containing the same auth
instructions. This caused the web UI to render the instructions twice — once as chat
text and once inside the config card. After the fix (SubmissionResult::AuthPending),
only auth_required is emitted; no response event accompanies it.

This test:
1. Starts an ironclaw instance with a GitHub skill + mock API (returns 401 without auth)
2. Connects to the SSE stream
3. Sends a chat message that triggers the GitHub skill → HTTP 401 → auth_required
4. Collects SSE events and asserts:
   - auth_required event IS present
   - No response event contains auth instruction text (the regression)
"""

import asyncio
import json
import os
import signal
import socket
import tempfile
from pathlib import Path

import httpx
import pytest

import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import api_get, api_post, AUTH_TOKEN, sse_stream, wait_for_ready


ROOT = Path(__file__).resolve().parent.parent.parent.parent
_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-auth-sse-e2e-")
_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-auth-sse-e2e-home-")


def _forward_coverage_env(env: dict):
    for key in os.environ:
        if key.startswith(("CARGO_LLVM_COV", "LLVM_", "CARGO_ENCODED_RUSTFLAGS",
                           "CARGO_INCREMENTAL")):
            env[key] = os.environ[key]


async def _stop_process(proc, sig=signal.SIGINT, timeout=5):
    async def _drain_pipes():
        try:
            await asyncio.wait_for(proc.communicate(), timeout=1)
        except (asyncio.TimeoutError, ValueError):
            pass

    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        await _drain_pipes()
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()
    await _drain_pipes()


# ---------------------------------------------------------------------------
# Mock API: returns 401 without Bearer auth
# ---------------------------------------------------------------------------

async def _start_mock_api():
    from aiohttp import web

    async def handle_issues(request):
        auth = request.headers.get("Authorization", "")
        if not auth.startswith("Bearer "):
            return web.json_response({"message": "Bad credentials"}, status=401)
        return web.json_response([{"number": 1, "title": "Test issue", "state": "open"}])

    app = web.Application()
    app.router.add_get("/repos/{owner}/{repo}/issues", handle_issues)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = site._server.sockets[0].getsockname()[1]
    return f"http://127.0.0.1:{port}", runner


def _write_skill(skills_dir: str, mock_api_host: str):
    skill_dir = os.path.join(skills_dir, "github")
    os.makedirs(skill_dir, exist_ok=True)
    with open(os.path.join(skill_dir, "SKILL.md"), "w") as f:
        f.write(f"""---
name: github
version: "1.0.0"
keywords:
  - github
  - issues
tags:
  - github
credentials:
  - name: github_token
    provider: github
    location:
      type: bearer
    hosts:
      - "{mock_api_host}"
    setup_instructions: "Paste your GitHub personal access token below."
---
# GitHub API Skill

Use the `http` tool to call the GitHub API.
Credentials are automatically injected.
""")


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
async def mock_api():
    base_url, runner = await _start_mock_api()
    yield base_url
    await runner.cleanup()


@pytest.fixture(scope="module")
async def auth_sse_server(ironclaw_binary, mock_llm_server, mock_api):
    mock_api_host = mock_api.replace("http://", "")
    home_dir = _HOME_TMPDIR.name
    skills_dir = os.path.join(home_dir, ".ironclaw", "skills")
    os.makedirs(skills_dir, exist_ok=True)
    _write_skill(skills_dir, mock_api_host)

    # Configure mock LLM to generate tool calls to mock API
    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{mock_llm_server}/__mock/set_github_api_url",
            json={"url": mock_api},
        )
        assert r.status_code == 200

    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    gateway_port = s.getsockname()[1]
    s.close()
    s2 = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s2.bind(("127.0.0.1", 0))
    http_port = s2.getsockname()[1]
    s2.close()

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
        "GATEWAY_USER_ID": "e2e-auth-sse-tester",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_DB_TMPDIR.name, "auth-sse-e2e.db"),
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
            f"auth-sse server failed to start on port {gateway_port}.\n"
            f"stderr: {stderr_bytes.decode('utf-8', errors='replace')}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


@pytest.fixture(autouse=True)
async def _pin_mock_github_api_url(mock_llm_server, mock_api):
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/set_github_api_url",
            json={"url": mock_api},
        )
        response.raise_for_status()
    yield


# ---------------------------------------------------------------------------
# Test
# ---------------------------------------------------------------------------

async def test_auth_required_sse_without_duplicate_response(auth_sse_server):
    """When auth is triggered, SSE emits auth_required but NOT a response with instructions."""
    base_url = auth_sse_server

    # Create thread
    thread_r = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    assert thread_r.status_code == 200
    thread_id = thread_r.json()["id"]

    # Collect SSE events in background
    collected_events = []

    async def collect_sse():
        try:
            async with sse_stream(base_url, timeout=60) as resp:
                while len(collected_events) < 50:
                    raw_line = await resp.content.readline()
                    if not raw_line:
                        break
                    line = raw_line.decode("utf-8", errors="replace").rstrip("\r\n")
                    if line.startswith("data:"):
                        try:
                            data = json.loads(line[5:].strip())
                            collected_events.append(data)
                        except json.JSONDecodeError:
                            pass
        except asyncio.CancelledError:
            pass

    sse_task = asyncio.create_task(collect_sse())
    await asyncio.sleep(1)  # Let SSE connect

    # Send message that triggers github skill → HTTP 401 → auth_required
    send_r = await api_post(
        base_url,
        "/api/chat/send",
        json={
            "content": "list issues in nearai/ironclaw github repo",
            "thread_id": thread_id,
        },
        timeout=30,
    )
    assert send_r.status_code == 202

    # Wait for auth_required, then collect for a grace period to catch any
    # trailing duplicate response events that might arrive shortly after.
    deadline = asyncio.get_running_loop().time() + 45
    auth_seen_at = None
    while asyncio.get_running_loop().time() < deadline:
        event_types = [e.get("type") for e in collected_events]
        if "auth_required" in event_types and auth_seen_at is None:
            auth_seen_at = asyncio.get_running_loop().time()
        if auth_seen_at and (asyncio.get_running_loop().time() - auth_seen_at) > 3:
            break
        await asyncio.sleep(0.5)

    sse_task.cancel()
    try:
        await sse_task
    except asyncio.CancelledError:
        pass

    # Assert auth_required event was emitted
    event_types = [e.get("type") for e in collected_events]
    assert "auth_required" in event_types, (
        f"Expected auth_required in SSE events, got: {event_types}"
    )

    # Assert NO response event contains auth instruction text.
    # The auth instructions typically contain phrases like "paste your token",
    # "authentication required", or the credential setup instructions.
    auth_indicators = ["paste your token", "token below", "authentication required"]
    response_events = [
        e for e in collected_events
        if e.get("type") == "response"
    ]
    for resp_event in response_events:
        content = (resp_event.get("content") or "").lower()
        for indicator in auth_indicators:
            assert indicator not in content, (
                f"Bug regression: response SSE event contains auth instructions "
                f"('{indicator}' found in: {content[:200]}). "
                f"Auth instructions should only appear in the auth_required event, "
                f"not as a duplicate text response."
            )
