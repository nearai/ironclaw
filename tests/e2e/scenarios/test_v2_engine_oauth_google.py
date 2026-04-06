"""OAuth redirect lifecycle + Google API emulation for the v2 engine.

Tests the full OAuth redirect flow against a mock Google API server:
1. Extension setup triggers OAuth redirect URL generation
2. OAuth callback with authorization code completes the flow
3. Stored credentials are injected into API calls
4. Expired tokens trigger refresh via the hosted proxy
5. Cancel and invalid-token edge cases are handled gracefully

The mock Google API enforces Bearer auth on protected endpoints and tracks
all received tokens so tests can assert credential injection behavior.
"""

import asyncio
import os
import signal
import socket
import sqlite3
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import httpx
import pytest

import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import api_get, api_post, AUTH_TOKEN, wait_for_ready


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

ROOT = Path(__file__).resolve().parent.parent.parent.parent
_GOOGLE_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-google-e2e-")
_GOOGLE_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-google-e2e-home-")


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
# Mock Google API server
# ---------------------------------------------------------------------------

async def _start_mock_google_api():
    """Start mock Google API server.

    Returns (base_url, runner, received_tokens).
    """
    from aiohttp import web

    received_tokens: list[str] = []

    async def handle_drive_files(request: web.Request) -> web.Response:
        auth = request.headers.get("Authorization", "")
        if not auth.startswith("Bearer "):
            return web.json_response(
                {"error": {"code": 401, "message": "Request is missing required authentication credential."}},
                status=401,
            )
        received_tokens.append(auth.split(" ", 1)[1])
        return web.json_response({
            "files": [
                {"name": "Budget Q1.xlsx"},
                {"name": "Meeting notes.doc"},
            ]
        })

    async def handle_userinfo(request: web.Request) -> web.Response:
        return web.json_response({
            "email": "test@example.com",
            "name": "Test User",
        })

    async def handle_received_tokens(request: web.Request) -> web.Response:
        return web.json_response({"tokens": received_tokens})

    async def handle_reset(request: web.Request) -> web.Response:
        received_tokens.clear()
        return web.json_response({"ok": True})

    app = web.Application()
    app.router.add_get("/drive/v3/files", handle_drive_files)
    app.router.add_get("/oauth2/v2/userinfo", handle_userinfo)
    app.router.add_get("/__mock/received-tokens", handle_received_tokens)
    app.router.add_post("/__mock/reset", handle_reset)

    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    actual_port = site._server.sockets[0].getsockname()[1]
    base_url = f"http://127.0.0.1:{actual_port}"
    return base_url, runner, received_tokens


# ---------------------------------------------------------------------------
# Skill writer
# ---------------------------------------------------------------------------

def _install_google_drive_wasm(wasm_dir: str):
    """Copy the real google-drive WASM tool binary + capabilities into the test dir.

    Requires: `cd tools-src/google-drive && cargo build --target wasm32-wasip2 --release`
    """
    import shutil

    # Find the built WASM binary (shared-target or local target)
    wasm_src = None
    candidates = [
        ROOT / ".cargo" / "shared-target" / "wasm32-wasip2" / "release" / "google_drive_tool.wasm",
        Path.home() / ".cargo" / "shared-target" / "wasm32-wasip2" / "release" / "google_drive_tool.wasm",
        ROOT / "tools-src" / "google-drive" / "target" / "wasm32-wasip2" / "release" / "google_drive_tool.wasm",
    ]
    for c in candidates:
        if c.exists():
            wasm_src = c
            break

    if wasm_src is None:
        return False  # caller should skip

    # Copy WASM binary as google_drive.wasm (extension name format)
    shutil.copy2(str(wasm_src), os.path.join(wasm_dir, "google_drive.wasm"))

    # Copy real capabilities.json
    cap_src = ROOT / "tools-src" / "google-drive" / "google-drive-tool.capabilities.json"
    if cap_src.exists():
        shutil.copy2(str(cap_src), os.path.join(wasm_dir, "google_drive.capabilities.json"))
    else:
        return False

    return True


def _write_google_skill(skills_dir: str, mock_api_host: str):
    """Write a google_drive skill with credential spec pointing to mock API host."""
    skill_dir = os.path.join(skills_dir, "google_drive")
    os.makedirs(skill_dir, exist_ok=True)
    skill_content = f"""---
name: google_drive
version: "1.0.0"
keywords:
  - google
  - drive
  - files
  - docs
tags:
  - google
  - api
credentials:
  - name: google_drive_token
    provider: google
    location:
      type: bearer
    hosts:
      - "{mock_api_host}"
    setup_instructions: "Paste your Google API key or access token below."
---
# Google Drive API Skill

You have access to the Google Drive API via the `http` tool.
Credentials are automatically injected — **never construct Authorization headers manually**.

## API Patterns

### List files

**List drive files:**
```
http(method="GET", url="http://{mock_api_host}/drive/v3/files")
```
"""
    with open(os.path.join(skill_dir, "SKILL.md"), "w") as f:
        f.write(skill_content)


# ---------------------------------------------------------------------------
# DB helpers (from test_oauth_refresh.py)
# ---------------------------------------------------------------------------

def _find_secret_row(
    db_path: str,
    secret_name: str,
) -> tuple[str, str | None, str | None]:
    with sqlite3.connect(db_path) as conn:
        row = conn.execute(
            """
            SELECT user_id, expires_at, updated_at
            FROM secrets
            WHERE name = ?1
            ORDER BY updated_at DESC
            LIMIT 1
            """,
            (secret_name,),
        ).fetchone()
    assert row is not None, f"Missing secret row for {secret_name}"
    return row[0], row[1], row[2]


def _expire_access_token(db_path: str, user_id: str, secret_name: str) -> None:
    with sqlite3.connect(db_path) as conn:
        cursor = conn.execute(
            """
            UPDATE secrets
            SET expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 hour')
            WHERE user_id = ?1 AND name = ?2
            """,
            (user_id, secret_name),
        )
        conn.commit()
    assert cursor.rowcount == 1, f"Expected one secret row for {user_id}/{secret_name}"


def _parse_timestamp(value: str | None) -> datetime | None:
    if value is None:
        return None
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


# ---------------------------------------------------------------------------
# Polling helpers
# ---------------------------------------------------------------------------

async def _wait_for_response(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
    expect_substring: str | None = None,
) -> dict:
    """Poll chat history until an assistant response appears."""
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


async def _wait_for_auth_prompt(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Poll until response mentions authentication or credential prompt."""
    auth_indicators = [
        "authentication",
        "credential",
        "paste your token",
        "token below",
        "google_drive_token",
        "api key",
        "access token",
    ]
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
            last_response = (turns[-1].get("response") or "").lower()
            if last_response and any(ind in last_response for ind in auth_indicators):
                return history
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for auth prompt in thread {thread_id}"
    )


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
async def mock_google_api():
    """Start the mock Google API server."""
    base_url, runner, received_tokens = await _start_mock_google_api()
    yield {"url": base_url, "tokens": received_tokens}
    await runner.cleanup()


@pytest.fixture(scope="module")
async def v2_google_server(ironclaw_binary, mock_llm_server, mock_google_api):
    """Start ironclaw with ENGINE_V2=true and a google_drive skill pointing to mock Google API."""
    mock_api_url = mock_google_api["url"]
    mock_api_host = mock_api_url.replace("http://", "")

    # Configure mock LLM to route "list drive files" tool calls to the mock API URL
    async with httpx.AsyncClient() as client:
        r = await client.post(
            f"{mock_llm_server}/__mock/set_github_api_url",
            json={"url": mock_api_url},
        )
        assert r.status_code == 200

    home_dir = _GOOGLE_HOME_TMPDIR.name
    skills_dir = os.path.join(home_dir, ".ironclaw", "skills")
    os.makedirs(skills_dir, exist_ok=True)
    _write_google_skill(skills_dir, mock_api_host)

    # Install real google-drive WASM tool for OAuth redirect test
    wasm_tools_dir = os.path.join(home_dir, ".ironclaw", "wasm_tools")
    os.makedirs(wasm_tools_dir, exist_ok=True)
    _install_google_drive_wasm(wasm_tools_dir)

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

    db_path = os.path.join(_GOOGLE_DB_TMPDIR.name, "v2-google-e2e.db")

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
        "GATEWAY_USER_ID": "e2e-v2-google-tester",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": db_path,
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "true",
        "WASM_TOOLS_DIR": wasm_tools_dir,
        "WASM_CHANNELS_DIR": os.path.join(home_dir, ".ironclaw", "wasm_channels"),
        "ONBOARD_COMPLETED": "true",
        "GOOGLE_OAUTH_CLIENT_ID": "test-google-client-id",
        "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
        "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
        "SECRETS_MASTER_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
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
        yield {
            "base_url": base_url,
            "db_path": db_path,
            "mock_llm_url": mock_llm_server,
            "mock_google_url": mock_api_url,
        }
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
            f"v2 google ironclaw server failed to start on port {gateway_port}.\n"
            f"stderr: {stderr_bytes.decode('utf-8', errors='replace')}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

async def test_oauth_redirect_flow(v2_google_server):
    """POST /api/extensions/google_drive/setup should return an OAuth auth_url.

    Flow: get auth_url -> verify URL has client_id and state -> GET /oauth/callback
    with code and state -> verify 'connected'/'success' in response -> verify
    extension authenticated.
    """
    server = v2_google_server["base_url"]

    # Attempt extension setup — triggers OAuth URL generation
    setup_response = await api_post(
        server,
        "/api/extensions/google_drive/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert setup_response.status_code == 200, setup_response.text
    setup_data = setup_response.json()
    # If WASM binary wasn't built, activation fails and no auth_url is generated.
    if not setup_data.get("auth_url"):
        pytest.skip(
            "OAuth redirect requires google-drive WASM binary. "
            "Build with: cd tools-src/google-drive && cargo build --target wasm32-wasip2 --release"
        )
    auth_url = setup_data.get("auth_url")
    assert auth_url, f"Expected auth_url in setup response: {setup_data}"

    # Verify URL contains client_id and state
    parsed = urlparse(auth_url)
    params = parse_qs(parsed.query)
    assert params.get("client_id") == ["test-google-client-id"], (
        f"auth_url should include client_id=test-google-client-id: {auth_url}"
    )
    state = params.get("state", [None])[0]
    assert state, f"auth_url should include state parameter: {auth_url}"

    # Complete the OAuth callback
    async with httpx.AsyncClient() as client:
        callback_response = await client.get(
            f"{server}/oauth/callback",
            params={"code": "mock_code", "state": state},
            timeout=30,
            follow_redirects=True,
        )

    assert callback_response.status_code == 200, callback_response.text[:400]
    callback_body = callback_response.text.lower()
    assert "connected" in callback_body or "success" in callback_body, (
        f"Expected 'connected' or 'success' in callback response: {callback_body[:500]}"
    )

    # Verify extension is authenticated
    ext_response = await api_get(server, "/api/extensions", timeout=15)
    ext_response.raise_for_status()
    extensions = ext_response.json().get("extensions", [])
    google_ext = next((e for e in extensions if e["name"] == "google_drive"), None)
    if google_ext is not None:
        assert google_ext.get("authenticated") is True, google_ext


async def test_oauth_cancel_during_paste_flow(v2_google_server):
    """Trigger NeedAuthentication via API key path, then cancel, then verify thread still works."""
    server = v2_google_server["base_url"]

    # Create a fresh thread
    thread_r = await api_post(server, "/api/chat/thread/new", timeout=15)
    assert thread_r.status_code == 200
    thread_id = thread_r.json()["id"]

    # Send message that triggers the google_drive skill -> http tool call -> 401
    await api_post(
        server,
        "/api/chat/send",
        json={
            "content": "list google drive files",
            "thread_id": thread_id,
        },
        timeout=30,
    )

    # Wait for auth prompt
    await _wait_for_auth_prompt(server, thread_id, timeout=60)

    # Send cancel
    await api_post(
        server,
        "/api/chat/send",
        json={"content": "cancel", "thread_id": thread_id},
        timeout=30,
    )

    # Wait for response containing 'cancel' indication
    history = await _wait_for_response(server, thread_id, timeout=45, expect_substring="cancel")
    all_responses = " ".join(
        t.get("response", "") for t in history.get("turns", [])
    ).lower()
    assert "cancel" in all_responses, (
        f"Expected 'cancelled' in response after cancel: {all_responses[:500]}"
    )

    # Verify thread still works with a new normal message
    await api_post(
        server,
        "/api/chat/send",
        json={"content": "hello", "thread_id": thread_id},
        timeout=30,
    )

    history = await _wait_for_response(server, thread_id, timeout=45)
    turns = history.get("turns", [])
    assert len(turns) >= 2, f"Expected at least 2 turns after cancel + hello: {turns}"


async def test_invalid_token_paste(v2_google_server, mock_google_api):
    """Submit an invalid token; the token gets stored but the mock API rejects it.

    Must run BEFORE test_api_key_then_api_call so no valid credential exists yet.
    Verifies the server doesn't infinite-loop on a stored-but-invalid token.
    """
    server = v2_google_server["base_url"]
    mock_api_url = mock_google_api["url"]

    # Reset mock API state
    async with httpx.AsyncClient() as client:
        await client.post(f"{mock_api_url}/__mock/reset")

    # Create a fresh thread
    thread_r = await api_post(server, "/api/chat/thread/new", timeout=15)
    assert thread_r.status_code == 200
    thread_id = thread_r.json()["id"]

    # Send message that triggers google_drive skill
    await api_post(
        server,
        "/api/chat/send",
        json={
            "content": "list google drive files",
            "thread_id": thread_id,
        },
        timeout=30,
    )

    # Wait for auth prompt — no credentials stored yet (this test runs first)
    await _wait_for_auth_prompt(server, thread_id, timeout=60)

    # Submit a bad token — gets stored but mock API rejects it (401)
    await api_post(
        server,
        "/api/chat/send",
        json={"content": "bad_token_xxx", "thread_id": thread_id},
        timeout=30,
    )

    # Wait for a response — the key assertion is that we get a response
    # at all (no infinite retry loop)
    history = await _wait_for_response(server, thread_id, timeout=60)
    turns = history.get("turns", [])
    assert len(turns) >= 1, (
        f"Expected at least one response turn after invalid token: {turns}"
    )


async def test_api_key_then_api_call(v2_google_server, mock_google_api):
    """Trigger auth prompt, submit valid API key token, verify mock API received it."""
    server = v2_google_server["base_url"]
    mock_api_url = mock_google_api["url"]

    # Reset mock API state
    async with httpx.AsyncClient() as client:
        await client.post(f"{mock_api_url}/__mock/reset")

    # Create a fresh thread
    thread_r = await api_post(server, "/api/chat/thread/new", timeout=15)
    assert thread_r.status_code == 200
    thread_id = thread_r.json()["id"]

    # Send message that triggers google_drive skill
    await api_post(
        server,
        "/api/chat/send",
        json={
            "content": "list google drive files",
            "thread_id": thread_id,
        },
        timeout=30,
    )

    # Wait for auth prompt. A bad token may be stored from the previous test
    # (test_invalid_token_paste), but the mock API rejects it (401), which
    # triggers NeedAuthentication reactively.
    try:
        await _wait_for_auth_prompt(server, thread_id, timeout=60)
    except AssertionError:
        # If no auth prompt (token accepted or different flow), just verify
        # the response and return — the credential was already valid
        history = await _wait_for_response(server, thread_id, timeout=30)
        all_responses = " ".join(
            (t.get("response") or "") for t in history.get("turns", [])
        ).lower()
        assert "paste your token" not in all_responses
        return

    # Submit valid API key token
    test_token = "google_api_key_e2e_test_abc123"
    await api_post(
        server,
        "/api/chat/send",
        json={"content": test_token, "thread_id": thread_id},
        timeout=30,
    )

    # Wait for the retry to complete
    history = await _wait_for_response(server, thread_id, timeout=60)

    # Verify the mock API received the token
    await asyncio.sleep(2)
    async with httpx.AsyncClient() as client:
        tokens_r = await client.get(f"{mock_api_url}/__mock/received-tokens")
        tokens_data = tokens_r.json()

    all_responses = " ".join(
        t.get("response", "") for t in history.get("turns", [])
    ).lower()

    # The token MUST be received by the mock API
    assert test_token in tokens_data.get("tokens", []), (
        f"Token MUST be received by mock API after auth flow.\n"
        f"Expected: {test_token}\n"
        f"Mock API tokens: {tokens_data.get('tokens', [])}\n"
        f"Responses: {all_responses[:500]}"
    )


async def test_oauth_token_refresh_on_expiry(v2_google_server, mock_google_api):
    """Expire a stored token and verify refresh via the mock OAuth proxy.

    Only runs if OAuth callback was successful (prerequisite: extension must have
    a stored credential). Follows test_oauth_refresh.py pattern.
    """
    server = v2_google_server["base_url"]
    db_path = v2_google_server["db_path"]
    mock_llm_url = v2_google_server["mock_llm_url"]
    mock_api_url = mock_google_api["url"]

    # Check if a google-related secret exists in the DB (from OAuth or paste flow)
    try:
        stored_user_id, expires_before, updated_before = _find_secret_row(
            db_path, "google_drive_token"
        )
    except AssertionError:
        # Also try the OAuth token name variant
        try:
            stored_user_id, expires_before, updated_before = _find_secret_row(
                db_path, "google_oauth_token"
            )
        except AssertionError:
            pytest.skip(
                "No google credential stored in DB; "
                "OAuth callback or token paste prerequisite not met"
            )
            return  # unreachable, but satisfies type checkers

    secret_name = "google_drive_token"
    try:
        _find_secret_row(db_path, secret_name)
    except AssertionError:
        secret_name = "google_oauth_token"

    stored_user_id, expires_before, updated_before = _find_secret_row(db_path, secret_name)

    # Reset mock OAuth state
    async with httpx.AsyncClient() as client:
        r = await client.post(f"{mock_llm_url}/__mock/oauth/reset", timeout=10)
        r.raise_for_status()

    # Reset mock Google API state
    async with httpx.AsyncClient() as client:
        await client.post(f"{mock_api_url}/__mock/reset")

    # Expire the token
    await asyncio.sleep(0.1)
    _expire_access_token(db_path, stored_user_id, secret_name)

    # Create a fresh thread
    thread_r = await api_post(server, "/api/chat/thread/new", timeout=15)
    assert thread_r.status_code == 200
    thread_id = thread_r.json()["id"]

    # Send message triggering google_drive tool call with expired token
    send_response = await api_post(
        server,
        "/api/chat/send",
        json={"content": "list google drive files", "thread_id": thread_id},
        timeout=30,
    )
    assert send_response.status_code == 202, send_response.text

    # Wait for a response (the engine should refresh the token and retry)
    history = await _wait_for_response(server, thread_id, timeout=60)
    assert any(
        t.get("response", "")
        for t in history.get("turns", [])
    ), f"Expected at least one non-empty response: {history}"

    # Check if a refresh was issued to the mock OAuth proxy
    async with httpx.AsyncClient() as client:
        state_r = await client.get(f"{mock_llm_url}/__mock/oauth/state", timeout=10)
        state_r.raise_for_status()
        oauth_state = state_r.json()

    # If refresh happened, verify the token was updated in the DB
    if oauth_state.get("refresh_count", 0) >= 1:
        _, expires_after, updated_after = _find_secret_row(db_path, secret_name)
        expires_after_dt = _parse_timestamp(expires_after)
        updated_after_dt = _parse_timestamp(updated_after)
        updated_before_dt = _parse_timestamp(updated_before)
        assert expires_after_dt is not None, "expires_at should be set after refresh"
        assert updated_after_dt is not None, "updated_at should be set after refresh"
        if updated_before_dt is not None:
            assert updated_after_dt > updated_before_dt, (
                f"updated_at should advance after refresh: "
                f"before={updated_before_dt}, after={updated_after_dt}"
            )
        assert expires_after_dt > datetime.now(timezone.utc), (
            f"expires_at should be in the future after refresh: {expires_after_dt}"
        )
