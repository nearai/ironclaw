"""Unified v2 auth/OAuth E2E matrix.

Exercises the public gateway against an isolated ENGINE_V2 instance and
verifies the hosted OAuth flow across:

- OAuth-backed WASM tools
- OAuth-backed WASM channels
- MCP servers

Negative-path coverage is included for provider error, stale/replayed callback
state, and token-exchange failure.
"""

import asyncio
import os
import signal
import socket
import sqlite3
import sys
import tempfile
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import httpx
import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import AUTH_TOKEN, api_get, api_post, wait_for_ready


ROOT = Path(__file__).resolve().parent.parent.parent.parent
TEST_USER_ID = "e2e-auth-matrix"
MASTER_KEY = (
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
)


def _forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(
            ("CARGO_LLVM_COV", "LLVM_", "CARGO_ENCODED_RUSTFLAGS", "CARGO_INCREMENTAL")
        ):
            env[key] = value


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


async def _start_mock_google_api():
    from aiohttp import web

    received_tokens: list[str] = []

    async def handle_drive_files(request: web.Request) -> web.Response:
        auth = request.headers.get("Authorization", "")
        if not auth.startswith("Bearer "):
            return web.json_response({"error": "missing_auth"}, status=401)
        received_tokens.append(auth.split(" ", 1)[1])
        return web.json_response(
            {
                "files": [
                    {"name": "Budget Q1.xlsx"},
                    {"name": "Roadmap.md"},
                ]
            }
        )

    async def handle_userinfo(request: web.Request) -> web.Response:
        return web.json_response({"email": "matrix@example.com", "name": "Matrix User"})

    async def handle_received_tokens(request: web.Request) -> web.Response:
        return web.json_response({"tokens": received_tokens})

    app = web.Application()
    app.router.add_get("/drive/v3/files", handle_drive_files)
    app.router.add_get("/oauth2/v2/userinfo", handle_userinfo)
    app.router.add_get("/__mock/received-tokens", handle_received_tokens)

    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = site._server.sockets[0].getsockname()[1]
    return {
        "base_url": f"http://127.0.0.1:{port}",
        "host": f"127.0.0.1:{port}",
        "runner": runner,
    }


def _write_google_skill(skills_dir: str, mock_api_host: str) -> None:
    skill_dir = os.path.join(skills_dir, "google-auth-matrix")
    os.makedirs(skill_dir, exist_ok=True)
    with open(os.path.join(skill_dir, "SKILL.md"), "w", encoding="utf-8") as handle:
        handle.write(
            f"""---
name: google_auth_matrix
version: "1.0.0"
keywords:
  - google
  - drive
  - gmail
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts:
      - "{mock_api_host}"
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      scopes:
        - "https://www.googleapis.com/auth/drive.readonly"
      test_url: "https://www.googleapis.com/oauth2/v1/userinfo"
    setup_instructions: "Sign in with Google"
---
# Google Auth Matrix Skill

Use the `http` tool to list Google Drive files.
"""
        )


def _write_oauth_wasm_channel(channels_dir: str) -> None:
    os.makedirs(channels_dir, exist_ok=True)
    wasm_path = os.path.join(channels_dir, "gmail-channel.wasm")
    caps_path = os.path.join(channels_dir, "gmail-channel.capabilities.json")
    with open(wasm_path, "wb") as handle:
        handle.write(b"fake-channel")
    with open(caps_path, "w", encoding="utf-8") as handle:
        handle.write(
            """{
  "name": "gmail-channel",
  "display_name": "Gmail Channel",
  "description": "OAuth-backed test channel",
  "setup": {
    "required_secrets": [
      {
        "name": "google_oauth_token",
        "prompt": "Sign in with Google"
      }
    ]
  }
}
"""
        )


def _extract_state(auth_url: str) -> str:
    parsed = urlparse(auth_url)
    state = parse_qs(parsed.query).get("state", [None])[0]
    assert state, f"auth_url missing state: {auth_url}"
    return state


async def _seed_mock_llm_api_url(mock_llm_server: str, mock_api_url: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/set_github_api_url",
            json={"url": mock_api_url},
            timeout=15,
        )
    response.raise_for_status()


async def _start_auth_matrix_server(
    ironclaw_binary: str,
    mock_llm_server: str,
    mock_api_url: str,
    *,
    exchange_url: str,
):
    reserved = []
    for _ in range(2):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", 0))
        reserved.append(sock)

    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-home-")
    tools_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-tools-")
    channels_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-channels-")

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            sock.close()

        home_dir = home_tmpdir.name
        skills_dir = os.path.join(home_dir, ".ironclaw", "skills")
        os.makedirs(skills_dir, exist_ok=True)
        _write_google_skill(skills_dir, mock_api_url.replace("http://", ""))
        _write_oauth_wasm_channel(channels_tmpdir.name)

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": home_dir,
            "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
            "RUST_LOG": "ironclaw=info",
            "RUST_BACKTRACE": "1",
            "ENGINE_V2": "true",
            "HTTP_ALLOW_LOCALHOST": "true",
            "SECRETS_MASTER_KEY": MASTER_KEY,
            "GATEWAY_ENABLED": "true",
            "GATEWAY_HOST": "127.0.0.1",
            "GATEWAY_PORT": str(gateway_port),
            "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
            "GATEWAY_USER_ID": TEST_USER_ID,
            "HTTP_HOST": "127.0.0.1",
            "HTTP_PORT": str(http_port),
            "CLI_ENABLED": "false",
            "LLM_BACKEND": "openai_compatible",
            "LLM_BASE_URL": mock_llm_server,
            "LLM_MODEL": "mock-model",
            "DATABASE_BACKEND": "libsql",
            "LIBSQL_PATH": os.path.join(db_tmpdir.name, "auth-matrix.db"),
            "SANDBOX_ENABLED": "false",
            "SKILLS_ENABLED": "true",
            "ROUTINES_ENABLED": "false",
            "HEARTBEAT_ENABLED": "false",
            "EMBEDDING_ENABLED": "false",
            "WASM_ENABLED": "true",
            "WASM_TOOLS_DIR": tools_tmpdir.name,
            "WASM_CHANNELS_DIR": channels_tmpdir.name,
            "ONBOARD_COMPLETED": "true",
            "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
            "IRONCLAW_OAUTH_EXCHANGE_URL": exchange_url,
            "GOOGLE_OAUTH_CLIENT_ID": "hosted-google-client-id",
        }
        _forward_coverage_env(env)

        proc = await asyncio.create_subprocess_exec(
            ironclaw_binary,
            "--no-onboard",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env,
        )

        base_url = f"http://127.0.0.1:{gateway_port}"
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            await _seed_mock_llm_api_url(mock_llm_server, mock_api_url)
            return {
                "base_url": base_url,
                "db_path": os.path.join(db_tmpdir.name, "auth-matrix.db"),
                "gateway_user_id": TEST_USER_ID,
                "mock_llm_url": mock_llm_server,
                "mock_api_url": mock_api_url,
                "proc": proc,
                "tmpdirs": [db_tmpdir, home_tmpdir, tools_tmpdir, channels_tmpdir],
            }
        except Exception:
            if proc.returncode is None:
                await _stop_process(proc, timeout=2)
            raise
    except Exception:
        for sock in reserved:
            try:
                sock.close()
            except Exception:
                pass
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()
        tools_tmpdir.cleanup()
        channels_tmpdir.cleanup()
        raise


async def _shutdown_auth_matrix_server(server: dict) -> None:
    proc = server["proc"]
    if proc.returncode is None:
        await _stop_process(proc, sig=signal.SIGINT, timeout=10)
        if proc.returncode is None:
            await _stop_process(proc, timeout=2)
    for tmpdir in server["tmpdirs"]:
        tmpdir.cleanup()


@pytest.fixture
async def auth_matrix_server(ironclaw_binary, mock_llm_server):
    mock_api = await _start_mock_google_api()
    server = await _start_auth_matrix_server(
        ironclaw_binary,
        mock_llm_server,
        mock_api["base_url"],
        exchange_url=mock_llm_server,
    )
    try:
        yield server
    finally:
        await _shutdown_auth_matrix_server(server)
        await mock_api["runner"].cleanup()


@pytest.fixture
async def auth_matrix_exchange_failure_server(ironclaw_binary, mock_llm_server):
    mock_api = await _start_mock_google_api()
    server = await _start_auth_matrix_server(
        ironclaw_binary,
        mock_llm_server,
        mock_api["base_url"],
        exchange_url="http://127.0.0.1:1",
    )
    try:
        yield server
    finally:
        await _shutdown_auth_matrix_server(server)
        await mock_api["runner"].cleanup()


def _secret_exists(db_path: str, user_id: str, name: str) -> bool:
    with sqlite3.connect(db_path) as conn:
        row = conn.execute(
            "SELECT 1 FROM secrets WHERE user_id = ?1 AND name = ?2 LIMIT 1",
            (user_id, name),
        ).fetchone()
    return row is not None


async def _get_extension(base_url: str, name: str) -> dict | None:
    response = await api_get(base_url, "/api/extensions", timeout=15)
    response.raise_for_status()
    for extension in response.json().get("extensions", []):
        if extension["name"] == name:
            return extension
    return None


async def _wait_for_extension(base_url: str, name: str, *, timeout: float = 30.0) -> dict:
    for _ in range(int(timeout * 2)):
        extension = await _get_extension(base_url, name)
        if extension is not None:
            return extension
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for extension {name}")


async def _install_extension(
    base_url: str,
    name: str,
    *,
    kind: str | None = None,
    url: str | None = None,
):
    payload: dict[str, str] = {"name": name}
    if kind is not None:
        payload["kind"] = kind
    if url is not None:
        payload["url"] = url
    response = await api_post(
        base_url,
        "/api/extensions/install",
        json=payload,
        timeout=180,
    )
    assert response.status_code == 200, response.text
    assert response.json().get("success") is True, response.text
    return response.json()


async def _wait_for_pending_gate(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    last = None
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        response.raise_for_status()
        history = response.json()
        last = history
        pending = history.get("pending_gate")
        if pending:
            return pending
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for pending_gate in thread {thread_id}. Last history: {last}"
    )


async def _wait_for_auth_prompt(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    last = None
    indicators = [
        "paste your token",
        "token below",
        "authentication required for",
    ]
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        response.raise_for_status()
        history = response.json()
        last = history
        turns = history.get("turns", [])
        if turns:
            text = (turns[-1].get("response") or "").lower()
            if any(ind in text for ind in indicators):
                return history
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for auth prompt in thread {thread_id}. Last history: {last}"
    )


async def _wait_for_no_pending_gate(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    last = None
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        response.raise_for_status()
        last = response.json()
        if not last.get("pending_gate"):
            return last
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for pending_gate to clear in {thread_id}: {last}")


async def _wait_for_response_contains(
    base_url: str,
    thread_id: str,
    needle: str,
    *,
    timeout: float = 45.0,
) -> dict:
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        response.raise_for_status()
        history = response.json()
        all_text = " ".join((turn.get("response") or "") for turn in history.get("turns", []))
        if needle.lower() in all_text.lower():
            return history
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for response containing {needle!r}")


async def _create_thread(base_url: str) -> str:
    response = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    response.raise_for_status()
    return response.json()["id"]


async def _send_chat(base_url: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code == 202, response.text


async def _complete_callback(
    base_url: str,
    auth_url: str,
    *,
    code: str,
) -> httpx.Response:
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{base_url}/oauth/callback",
            params={"code": code, "state": _extract_state(auth_url)},
            timeout=30,
            follow_redirects=True,
        )
    return response


async def _run_provider_error_callback(base_url: str, auth_url: str) -> httpx.Response:
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{base_url}/oauth/callback",
            params={
                "error": "access_denied",
                "error_description": "access_denied",
                "state": _extract_state(auth_url),
            },
            timeout=30,
            follow_redirects=True,
        )
    return response


async def _get_mock_received_tokens(mock_api_url: str) -> list[str]:
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{mock_api_url}/__mock/received-tokens", timeout=15)
    response.raise_for_status()
    return response.json()["tokens"]


async def _wait_for_mock_token(
    mock_api_url: str,
    token: str,
    *,
    timeout: float = 45.0,
) -> list[str]:
    last: list[str] = []
    for _ in range(int(timeout * 2)):
        last = await _get_mock_received_tokens(mock_api_url)
        if token in last:
            return last
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for token {token!r}. Last tokens: {last}")


async def _get_http_auth_prompt(
    server: dict, prompt: str = "list google drive files"
) -> tuple[str, dict]:
    thread_id = await _create_thread(server["base_url"])
    await _send_chat(server["base_url"], thread_id, prompt)
    history = await _wait_for_auth_prompt(server["base_url"], thread_id, timeout=60)
    return thread_id, history


async def _wasm_tool_auth_url(server: dict) -> str:
    await _install_extension(server["base_url"], "gmail")
    response = await api_post(
        server["base_url"],
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert response.status_code == 200, response.text
    auth_url = response.json().get("auth_url")
    assert auth_url, response.text
    return auth_url


async def _wasm_channel_auth_url(server: dict) -> str:
    await _wait_for_extension(server["base_url"], "gmail-channel")
    response = await api_post(
        server["base_url"],
        "/api/extensions/gmail-channel/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert response.status_code == 200, response.text
    auth_url = response.json().get("auth_url")
    assert auth_url, response.text
    return auth_url


async def _mcp_auth_url(server: dict) -> str:
    await _install_extension(
        server["base_url"],
        "mock-mcp",
        kind="mcp_server",
        url=f"{server['mock_llm_url']}/mcp",
    )
    response = await api_post(
        server["base_url"],
        "/api/extensions/mock-mcp/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert response.status_code == 200, response.text
    auth_url = response.json().get("auth_url")
    if auth_url:
        return auth_url
    response = await api_post(
        server["base_url"],
        "/api/extensions/mock-mcp/activate",
        timeout=30,
    )
    assert response.status_code == 200, response.text
    auth_url = response.json().get("auth_url")
    assert auth_url, response.text
    return auth_url


async def _cancel_gate(base_url: str, thread_id: str, request_id: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/gate/resolve",
        json={
            "thread_id": thread_id,
            "request_id": request_id,
            "resolution": "cancelled",
        },
        timeout=30,
    )
    assert response.status_code == 200, response.text


async def _assert_callback_failed(response: httpx.Response) -> None:
    assert response.status_code == 200, response.text[:400]
    body = response.text.lower()
    assert "failed" in body or "error" in body or "expired" in body, response.text[:500]


async def test_wasm_tool_oauth_provider_error_leaves_extension_unauthed(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _wasm_tool_auth_url(server)

    response = await _run_provider_error_callback(server["base_url"], auth_url)
    await _assert_callback_failed(response)

    extension = await _wait_for_extension(server["base_url"], "gmail")
    assert extension["authenticated"] is False, extension


async def test_wasm_tool_oauth_exchange_failure_leaves_extension_unauthed(
    auth_matrix_exchange_failure_server,
):
    server = auth_matrix_exchange_failure_server
    auth_url = await _wasm_tool_auth_url(server)

    response = await _complete_callback(
        server["base_url"], auth_url, code="mock_auth_code"
    )
    await _assert_callback_failed(response)

    extension = await _wait_for_extension(server["base_url"], "gmail")
    assert extension["authenticated"] is False, extension


async def test_wasm_tool_oauth_roundtrip(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _wasm_tool_auth_url(server)

    response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
    assert response.status_code == 200, response.text[:400]

    extension = await _wait_for_extension(server["base_url"], "gmail")
    assert extension["authenticated"] is True, extension


async def test_wasm_channel_oauth_roundtrip(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _wasm_channel_auth_url(server)

    response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
    assert response.status_code == 200, response.text[:400]

    extension = await _wait_for_extension(server["base_url"], "gmail-channel")
    assert extension["authenticated"] is True, extension
    assert _secret_exists(
        server["db_path"], server["gateway_user_id"], "google_oauth_token"
    )


async def test_mcp_oauth_roundtrip(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _mcp_auth_url(server)

    response = await _complete_callback(server["base_url"], auth_url, code="mock_mcp_code")
    assert response.status_code == 200, response.text[:400]

    extension = await _wait_for_extension(server["base_url"], "mock-mcp")
    assert extension["authenticated"] is True, extension
    assert any("mock_search" in tool for tool in extension.get("tools", [])), extension


@pytest.mark.parametrize(
    ("surface", "code"),
    [
        ("wasm_tool", "mock_auth_code"),
        ("wasm_channel", "mock_auth_code"),
        ("mcp", "mock_mcp_code"),
    ],
)
async def test_oauth_callback_replay_is_rejected(auth_matrix_server, surface, code):
    server = auth_matrix_server
    if surface == "wasm_tool":
        auth_url = await _wasm_tool_auth_url(server)
    elif surface == "wasm_channel":
        auth_url = await _wasm_channel_auth_url(server)
    else:
        auth_url = await _mcp_auth_url(server)

    first = await _complete_callback(server["base_url"], auth_url, code=code)
    assert first.status_code == 200, first.text[:400]

    replay = await _complete_callback(server["base_url"], auth_url, code=code)
    await _assert_callback_failed(replay)
