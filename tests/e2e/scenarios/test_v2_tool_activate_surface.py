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
