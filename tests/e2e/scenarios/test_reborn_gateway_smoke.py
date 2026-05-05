"""Dedicated Reborn gateway smoke E2E.

This is intentionally small. The Rust Reborn gate proves the host/runtime
architecture. This Playwright/API smoke test proves the reborn-main branch still
boots an isolated ENGINE_V2 gateway, serves the browser shell, persists a normal
chat turn, and completes a simple tool-capable turn without duplicate terminal
assistant responses.
"""

import asyncio
import os
import signal
import socket
import tempfile

import pytest
from playwright.async_api import expect

from helpers import AUTH_TOKEN, SEL, api_get, api_post, wait_for_ready

_REBORN_GATEWAY_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-reborn-gateway-e2e-")
_REBORN_GATEWAY_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-reborn-gateway-e2e-home-")


def _reserve_loopback_sockets(count: int) -> list[socket.socket]:
    sockets: list[socket.socket] = []
    try:
        while len(sockets) < count:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.bind(("127.0.0.1", 0))
            sockets.append(sock)
        return sockets
    except Exception:
        for sock in sockets:
            sock.close()
        raise


def _close_reserved_sockets(sockets: list[socket.socket]) -> None:
    for sock in sockets:
        if sock.fileno() != -1:
            sock.close()


def _forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value


async def _stop_process(proc, *, sig=signal.SIGINT, timeout: float = 10) -> None:
    async def _drain_pipes() -> None:
        try:
            await asyncio.wait_for(proc.communicate(), timeout=1)
        except (asyncio.TimeoutError, ValueError):
            pass

    if proc.returncode is not None:
        await _drain_pipes()
        return

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
async def reborn_gateway_server(ironclaw_binary, mock_llm_server):
    """Start an isolated gateway configured for the Reborn/V2 product shell."""
    home_dir = _REBORN_GATEWAY_HOME_TMPDIR.name
    base_dir = os.path.join(home_dir, ".ironclaw")
    os.makedirs(base_dir, exist_ok=True)
    reserved_sockets = _reserve_loopback_sockets(2)
    gateway_port = reserved_sockets[0].getsockname()[1]
    http_port = reserved_sockets[1].getsockname()[1]

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": base_dir,
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "ENGINE_V2": "true",
        "AGENT_AUTO_APPROVE_TOOLS": "true",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "reborn-gateway-e2e-user",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_API_KEY": "mock-api-key",
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_REBORN_GATEWAY_DB_TMPDIR.name, "reborn-gateway-e2e.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "false",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "false",
        "ONBOARD_COMPLETED": "true",
    }
    _forward_coverage_env(env)

    _close_reserved_sockets(reserved_sockets)
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
        yield base_url
    except TimeoutError:
        if proc.returncode is None:
            await _stop_process(proc, timeout=2)
        stderr_text = ""
        if proc.stderr:
            try:
                stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
                stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            except asyncio.TimeoutError:
                pass
        pytest.fail(
            f"Reborn gateway smoke server failed to start on port {gateway_port}.\n"
            f"stderr:\n{stderr_text}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


@pytest.fixture
async def reborn_gateway_page(reborn_gateway_server, browser):
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{reborn_gateway_server}/?token={AUTH_TOKEN}")
    await page.wait_for_selector(SEL["auth_screen"], state="hidden", timeout=15000)
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=10000,
    )
    yield page
    await context.close()


async def _create_thread(base_url: str) -> str:
    response = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    response.raise_for_status()
    return response.json()["id"]


async def _send_message(base_url: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text


async def _wait_for_terminal_turn(
    base_url: str,
    thread_id: str,
    expected_user_input: str,
    *,
    timeout: float = 45.0,
) -> dict:
    last_history = {}
    for _ in range(int(timeout * 2)):
        response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        response.raise_for_status()
        history = response.json()
        last_history = history
        turns = history.get("turns", [])
        matching_turns = [
            turn
            for turn in turns
            if expected_user_input in (turn.get("user_input") or "")
        ]
        if matching_turns and (matching_turns[-1].get("response") or "").strip():
            return matching_turns[-1]
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for terminal turn containing {expected_user_input!r}. "
        f"Last history: {last_history}"
    )


async def test_reborn_gateway_loads_engine_v2_shell(reborn_gateway_page):
    """The isolated Reborn smoke gateway should boot the ENGINE_V2 shell."""
    chat_tab = reborn_gateway_page.locator(SEL["tab_button"].format(tab="chat"))
    missions_tab = reborn_gateway_page.locator(SEL["tab_button"].format(tab="missions"))
    routines_tab = reborn_gateway_page.locator(SEL["tab_button"].format(tab="routines"))

    await expect(chat_tab).to_be_visible()
    await expect(missions_tab).to_be_visible()
    await expect(routines_tab).to_be_hidden()


async def test_reborn_gateway_persists_text_and_tool_turns_without_duplicate_response(
    reborn_gateway_server,
):
    """A text turn and an auto-approved tool turn should each produce one terminal response."""
    thread_id = await _create_thread(reborn_gateway_server)

    text_prompt = "reborn gateway smoke: what is 2+2?"
    await _send_message(reborn_gateway_server, thread_id, text_prompt)
    text_turn = await _wait_for_terminal_turn(reborn_gateway_server, thread_id, text_prompt)
    assert "4" in text_turn.get("response", "")

    tool_prompt = "echo reborn gateway smoke tool result"
    await _send_message(reborn_gateway_server, thread_id, tool_prompt)
    tool_turn = await _wait_for_terminal_turn(reborn_gateway_server, thread_id, tool_prompt)
    assert "reborn gateway smoke tool result" in tool_turn.get("response", "").lower()

    tool_calls = tool_turn.get("tool_calls", [])
    assert tool_calls, f"Expected persisted tool call metadata, got: {tool_turn}"
    assert any(call.get("name") == "echo" and call.get("has_result") for call in tool_calls)

    history_response = await api_get(
        reborn_gateway_server,
        f"/api/chat/history?thread_id={thread_id}",
        timeout=15,
    )
    history_response.raise_for_status()
    matching_tool_turns = [
        turn
        for turn in history_response.json().get("turns", [])
        if tool_prompt in (turn.get("user_input") or "")
        and (turn.get("response") or "").strip()
    ]
    assert len(matching_tool_turns) == 1, (
        "Expected one terminal assistant response for the tool prompt, got "
        f"{len(matching_tool_turns)} turns: {matching_tool_turns}"
    )
