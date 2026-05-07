"""E2E test: v2 tool activation surface.

Verifies the simplified model-facing contract:
- `tool_activate` is the surfaced enablement tool in engine v2
- `tool_auth` and `tool_install` are not part of the normal surfaced prompt
- blocked managed integrations appear in `Activatable Integrations`
"""

import asyncio
import json
import os
import signal
import socket
import tempfile
from urllib.parse import parse_qs, urlparse

import httpx
import pytest

import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import api_get, api_post, AUTH_TOKEN, wait_for_ready


_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-activate-surface-db-")
_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-activate-surface-home-")


def _forward_coverage_env(env: dict):
    for key in os.environ:
        if key.startswith(
            ("CARGO_LLVM_COV", "LLVM_", "CARGO_ENCODED_RUSTFLAGS", "CARGO_INCREMENTAL")
        ):
            env[key] = os.environ[key]


async def _pin_mock_llm_settings(base_url: str, mock_llm_server: str) -> None:
    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    writes = [
        ("llm_backend", "openai_compatible"),
        ("openai_compatible_base_url", mock_llm_server),
        ("selected_model", "mock-model"),
    ]
    async with httpx.AsyncClient() as client:
        for key, value in writes:
            response = await client.put(
                f"{base_url}/api/settings/{key}",
                headers=headers,
                json={"value": value},
                timeout=15,
            )
            assert response.status_code in (200, 201, 204), (
                f"failed to pin {key}: {response.status_code} {response.text[:300]}"
            )


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
    received_requests: list[str] = []
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
                    {"name": "To", "value": "surface@example.com"},
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

    def _record_request(request: web.Request) -> None:
        received_requests.append(f"{request.method} {request.path}")

    async def handle_userinfo(request: web.Request) -> web.Response:
        _record_request(request)
        return web.json_response({"email": "surface@example.com", "name": "Surface User"})

    async def handle_gmail_messages(request: web.Request) -> web.Response:
        _record_request(request)
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
        _record_request(request)
        if _authorized(request) is None:
            return web.json_response({"error": "missing_auth"}, status=401)
        message_id = request.match_info["message_id"]
        message = next((item for item in messages if item["id"] == message_id), None)
        if message is None:
            return web.json_response({"error": "not_found"}, status=404)
        return web.json_response(message)

    async def handle_received_tokens(request: web.Request) -> web.Response:
        return web.json_response({"tokens": received_tokens})

    async def handle_received_requests(request: web.Request) -> web.Response:
        return web.json_response({"requests": received_requests})

    async def handle_reset(request: web.Request) -> web.Response:
        received_tokens.clear()
        received_requests.clear()
        return web.json_response({"ok": True})

    app = web.Application()
    app.router.add_get("/oauth2/v1/userinfo", handle_userinfo)
    app.router.add_get("/oauth2/v2/userinfo", handle_userinfo)
    app.router.add_get("/gmail/v1/users/me/messages", handle_gmail_messages)
    app.router.add_get("/gmail/v1/users/me/messages/{message_id}", handle_gmail_message)
    app.router.add_get("/__mock/received-tokens", handle_received_tokens)
    app.router.add_get("/__mock/received-requests", handle_received_requests)
    app.router.add_post("/__mock/reset", handle_reset)

    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = site._server.sockets[0].getsockname()[1]
    return {
        "base_url": f"http://127.0.0.1:{port}",
        "runner": runner,
    }


def _extract_state(auth_url: str) -> str:
    parsed = urlparse(auth_url)
    state = parse_qs(parsed.query).get("state", [None])[0]
    assert state, f"auth_url missing state: {auth_url}"
    return state


async def _wait_for_response_contains(
    base_url: str,
    thread_id: str,
    needle: str,
    *,
    timeout: float = 45.0,
) -> dict:
    for _ in range(int(timeout * 2)):
        response = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15)
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
            response = await client.get(f"{mock_api_url}/__mock/received-tokens", timeout=15)
            response.raise_for_status()
            tokens = response.json().get("tokens", [])
            if tokens:
                return tokens
            await asyncio.sleep(0.5)
    raise AssertionError("Timed out waiting for Gmail HTTP execution against the mock API")


async def _get_mock_google_requests(mock_api_url: str) -> list[str]:
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{mock_api_url}/__mock/received-requests", timeout=15)
    response.raise_for_status()
    return response.json().get("requests", [])


async def _reset_mock_google_state(mock_api_url: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(f"{mock_api_url}/__mock/reset", timeout=10)
    response.raise_for_status()


async def _wait_for_tool_call(
    base_url: str,
    thread_id: str,
    tool_name: str,
    timeout: float = 30.0,
) -> dict:
    approved_request_ids = set()
    for _ in range(int(timeout * 2)):
        response = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15)
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


async def _gmail_setup_auth_url(base_url: str) -> str:
    response = await api_post(base_url, "/api/extensions/gmail/setup", json={}, timeout=30)
    assert response.status_code == 200, response.text
    auth_url = response.json().get("auth_url")
    assert auth_url, response.text
    return auth_url


@pytest.fixture(scope="module")
async def v2_activate_surface_server(ironclaw_binary, mock_llm_server, wasm_tools_dir):
    socks = []
    for _ in range(2):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", 0))
        socks.append(sock)
    gateway_port = socks[0].getsockname()[1]
    http_port = socks[1].getsockname()[1]
    for sock in socks:
        sock.close()

    home_dir = _HOME_TMPDIR.name
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
        "GATEWAY_USER_ID": "e2e-v2-activate-surface",
        "IRONCLAW_OWNER_ID": "e2e-v2-activate-surface",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_API_KEY": "mock-api-key",
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_DB_TMPDIR.name, "v2-activate-surface.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "true",
        "WASM_TOOLS_DIR": wasm_tools_dir,
        "ONBOARD_COMPLETED": "true",
        "SECRETS_MASTER_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
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
        await _pin_mock_llm_settings(base_url, mock_llm_server)
        yield base_url
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


@pytest.fixture(scope="module")
async def v2_activate_surface_auth_server(ironclaw_binary, mock_llm_server, wasm_tools_dir):
    mock_api = await _start_mock_google_api()

    socks = []
    for _ in range(2):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", 0))
        socks.append(sock)
    gateway_port = socks[0].getsockname()[1]
    http_port = socks[1].getsockname()[1]
    for sock in socks:
        sock.close()

    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-v2-activate-auth-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-v2-activate-auth-home-")
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_tmpdir.name,
        "IRONCLAW_BASE_DIR": os.path.join(home_tmpdir.name, ".ironclaw"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "ENGINE_V2": "true",
        "HTTP_ALLOW_LOCALHOST": "true",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-v2-activate-auth-surface",
        "IRONCLAW_OWNER_ID": "e2e-v2-activate-auth-surface",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_API_KEY": "mock-api-key",
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(db_tmpdir.name, "v2-activate-auth-surface.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "true",
        "WASM_TOOLS_DIR": wasm_tools_dir,
        "ONBOARD_COMPLETED": "true",
        "SECRETS_MASTER_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
        "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
        "IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK": "1",
        "GOOGLE_OAUTH_CLIENT_ID": "hosted-google-client-id",
        "IRONCLAW_TEST_HTTP_REMAP": (
            f"gmail.googleapis.com={mock_api['base_url']},"
            f"www.googleapis.com={mock_api['base_url']}"
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
        await _pin_mock_llm_settings(base_url, mock_llm_server)
        yield {
            "base_url": base_url,
            "mock_api_url": mock_api["base_url"],
        }
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)
        await mock_api["runner"].cleanup()
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()


async def _wait_for_engine_system_prompt(
    base_url: str,
    *,
    goal_substring: str,
    timeout: float = 45.0,
):
    last_threads = []
    last_detail = {}
    for _ in range(int(timeout * 2)):
        threads_response = await api_get(base_url, "/api/engine/threads", timeout=15)
        threads_response.raise_for_status()
        threads = threads_response.json().get("threads", [])
        last_threads = threads
        matches = [
            thread
            for thread in threads
            if goal_substring.lower() in (thread.get("goal") or "").lower()
        ]
        matches.sort(key=lambda thread: thread.get("updated_at") or "")

        for match in reversed(matches):
            detail_response = await api_get(
                base_url,
                f"/api/engine/threads/{match['id']}",
                timeout=15,
            )
            detail_response.raise_for_status()
            detail = detail_response.json().get("thread", {})
            last_detail = detail
            for message in detail.get("messages", []):
                if message.get("role") == "System" and message.get("content"):
                    return message["content"]

        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for engine system prompt for goal containing {goal_substring!r}. "
        f"Last threads: {json.dumps(last_threads)[:1200]}; "
        f"Last detail: {json.dumps(last_detail)[:1200]}"
    )


async def _get_extension(base_url: str, name: str):
    response = await api_get(base_url, "/api/extensions", timeout=30)
    response.raise_for_status()
    payload = response.json()
    for extension in payload.get("extensions", []):
        if extension.get("name") == name:
            return extension
    return None


async def _ensure_removed(base_url: str, name: str):
    extension = await _get_extension(base_url, name)
    if extension:
        await api_post(base_url, f"/api/extensions/{name}/remove", timeout=30)


async def test_tool_activate_is_the_visible_enablement_tool(v2_activate_surface_server):
    goal = "baseline activate surface e2e"
    thread_response = await api_post(v2_activate_surface_server, "/api/chat/thread/new", timeout=15)
    thread_id = thread_response.json()["id"]

    await api_post(
        v2_activate_surface_server,
        "/api/chat/send",
        json={"content": goal, "thread_id": thread_id},
        timeout=30,
    )

    system_prompt = await _wait_for_engine_system_prompt(
        v2_activate_surface_server,
        goal_substring=goal,
        timeout=45,
    )
    assert 'tool_activate(name="<integration>")' in system_prompt
    assert 'tool_info(name="<tool>", detail="summary")' in system_prompt
    assert "tool_auth" not in system_prompt
    assert "tool_install" not in system_prompt


async def test_blocked_integration_surfaces_in_activatable_section(v2_activate_surface_server):
    goal = "gmail activate surface e2e"
    await _ensure_removed(v2_activate_surface_server, "gmail")

    try:
        install = await api_post(
            v2_activate_surface_server,
            "/api/extensions/install",
            json={"name": "gmail"},
            timeout=180,
        )
        assert install.status_code == 200, install.text
        payload = install.json()
        assert payload.get("success") is True, payload

        thread_response = await api_post(
            v2_activate_surface_server,
            "/api/chat/thread/new",
            timeout=15,
        )
        thread_id = thread_response.json()["id"]

        await api_post(
            v2_activate_surface_server,
            "/api/chat/send",
            json={"content": goal, "thread_id": thread_id},
            timeout=30,
        )

        system_prompt = await _wait_for_engine_system_prompt(
            v2_activate_surface_server,
            goal_substring=goal,
            timeout=45,
        )
        assert "## Activatable Integrations" in system_prompt
        assert 'tool_activate(name="<integration>")' in system_prompt
        assert 'tool_info(name="<tool>", detail="summary")' in system_prompt
        assert "`gmail` [provider]" in system_prompt
        assert "tool_install" not in system_prompt
    finally:
        await _ensure_removed(v2_activate_surface_server, "gmail")


async def test_blocked_gmail_auth_blocks_upstream_requests_before_auth(
    v2_activate_surface_auth_server,
):
    server = v2_activate_surface_auth_server
    await _ensure_removed(server["base_url"], "gmail")

    try:
        await _reset_mock_google_state(server["mock_api_url"])

        install = await api_post(
            server["base_url"],
            "/api/extensions/install",
            json={"name": "gmail"},
            timeout=180,
        )
        assert install.status_code == 200, install.text
        assert install.json().get("success") is True, install.text

        assert await _get_mock_google_requests(server["mock_api_url"]) == []
        auth_url = await _gmail_setup_auth_url(server["base_url"])
        response = await _complete_callback(server["base_url"], auth_url, code="mock_auth_code")
        assert response.status_code == 200, response.text[:400]

        thread_response = await api_post(server["base_url"], "/api/chat/thread/new", timeout=15)
        thread_id = thread_response.json()["id"]
        send = await api_post(
            server["base_url"],
            "/api/chat/send",
            json={"content": "check gmail unread", "thread_id": thread_id},
            timeout=30,
        )
        assert send.status_code == 202, send.text

        await _wait_for_tool_call(server["base_url"], thread_id, "gmail", timeout=60.0)
        tokens = await _wait_for_mock_google_tokens(server["mock_api_url"], timeout=60.0)
        assert tokens, "expected Gmail to hit the mock Google API after auth replay"
        history = await _wait_for_response_contains(
            server["base_url"], thread_id, "Quarterly update", timeout=60.0
        )
        assert history.get("pending_gate") is None, history
    finally:
        await _ensure_removed(server["base_url"], "gmail")
