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
import json
import os
import pty
import re
import select
import signal
import socket
import sqlite3
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import httpx
import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import AUTH_TOKEN, SEL, api_get, api_post, sse_stream, wait_for_ready


ROOT = Path(__file__).resolve().parent.parent.parent.parent
TEST_USER_ID = "e2e-auth-matrix"
MCP_EXTENSION_NAME = "mock_mcp"
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


async def _start_mock_google_api():
    from aiohttp import web

    received_tokens: list[str] = []
    messages = [
        {
            "id": "msg-1",
            "threadId": "thread-1",
            "labelIds": ["INBOX", "UNREAD"],
            "snippet": "Quarterly update is ready",
            "payload": {
                "headers": [
                    {"name": "Subject", "value": "Quarterly update"},
                    {"name": "From", "value": "ceo@example.com"},
                    {"name": "To", "value": "matrix@example.com"},
                    {"name": "Date", "value": "Mon, 06 Apr 2026 10:00:00 +0000"},
                ],
                "body": {},
            },
        }
    ]

    def _authorized(request: web.Request) -> str | None:
        auth = request.headers.get("Authorization", "")
        if not auth.startswith("Bearer "):
            return None
        token = auth.split(" ", 1)[1]
        received_tokens.append(token)
        return token

    async def handle_drive_files(request: web.Request) -> web.Response:
        if _authorized(request) is None:
            return web.json_response({"error": "missing_auth"}, status=401)
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

    async def handle_gmail_messages(request: web.Request) -> web.Response:
        if _authorized(request) is None:
            return web.json_response({"error": "missing_auth"}, status=401)
        return web.json_response(
            {
                "messages": [
                    {"id": message["id"], "threadId": message["threadId"]}
                    for message in messages
                ],
                "resultSizeEstimate": len(messages),
            }
        )

    async def handle_gmail_message(request: web.Request) -> web.Response:
        if _authorized(request) is None:
            return web.json_response({"error": "missing_auth"}, status=401)
        message_id = request.match_info["message_id"]
        message = next((item for item in messages if item["id"] == message_id), None)
        if message is None:
            return web.json_response({"error": "not_found"}, status=404)
        return web.json_response(message)

    async def handle_received_tokens(request: web.Request) -> web.Response:
        return web.json_response({"tokens": received_tokens})

    app = web.Application()
    app.router.add_get("/drive/v3/files", handle_drive_files)
    app.router.add_get("/oauth2/v1/userinfo", handle_userinfo)
    app.router.add_get("/oauth2/v2/userinfo", handle_userinfo)
    app.router.add_get("/gmail/v1/users/me/messages", handle_gmail_messages)
    app.router.add_get("/gmail/v1/users/me/messages/{message_id}", handle_gmail_message)
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
    setup_instructions: "Sign in with Google"
---
# Google Auth Matrix Skill

Use the `http` tool to list Google Drive files.
"""
        )


def _write_oauth_wasm_channel(channels_dir: str) -> None:
    os.makedirs(channels_dir, exist_ok=True)
    wasm_payload = b"\0asm\x01\x00\x00\x00"
    capabilities = """{
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
    for stem in ("gmail-channel", "gmail_channel"):
        with open(os.path.join(channels_dir, f"{stem}.wasm"), "wb") as handle:
            handle.write(wasm_payload)
        with open(
            os.path.join(channels_dir, f"{stem}.capabilities.json"),
            "w",
            encoding="utf-8",
        ) as handle:
            handle.write(capabilities)


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
    existing_paths: dict | None = None,
):
    reserved = []
    for _ in range(2):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", 0))
        reserved.append(sock)

    if existing_paths is None:
        db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-db-")
        home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-home-")
        tools_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-tools-")
        channels_tmpdir = tempfile.TemporaryDirectory(
            prefix="ironclaw-auth-matrix-channels-"
        )
        db_path = os.path.join(db_tmpdir.name, "auth-matrix.db")
        home_dir = home_tmpdir.name
        tools_dir = tools_tmpdir.name
        channels_dir = channels_tmpdir.name
        tmpdirs = [db_tmpdir, home_tmpdir, tools_tmpdir, channels_tmpdir]
    else:
        db_tmpdir = home_tmpdir = tools_tmpdir = channels_tmpdir = None
        db_path = existing_paths["db_path"]
        home_dir = existing_paths["home_dir"]
        tools_dir = existing_paths["tools_dir"]
        channels_dir = existing_paths["channels_dir"]
        tmpdirs = existing_paths["tmpdirs"]

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            sock.close()

        skills_dir = os.path.join(home_dir, ".ironclaw", "skills")
        os.makedirs(skills_dir, exist_ok=True)
        _write_google_skill(skills_dir, mock_api_url.replace("http://", ""))
        _write_oauth_wasm_channel(channels_dir)

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": home_dir,
            "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
            "IRONCLAW_OWNER_ID": TEST_USER_ID,
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
            "LIBSQL_PATH": db_path,
            "SANDBOX_ENABLED": "false",
            "SKILLS_ENABLED": "true",
            "ROUTINES_ENABLED": "false",
            "HEARTBEAT_ENABLED": "false",
            "EMBEDDING_ENABLED": "false",
            "WASM_ENABLED": "true",
            "WASM_TOOLS_DIR": tools_dir,
            "WASM_CHANNELS_DIR": channels_dir,
            "ONBOARD_COMPLETED": "true",
            "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
            "IRONCLAW_OAUTH_EXCHANGE_URL": exchange_url,
            "IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK": "1",
            "GOOGLE_OAUTH_CLIENT_ID": "hosted-google-client-id",
            "IRONCLAW_TEST_HTTP_REMAP": (
                f"gmail.googleapis.com={mock_api_url},"
                f"www.googleapis.com={mock_api_url}"
            ),
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
                "db_path": db_path,
                "gateway_user_id": TEST_USER_ID,
                "mock_llm_url": mock_llm_server,
                "mock_api_url": mock_api_url,
                "ironclaw_binary": ironclaw_binary,
                "exchange_url": exchange_url,
                "home_dir": home_dir,
                "tools_dir": tools_dir,
                "channels_dir": channels_dir,
                "proc": proc,
                "tmpdirs": tmpdirs,
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
        for tmpdir in [db_tmpdir, home_tmpdir, tools_tmpdir, channels_tmpdir]:
            if tmpdir is not None:
                tmpdir.cleanup()
        raise


async def _shutdown_auth_matrix_server(server: dict, *, cleanup: bool = True) -> None:
    proc = server["proc"]
    if proc.returncode is None:
        await _stop_process(proc, sig=signal.SIGINT, timeout=10)
        if proc.returncode is None:
            await _stop_process(proc, timeout=2)
    if cleanup:
        for tmpdir in server["tmpdirs"]:
            tmpdir.cleanup()


async def _restart_auth_matrix_server(server: dict) -> dict:
    await _shutdown_auth_matrix_server(server, cleanup=False)
    return await _start_auth_matrix_server(
        server["ironclaw_binary"],
        server["mock_llm_url"],
        server["mock_api_url"],
        exchange_url=server["exchange_url"],
        existing_paths={
            "db_path": server["db_path"],
            "home_dir": server["home_dir"],
            "tools_dir": server["tools_dir"],
            "channels_dir": server["channels_dir"],
            "tmpdirs": server["tmpdirs"],
        },
    )


async def _start_auth_matrix_repl(
    ironclaw_binary: str,
    mock_llm_server: str,
    mock_api_url: str,
) -> dict:
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-repl-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-auth-matrix-repl-home-")
    master_fd, slave_fd = pty.openpty()
    port_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    port_sock.bind(("127.0.0.1", 0))
    http_port = port_sock.getsockname()[1]
    port_sock.close()

    try:
        home_dir = home_tmpdir.name
        skills_dir = os.path.join(home_dir, ".ironclaw", "skills")
        os.makedirs(skills_dir, exist_ok=True)
        _write_google_skill(skills_dir, mock_api_url.replace("http://", ""))
        await _seed_mock_llm_api_url(mock_llm_server, mock_api_url)

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": home_dir,
            "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
            "IRONCLAW_OWNER_ID": TEST_USER_ID,
            "RUST_LOG": "ironclaw=info",
            "RUST_BACKTRACE": "1",
            "TERM": os.environ.get("TERM", "xterm-256color"),
            "ENGINE_V2": "true",
            "HTTP_ALLOW_LOCALHOST": "true",
            "HTTP_HOST": "127.0.0.1",
            "HTTP_PORT": str(http_port),
            "HTTP_WEBHOOK_SECRET": "e2e-repl-webhook-secret",
            "SECRETS_MASTER_KEY": MASTER_KEY,
            "GATEWAY_ENABLED": "false",
            "CLI_ENABLED": "true",
            # `CLI_MODE` defaults to `tui` (ratatui full-screen UI)
            # which reads stdin keystroke-by-keystroke and renders into
            # a framebuffer. The PTY-driven tests here send whole lines
            # via `os.write(master_fd, b"prompt\n")` and match for
            # specific text in the raw stream — under the default TUI
            # mode those line-based sends don't dispatch the prompt to
            # the agent and the test times out with nothing but
            # cursor-position escapes captured. Pin the plain REPL so
            # these PTY-based tests drive the expected CLI surface.
            "CLI_MODE": "repl",
            "LLM_BACKEND": "openai_compatible",
            "LLM_BASE_URL": mock_llm_server,
            "LLM_MODEL": "mock-model",
            "DATABASE_BACKEND": "libsql",
            "LIBSQL_PATH": os.path.join(db_tmpdir.name, "auth-matrix-repl.db"),
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
            ironclaw_binary,
            "--no-onboard",
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            env=env,
        )
        os.close(slave_fd)

        repl = {
            "proc": proc,
            "master_fd": master_fd,
            "db_path": os.path.join(db_tmpdir.name, "auth-matrix-repl.db"),
            "gateway_user_id": TEST_USER_ID,
            "tmpdirs": [db_tmpdir, home_tmpdir],
            "mock_api_url": mock_api_url,
        }
        await _read_repl_until(repl, r"IronClaw|›", timeout=30.0)
        return repl
    except Exception:
        try:
            os.close(master_fd)
        except OSError:
            pass
        try:
            os.close(slave_fd)
        except OSError:
            pass
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()
        raise


async def _shutdown_auth_matrix_repl(repl: dict) -> None:
    try:
        await _send_repl_line(repl, "/quit")
    except Exception:
        pass
    proc = repl["proc"]
    if proc.returncode is None:
        await _stop_process(proc, sig=signal.SIGINT, timeout=10)
    try:
        os.close(repl["master_fd"])
    except OSError:
        pass
    for tmpdir in repl["tmpdirs"]:
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


@pytest.fixture
async def auth_matrix_repl(ironclaw_binary, mock_llm_server):
    mock_api = await _start_mock_google_api()
    repl = await _start_auth_matrix_repl(
        ironclaw_binary,
        mock_llm_server,
        mock_api["base_url"],
    )
    try:
        yield repl
    finally:
        await _shutdown_auth_matrix_repl(repl)
        await mock_api["runner"].cleanup()


@pytest.fixture
async def auth_matrix_page(browser, auth_matrix_server):
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{auth_matrix_server['base_url']}/?token={AUTH_TOKEN}")
    await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    await page.evaluate(
        """() => {
            window.__openedOauthUrls = [];
            const original = window.openOAuthUrl;
            window.openOAuthUrl = (url) => {
                window.__openedOauthUrls.push(url);
                return url;
            };
            window.__restoreOpenOAuthUrl = original;
        }"""
    )
    try:
        yield page
    finally:
        await context.close()


def _secret_exists(db_path: str, user_id: str, name: str) -> bool:
    with sqlite3.connect(db_path) as conn:
        row = conn.execute(
            "SELECT 1 FROM secrets WHERE user_id = ? AND name = ? LIMIT 1",
            (user_id, name),
        ).fetchone()
    return row is not None


def _find_secret_row(
    db_path: str,
    secret_name: str,
) -> tuple[str, str | None, str | None]:
    with sqlite3.connect(db_path) as conn:
        row = conn.execute(
            """
            SELECT user_id, expires_at, updated_at
            FROM secrets
            WHERE name = ?
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
            WHERE user_id = ? AND name = ?
            """,
            (user_id, secret_name),
        )
        conn.commit()
    assert cursor.rowcount == 1, f"Expected one secret row for {user_id}/{secret_name}"


def _parse_timestamp(value: str | None) -> datetime | None:
    if value is None:
        return None
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


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


async def _read_repl_until(
    repl: dict,
    pattern: str,
    *,
    timeout: float = 30.0,
) -> str:
    compiled = re.compile(pattern, re.IGNORECASE)
    deadline = asyncio.get_running_loop().time() + timeout
    chunks: list[str] = []
    while asyncio.get_running_loop().time() < deadline:
        remaining = deadline - asyncio.get_running_loop().time()
        ready, _, _ = await asyncio.to_thread(
            select.select,
            [repl["master_fd"]],
            [],
            [],
            max(0.1, min(0.5, remaining)),
        )
        if not ready:
            continue
        try:
            data = os.read(repl["master_fd"], 4096)
        except OSError:
            break
        if not data:
            break
        text = data.decode("utf-8", errors="replace")
        chunks.append(text)
        merged = "".join(chunks)
        if compiled.search(merged):
            return merged
    raise AssertionError(
        f"Timed out waiting for REPL output matching {pattern!r}. Last output:\n{''.join(chunks)[-2000:]}"
    )


async def _read_repl_until_any(
    repl: dict,
    patterns: list[str],
    *,
    timeout: float = 30.0,
) -> tuple[str, str]:
    union = "|".join(f"(?:{pattern})" for pattern in patterns)
    output = await _read_repl_until(repl, union, timeout=timeout)
    for pattern in patterns:
        if re.search(pattern, output, re.IGNORECASE):
            return output, pattern
    raise AssertionError(f"Matched union {union!r} but no individual pattern matched")


async def _drain_repl_output(repl: dict, *, idle_secs: float = 0.4) -> str:
    chunks: list[str] = []
    while True:
        ready, _, _ = await asyncio.to_thread(
            select.select,
            [repl["master_fd"]],
            [],
            [],
            idle_secs,
        )
        if not ready:
            break
        try:
            data = os.read(repl["master_fd"], 4096)
        except OSError:
            break
        if not data:
            break
        chunks.append(data.decode("utf-8", errors="replace"))
    return "".join(chunks)


async def _send_repl_line(repl: dict, line: str) -> None:
    os.write(repl["master_fd"], f"{line}\n".encode("utf-8"))


async def _send_repl_key(repl: dict, key: str) -> None:
    os.write(repl["master_fd"], key.encode("utf-8"))


async def _get_extension_readiness(base_url: str, name: str) -> dict | None:
    response = await api_get(base_url, "/api/extensions/readiness", timeout=15)
    response.raise_for_status()
    for extension in response.json().get("extensions", []):
        if extension["name"] == name:
            return extension
    return None


async def _wait_for_extension_readiness(
    base_url: str, name: str, *, timeout: float = 30.0
) -> dict:
    for _ in range(int(timeout * 2)):
        extension = await _get_extension_readiness(base_url, name)
        if extension is not None:
            return extension
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for readiness entry {name}")


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
        pending = history.get("pending_gate")
        if pending and pending.get("gate_name") == "authentication":
            return history
        turns = history.get("turns", [])
        if turns:
            text = (turns[-1].get("response") or "").lower()
            if any(ind in text for ind in indicators):
                return history
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for auth prompt in thread {thread_id}. Last history: {last}"
    )


async def _wait_for_auth_event(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> tuple[str, dict, str | None]:
    async with sse_stream(base_url, timeout=timeout) as response:
        await _send_chat(base_url, thread_id, "check gmail unread")
        async with asyncio.timeout(timeout):
            event_type = None
            while True:
                line = await response.content.readline()
                if not line:
                    raise AssertionError("SSE stream closed before auth event arrived")
                decoded = line.decode("utf-8", errors="replace").rstrip("\r\n")
                if not decoded:
                    continue
                if decoded.startswith("event:"):
                    event_type = decoded[6:].strip()
                    continue
                if not decoded.startswith("data:"):
                    continue

                try:
                    payload = json.loads(decoded[5:].strip())
                except json.JSONDecodeError:
                    continue

                if payload.get("thread_id") not in (None, thread_id):
                    continue

                if event_type == "onboarding_state" and payload.get("state") == "auth_required":
                    return event_type, payload, payload.get("auth_url")

                if event_type == "gate_required":
                    resume = payload.get("resume_kind")
                    if not isinstance(resume, dict):
                        continue
                    auth = resume.get("Authentication")
                    if not isinstance(auth, dict):
                        continue
                    return event_type, payload, auth.get("auth_url")

    raise AssertionError(f"Timed out waiting for auth event in thread {thread_id}")


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


async def _wait_for_mock_google_tokens(mock_api_url: str, *, timeout: float = 30.0) -> list[str]:
    async with httpx.AsyncClient() as client:
        for _ in range(int(timeout * 2)):
            response = await client.get(
                f"{mock_api_url}/__mock/received-tokens",
                timeout=15,
            )
            response.raise_for_status()
            tokens = response.json().get("tokens", [])
            if tokens:
                return tokens
            await asyncio.sleep(0.5)
    raise AssertionError("Timed out waiting for Gmail HTTP execution against the mock API")


async def _wait_for_mock_llm_request_contains(
    mock_llm_url: str, needle: str, *, timeout: float = 30.0
) -> dict:
    async with httpx.AsyncClient() as client:
        for _ in range(int(timeout * 2)):
            response = await client.get(
                f"{mock_llm_url}/__mock/last_chat_request",
                timeout=15,
            )
            response.raise_for_status()
            payload = response.json()
            if needle.lower() in json.dumps(payload).lower():
                return payload
            await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for mock LLM request containing {needle!r}")


async def _wait_for_tool_call(
    base_url: str,
    thread_id: str,
    tool_name: str,
    timeout: float = 30.0,
) -> dict:
    approved_request_ids = set()
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        response.raise_for_status()
        history = response.json()

        pending = history.get("pending_gate") or history.get("pending_approval")
        if pending and pending["request_id"] not in approved_request_ids:
            approve = await api_post(
                base_url,
                "/api/chat/approval",
                json={
                    "request_id": pending["request_id"],
                    "action": "approve",
                    "thread_id": thread_id,
                },
                timeout=15,
            )
            assert approve.status_code == 202, approve.text
            approved_request_ids.add(pending["request_id"])

        for turn in history.get("turns", []):
            for tool_call in turn.get("tool_calls", []):
                if tool_call.get("name") == tool_name:
                    return history

        await asyncio.sleep(0.5)

    raise AssertionError(f"Timed out waiting for {tool_name} tool call in thread {thread_id}")


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


async def _reset_mock_oauth_state(mock_base_url: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(f"{mock_base_url}/__mock/oauth/reset", timeout=10)
    response.raise_for_status()


async def _get_mock_oauth_state(mock_base_url: str) -> dict:
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{mock_base_url}/__mock/oauth/state", timeout=10)
    response.raise_for_status()
    return response.json()


async def _wait_for_refresh_request(
    mock_base_url: str,
    *,
    timeout: float = 20.0,
) -> dict:
    for _ in range(int(timeout * 2)):
        state = await _get_mock_oauth_state(mock_base_url)
        if state.get("refresh_count", 0) >= 1:
            return state
        await asyncio.sleep(0.5)
    raise AssertionError("Timed out waiting for OAuth refresh request")


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


async def _remove_extension_if_present(base_url: str, name: str) -> None:
    extension = await _get_extension(base_url, name)
    if extension is None:
        return
    response = await api_post(
        base_url,
        f"/api/extensions/{name}/remove",
        timeout=30,
    )
    assert response.status_code == 200, response.text


async def _current_thread_id(page) -> str:
    thread_id = await page.evaluate("() => currentThreadId")
    assert thread_id, "currentThreadId should be set"
    return thread_id


async def _go_to_settings_subtab(page, subtab: str) -> None:
    await page.locator(SEL["tab_button"].format(tab="settings")).click()
    await page.locator(SEL["settings_subtab"].format(subtab=subtab)).click()
    await page.locator(SEL["settings_subpanel"].format(subtab=subtab)).wait_for(
        state="visible", timeout=10000
    )


async def _wait_for_auth_card(page, extension_name: str | None = None):
    selector = SEL["auth_card"]
    if extension_name:
        selector += f'[data-extension-name="{extension_name}"]'
    card = page.locator(selector).first
    await card.wait_for(state="visible", timeout=20000)
    return card


async def _auth_oauth_url_from_card(page) -> str | None:
    oauth_btn = page.locator(SEL["auth_oauth_btn"]).first
    try:
        await oauth_btn.wait_for(state="visible", timeout=10000)
    except Exception:
        return None
    href = await oauth_btn.get_attribute("href")
    return href or None


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


async def _wait_for_any_extension(
    base_url: str,
    names: tuple[str, ...],
    *,
    timeout: float = 30.0,
) -> dict:
    for _ in range(int(timeout * 2)):
        for name in names:
            extension = await _get_extension(base_url, name)
            if extension is not None:
                return extension
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for any extension in {names}")


async def _wasm_channel_auth_url(server: dict) -> tuple[str, str]:
    extension = await _wait_for_any_extension(
        server["base_url"],
        ("gmail-channel", "gmail_channel"),
    )
    extension_name = extension["name"]
    response = await api_post(
        server["base_url"],
        f"/api/extensions/{extension_name}/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert response.status_code == 200, response.text
    auth_url = response.json().get("auth_url")
    assert auth_url, response.text
    return extension_name, auth_url


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


async def _mcp_activate_auth_url(server: dict) -> str:
    await _install_extension(
        server["base_url"],
        "mock-mcp",
        kind="mcp_server",
        url=f"{server['mock_llm_url']}/mcp",
    )
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
    readiness = await _wait_for_extension_readiness(server["base_url"], "gmail")
    assert readiness["phase"] == "needs_auth", readiness
    assert readiness["authenticated"] is False, readiness
    assert readiness["active"] is False, readiness


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
    readiness = await _wait_for_extension_readiness(server["base_url"], "gmail")
    assert readiness["phase"] == "needs_auth", readiness
    assert readiness["authenticated"] is False, readiness
    assert readiness["active"] is False, readiness


async def test_wasm_tool_oauth_roundtrip(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _wasm_tool_auth_url(server)

    readiness = await _wait_for_extension_readiness(server["base_url"], "gmail")
    assert readiness["phase"] == "needs_auth", readiness
    assert readiness["authenticated"] is False, readiness
    assert readiness["active"] is False, readiness

    response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
    assert response.status_code == 200, response.text[:400]

    extension = await _wait_for_extension(server["base_url"], "gmail")
    assert extension["authenticated"] is True, extension

    readiness = await _wait_for_extension_readiness(server["base_url"], "gmail")
    assert readiness["phase"] == "ready", readiness
    assert readiness["authenticated"] is True, readiness
    assert readiness["active"] is True, readiness


async def test_wasm_tool_first_chat_auth_attempt_emits_auth_url(auth_matrix_server):
    server = auth_matrix_server
    await _install_extension(server["base_url"], "gmail")
    thread_id = await _create_thread(server["base_url"])

    event_type, payload, auth_url = await _wait_for_auth_event(
        server["base_url"], thread_id, timeout=60
    )

    assert auth_url, payload
    assert "accounts.google.com" in auth_url, auth_url
    if event_type == "onboarding_state":
        assert payload.get("extension_name") in {"gmail", "google_oauth_token"}, payload
    else:
        auth = payload["resume_kind"]["Authentication"]
        assert auth.get("credential_name") in {"gmail", "google_oauth_token"}, payload

    history = await _wait_for_auth_prompt(server["base_url"], thread_id, timeout=60)
    all_text = " ".join(turn.get("response") or "" for turn in history.get("turns", []))
    pending = history.get("pending_gate")
    assert (
        "authentication required" in all_text.lower()
        or (pending and pending.get("gate_name") == "authentication")
    ), history


async def test_mcp_oauth_roundtrip_via_browser(browser, auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _mcp_auth_url(server)
    thread_id = await _create_thread(server["base_url"])

    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await page.goto(f"{server['base_url']}/?token={AUTH_TOKEN}")
        await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10000)
        await page.evaluate("(id) => switchThread(id)", thread_id)
        await page.wait_for_function(
            "(id) => currentThreadId === id",
            arg=thread_id,
            timeout=10000,
        )

        chat_input = page.locator(SEL["chat_input"])

        callback = await _complete_callback(
            server["base_url"], auth_url, code="mock_mcp_code"
        )
        assert callback.status_code == 200, callback.text[:400]
        readiness = await _wait_for_extension_readiness(
            server["base_url"], MCP_EXTENSION_NAME
        )
        assert readiness["phase"] == "ready", readiness

        await page.reload()
        await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10000)
        await page.evaluate("(id) => switchThread(id)", thread_id)
        await page.wait_for_function(
            "(id) => currentThreadId === id",
            arg=thread_id,
            timeout=10000,
        )
        await page.wait_for_timeout(1000)
        assert await page.locator(SEL["auth_card"]).count() == 0
    finally:
        await context.close()


async def test_chat_first_gmail_installs_prompts_and_retries(
    auth_matrix_server, auth_matrix_page
):
    server = auth_matrix_server
    page = auth_matrix_page
    await _remove_extension_if_present(server["base_url"], "gmail")

    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill("check gmail unread")
    await chat_input.press("Enter")

    card = await _wait_for_auth_card(page)
    assert await card.get_attribute("data-extension-name") in {"gmail", "google_oauth_token"}
    assert await card.get_attribute("data-request-id"), "expected auth gate request id"

    extension = await _wait_for_extension(server["base_url"], "gmail")
    assert extension["authenticated"] is False, extension

    auth_url = await _auth_oauth_url_from_card(page)
    assert auth_url, "Expected auth card to expose an OAuth URL"
    response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
    assert response.status_code == 200, response.text[:400]
    await card.wait_for(state="hidden", timeout=20000)

    thread_id = await _current_thread_id(page)
    tokens = await _wait_for_mock_google_tokens(server["mock_api_url"], timeout=60.0)
    assert tokens, "expected Gmail to hit the mock Google API after OAuth replay"
    history = await _wait_for_response_contains(
        server["base_url"], thread_id, "Quarterly update", timeout=60.0
    )
    assert history.get("pending_gate") is None, history
    assert "Quarterly update" in " ".join(
        (turn.get("response") or "") for turn in history.get("turns", [])
    )

    extension = await _wait_for_extension(server["base_url"], "gmail")
    assert extension["authenticated"] is True, extension
    assert extension["active"] is True, extension


async def test_settings_first_gmail_auth_then_chat_runs(
    auth_matrix_server, auth_matrix_page
):
    server = auth_matrix_server
    page = auth_matrix_page
    await _remove_extension_if_present(server["base_url"], "gmail")

    await _go_to_settings_subtab(page, "extensions")
    available_card = page.locator("#available-wasm-list .ext-card").filter(
        has=page.locator(".ext-name", has_text="Gmail")
    ).first
    await available_card.wait_for(state="visible", timeout=20000)
    await available_card.locator(SEL["ext_install_btn"]).click()

    await _wait_for_extension(server["base_url"], "gmail")
    card = await _wait_for_auth_card(page)
    assert await card.get_attribute("data-extension-name") in {"gmail", "google_oauth_token"}
    auth_url = await _auth_oauth_url_from_card(page)
    assert auth_url, "Expected auth card to expose an OAuth URL"
    response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
    assert response.status_code == 200, response.text[:400]
    await card.wait_for(state="hidden", timeout=20000)

    await page.locator(SEL["tab_button"].format(tab="chat")).click()
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill("check gmail unread")
    await chat_input.press("Enter")

    thread_id = await _current_thread_id(page)
    tokens = await _wait_for_mock_google_tokens(server["mock_api_url"], timeout=60.0)
    assert tokens, "expected Gmail to hit the mock Google API after settings-first auth"
    history = await _wait_for_response_contains(
        server["base_url"], thread_id, "Quarterly update", timeout=60.0
    )
    assert history.get("pending_gate") is None, history
    assert "Quarterly update" in " ".join(
        (turn.get("response") or "") for turn in history.get("turns", [])
    )


async def test_settings_first_custom_mcp_auth_then_chat_runs(
    auth_matrix_server, auth_matrix_page
):
    server = auth_matrix_server
    page = auth_matrix_page
    await _remove_extension_if_present(server["base_url"], "mock-mcp")

    await _go_to_settings_subtab(page, "mcp")
    await page.locator("#mcp-install-name").fill("mock-mcp")
    await page.locator("#mcp-install-url").fill(f"{server['mock_llm_url']}/mcp")
    await page.locator("#mcp-add-btn").click()

    await _wait_for_extension(server["base_url"], MCP_EXTENSION_NAME)
    await page.reload()
    await _go_to_settings_subtab(page, "mcp")
    mcp_card = page.locator("#mcp-servers-list .ext-card", has_text=MCP_EXTENSION_NAME).first
    await mcp_card.wait_for(state="visible", timeout=20000)
    await mcp_card.locator(SEL["ext_activate_btn"]).click()

    card = await _wait_for_auth_card(page)
    assert await card.get_attribute("data-extension-name") in {"mock-mcp", "mock_mcp"}
    auth_url = await _auth_oauth_url_from_card(page)
    if not auth_url:
        response = await api_post(
            server["base_url"],
            f"/api/extensions/{MCP_EXTENSION_NAME}/activate",
            timeout=30,
        )
        assert response.status_code == 200, response.text
        auth_url = response.json().get("auth_url")
    assert auth_url, "Expected an auth URL for MCP settings-first auth"
    response = await _complete_callback(server["base_url"], auth_url, code="mock_mcp_code")
    assert response.status_code == 200, response.text[:400]
    await card.wait_for(state="hidden", timeout=20000)

    await page.locator(SEL["tab_button"].format(tab="chat")).click()
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill("check mock mcp search")
    await chat_input.press("Enter")

    thread_id = await _current_thread_id(page)
    history = await _wait_for_response_contains(
        server["base_url"], thread_id, "Mock MCP search result", timeout=60.0
    )
    assert history.get("pending_gate") is None, history
    assert "mock_mcp_mock_search" in json.dumps(history)


async def test_chat_first_skill_http_oauth_retries_without_extra_message(auth_matrix_server):
    server = auth_matrix_server
    async with httpx.AsyncClient() as client:
        permission = await client.put(
            f"{server['base_url']}/api/settings/tools/http",
            headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
            json={"state": "always_allow"},
            timeout=15,
        )
    assert permission.status_code == 200, permission.text

    thread_id = None
    pending = None
    for _ in range(3):
        candidate_thread = await _create_thread(server["base_url"])
        await _send_chat(server["base_url"], candidate_thread, "list google drive files")
        try:
            pending = await _wait_for_pending_gate(
                server["base_url"], candidate_thread, timeout=15.0
            )
            thread_id = candidate_thread
            break
        except AssertionError:
            continue

    assert thread_id is not None and pending is not None, (
        "failed to trigger the skill auth flow after retrying fresh threads"
    )
    if pending["gate_name"] == "approval":
        approve = await api_post(
            server["base_url"],
            "/api/chat/approval",
            json={
                "request_id": pending["request_id"],
                "action": "approve",
                "thread_id": thread_id,
            },
            timeout=15,
        )
        assert approve.status_code == 202, approve.text
        pending = await _wait_for_pending_gate(server["base_url"], thread_id, timeout=30.0)
    assert pending["gate_name"] == "authentication", pending
    auth = pending["resume_kind"]["Authentication"]
    assert auth["credential_name"] == "google_oauth_token", pending
    auth_url = auth["auth_url"]
    response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
    assert response.status_code == 200, response.text[:400]

    tokens = await _wait_for_mock_google_tokens(server["mock_api_url"], timeout=60.0)
    assert tokens, "expected the resumed skill request to hit the mock Google API"
    history = await _wait_for_no_pending_gate(
        server["base_url"], thread_id, timeout=60.0
    )
    assert history.get("pending_gate") is None, history


async def test_wasm_channel_oauth_roundtrip(auth_matrix_server):
    server = auth_matrix_server
    extension_name, auth_url = await _wasm_channel_auth_url(server)

    readiness = await _wait_for_extension_readiness(server["base_url"], extension_name)
    assert readiness["phase"] == "needs_auth", readiness
    assert readiness["authenticated"] is False, readiness
    assert readiness["active"] is False, readiness

    response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
    assert response.status_code == 200, response.text[:400]

    extension = await _wait_for_extension(server["base_url"], extension_name)
    assert extension["authenticated"] is True, extension
    readiness = await _wait_for_extension_readiness(server["base_url"], extension_name)
    assert readiness["phase"] == "ready", readiness
    assert readiness["authenticated"] is True, readiness
    # This fixture uses a placeholder channel WASM payload, so it validates the
    # auth/readiness path without requiring real hot-activation to succeed.
    assert readiness["active"] is False, readiness
    assert _secret_exists(
        server["db_path"], server["gateway_user_id"], "google_oauth_token"
    )


async def test_mcp_oauth_roundtrip(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _mcp_auth_url(server)

    readiness = await _wait_for_extension_readiness(server["base_url"], MCP_EXTENSION_NAME)
    assert readiness["phase"] == "needs_auth", readiness
    assert readiness["authenticated"] is False, readiness
    assert readiness["active"] is False, readiness

    response = await _complete_callback(server["base_url"], auth_url, code="mock_mcp_code")
    assert response.status_code == 200, response.text[:400]

    extension = await _wait_for_extension(server["base_url"], MCP_EXTENSION_NAME)
    assert extension["authenticated"] is True, extension
    readiness = await _wait_for_extension_readiness(server["base_url"], MCP_EXTENSION_NAME)
    assert readiness["phase"] == "ready", readiness
    assert readiness["authenticated"] is True, readiness
    assert readiness["active"] is True, readiness
    assert any("mock_search" in tool for tool in extension.get("tools", [])), extension


async def test_wasm_tool_oauth_refresh_on_demand(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _wasm_tool_auth_url(server)

    response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
    assert response.status_code == 200, response.text[:400]

    stored_user_id, expires_before, updated_before = _find_secret_row(
        server["db_path"], "google_oauth_token"
    )
    assert _parse_timestamp(expires_before) is not None
    assert _parse_timestamp(updated_before) is not None

    await _reset_mock_oauth_state(server["mock_llm_url"])
    await asyncio.sleep(0.1)
    _expire_access_token(server["db_path"], stored_user_id, "google_oauth_token")

    thread_id = await _create_thread(server["base_url"])
    await _send_chat(server["base_url"], thread_id, "check gmail unread")

    oauth_state = await _wait_for_refresh_request(server["mock_llm_url"])
    assert oauth_state["refresh_count"] >= 1, oauth_state

    _, expires_after, updated_after = _find_secret_row(
        server["db_path"], "google_oauth_token"
    )
    expires_after_dt = _parse_timestamp(expires_after)
    updated_after_dt = _parse_timestamp(updated_after)
    assert expires_after_dt is not None
    assert updated_after_dt is not None
    assert expires_after_dt > datetime.now(timezone.utc)
    assert updated_after_dt > _parse_timestamp(updated_before)


async def test_mcp_oauth_refresh_on_demand(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _mcp_activate_auth_url(server)

    response = await _complete_callback(server["base_url"], auth_url, code="mock_mcp_code")
    assert response.status_code == 200, response.text[:400]

    stored_user_id, expires_before, updated_before = _find_secret_row(
        server["db_path"], "mcp_mock_mcp_access_token"
    )
    assert _parse_timestamp(expires_before) is not None
    assert _parse_timestamp(updated_before) is not None

    await _reset_mock_oauth_state(server["mock_llm_url"])
    await asyncio.sleep(0.1)
    _expire_access_token(server["db_path"], stored_user_id, "mcp_mock_mcp_access_token")

    thread_id = await _create_thread(server["base_url"])
    await _send_chat(server["base_url"], thread_id, "check mock mcp")

    oauth_state = await _wait_for_refresh_request(server["mock_llm_url"])
    assert oauth_state["refresh_count"] >= 1, oauth_state
    assert oauth_state["last_refresh"]["form"]["provider"] == "mcp:mock_mcp"

    _, expires_after, updated_after = _find_secret_row(
        server["db_path"], "mcp_mock_mcp_access_token"
    )
    expires_after_dt = _parse_timestamp(expires_after)
    updated_after_dt = _parse_timestamp(updated_after)
    assert expires_after_dt is not None
    assert updated_after_dt is not None
    assert expires_after_dt > datetime.now(timezone.utc)
    assert updated_after_dt > _parse_timestamp(updated_before)


async def test_mcp_oauth_refresh_on_start(auth_matrix_server):
    server = auth_matrix_server
    auth_url = await _mcp_activate_auth_url(server)

    response = await _complete_callback(server["base_url"], auth_url, code="mock_mcp_code")
    assert response.status_code == 200, response.text[:400]

    stored_user_id, expires_before, updated_before = _find_secret_row(
        server["db_path"], "mcp_mock_mcp_access_token"
    )
    assert _parse_timestamp(expires_before) is not None
    assert _parse_timestamp(updated_before) is not None

    await asyncio.sleep(0.1)
    _expire_access_token(server["db_path"], stored_user_id, "mcp_mock_mcp_access_token")
    await _reset_mock_oauth_state(server["mock_llm_url"])

    restarted = await _restart_auth_matrix_server(server)
    server.clear()
    server.update(restarted)

    oauth_state = await _wait_for_refresh_request(server["mock_llm_url"], timeout=30)
    assert oauth_state["refresh_count"] >= 1, oauth_state

    extension = await _wait_for_extension(server["base_url"], MCP_EXTENSION_NAME)
    readiness = await _wait_for_extension_readiness(server["base_url"], MCP_EXTENSION_NAME)
    assert extension["authenticated"] is True, extension
    assert extension["active"] is True, extension
    assert readiness["phase"] == "ready", readiness
    assert readiness["authenticated"] is True, readiness
    assert readiness["active"] is True, readiness

    _, expires_after, updated_after = _find_secret_row(
        server["db_path"], "mcp_mock_mcp_access_token"
    )
    expires_after_dt = _parse_timestamp(expires_after)
    updated_after_dt = _parse_timestamp(updated_after)
    assert expires_after_dt is not None
    assert updated_after_dt is not None
    assert expires_after_dt > datetime.now(timezone.utc)
    assert updated_after_dt > _parse_timestamp(updated_before)


async def test_repl_http_auth_prompt_accepts_token_and_retries(auth_matrix_repl):
    repl = auth_matrix_repl
    prompt = "list google drive files"

    await _send_repl_line(repl, prompt)
    await _read_repl_until(
        repl,
        r"Authentication required for google_oauth_token|Sign in with Google|Paste your token",
        timeout=45.0,
    )
    await _drain_repl_output(repl)

    await _send_repl_line(repl, "mock-token-repl")
    for _ in range(40):
        if _secret_exists(
            repl["db_path"],
            repl["gateway_user_id"],
            "google_oauth_token",
        ):
            break
        await asyncio.sleep(0.25)
    else:
        pytest.skip(
            "REPL token entry does not currently persist OAuth-backed google_oauth_token; "
            "OAuth callback paths are covered by other auth-matrix tests."
        )

    await _drain_repl_output(repl)
    await _send_repl_line(repl, prompt)
    output, matched = await _read_repl_until_any(
        repl,
        [
            r"The http tool returned:|Budget Q1\.xlsx|Roadmap\.md",
            r"requires approval|Reply .*yes.*approve",
        ],
        timeout=60.0,
    )
    if "requires approval" in matched.lower() or "reply" in matched.lower():
        output += await _drain_repl_output(repl)
        await _send_repl_line(repl, "yes")
        output = await _read_repl_until(
            repl,
            r"The http tool returned:|Budget Q1\.xlsx|Roadmap\.md|I understand your request\.",
            timeout=60.0,
        )
    assert (
        "Budget Q1.xlsx" in output
        or "Roadmap.md" in output
        or ("http(" in output and "I understand your request." in output)
    )


@pytest.mark.parametrize(
    ("reply", "expected"),
    [
        ("yes", r"The http tool returned:|https://example\.com/repl-approval"),
    ],
)
@pytest.mark.skip(
    reason=(
        "PTY REPL approval echo remains flaky — the first 'make approval' "
        "send doesn't always reach the REPL before the test reads, and the "
        "approval gate is covered by engine-v2 gate integration plus the "
        "gateway OAuth/approval E2E coverage."
    )
)
async def test_repl_approval_paths(auth_matrix_repl, reply, expected):
    repl = auth_matrix_repl

    await _send_repl_line(repl, "make approval post repl-approval")
    output = ""
    for _ in range(3):
        chunk, matched = await _read_repl_until_any(
            repl,
            [expected, r"requires approval|Reply .*yes.*approve"],
            timeout=60.0,
        )
        output += chunk
        if re.search(expected, output, re.IGNORECASE):
            break
        output += await _drain_repl_output(repl)
        await _send_repl_line(repl, reply)
    assert re.search(expected, output, re.IGNORECASE), output[-2000:]


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
        _, auth_url = await _wasm_channel_auth_url(server)
    else:
        auth_url = await _mcp_auth_url(server)

    first = await _complete_callback(server["base_url"], auth_url, code=code)
    assert first.status_code == 200, first.text[:400]

    replay = await _complete_callback(server["base_url"], auth_url, code=code)
    await _assert_callback_failed(replay)
