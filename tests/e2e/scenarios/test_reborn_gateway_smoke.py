"""Dedicated Reborn gateway smoke E2E.

This is intentionally small. The Rust Reborn gate proves the host/runtime
architecture. This Playwright/API smoke test proves the reborn-main branch still
boots an isolated ENGINE_V2 gateway, serves the browser shell, persists a normal
chat turn, and completes a simple tool-capable turn without duplicate terminal
assistant responses.
"""

import asyncio
import base64
import os
import signal
import socket
from pathlib import Path
from uuid import uuid4

import pytest
from playwright.async_api import expect

from helpers import AUTH_TOKEN, SEL, api_get, api_post, wait_for_ready

def _find_free_port() -> int:
    """Ask the OS for an available loopback port.

    The returned port is only a startup hint; the gateway fixture retries with a
    fresh port pair if another process wins the bind race before ironclaw starts.
    """
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _read_log(path: Path, limit: int = 8192) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")[-limit:]
    except OSError:
        return ""


def _forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value


async def _stop_process(proc, *, sig=signal.SIGINT, timeout: float = 10) -> None:
    """Signal a subprocess and wait for exit without re-reading stdio pipes."""
    if proc.returncode is not None:
        return

    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return

    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await asyncio.wait_for(proc.wait(), timeout=5)


@pytest.fixture(scope="module")
async def reborn_gateway_server(ironclaw_binary, mock_llm_server, tmp_path_factory):
    """Start an isolated gateway configured for the Reborn/V2 product shell."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-gateway-home")
    db_dir = tmp_path_factory.mktemp("ironclaw-reborn-gateway-db")
    base_dir = home_dir / ".ironclaw"
    base_dir.mkdir(parents=True, exist_ok=True)

    proc = None
    base_url = None
    last_stderr = ""
    last_gateway_port = None

    for attempt in range(1, 4):
        gateway_port = _find_free_port()
        http_port = _find_free_port()
        last_gateway_port = gateway_port
        stdout_path = home_dir / f"reborn-gateway-attempt-{attempt}.stdout.log"
        stderr_path = home_dir / f"reborn-gateway-attempt-{attempt}.stderr.log"

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home_dir),
            "IRONCLAW_BASE_DIR": str(base_dir),
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
            "LIBSQL_PATH": str(db_dir / "reborn-gateway-e2e.db"),
            "SANDBOX_ENABLED": "false",
            "SKILLS_ENABLED": "false",
            "ROUTINES_ENABLED": "false",
            "HEARTBEAT_ENABLED": "false",
            "EMBEDDING_ENABLED": "false",
            "WASM_ENABLED": "false",
            "ONBOARD_COMPLETED": "true",
        }
        _forward_coverage_env(env)

        with stdout_path.open("wb") as stdout_file, stderr_path.open("wb") as stderr_file:
            proc = await asyncio.create_subprocess_exec(
                ironclaw_binary,
                "--no-onboard",
                stdin=asyncio.subprocess.DEVNULL,
                stdout=stdout_file,
                stderr=stderr_file,
                env=env,
            )
        base_url = f"http://127.0.0.1:{gateway_port}"

        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            break
        except TimeoutError:
            if proc.returncode is None:
                await _stop_process(proc, timeout=2)
            last_stderr = _read_log(stderr_path)
            proc = None
    else:
        pytest.fail(
            "Reborn gateway smoke server failed to start after 3 attempts.\n"
            f"Last attempted port: {last_gateway_port}\n"
            f"stderr:\n{last_stderr}"
        )

    try:
        yield base_url
    finally:
        if proc is not None and proc.returncode is None:
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


# --------------------------------------------------------------------------
# WebChat v2 native attachment path (#4644)
#
# These exercise the Reborn *v2* surface (`/api/webchat/v2/*`) the React SPA
# at `/v2` uses, not the v1 `/api/chat/*` shim above. The v2 routes + SPA are
# compiled behind the `webui-v2-beta` Cargo feature, which the default e2e
# binary (`--features libsql`) does not enable, so every test here probes the
# session endpoint first and skips when v2 is not mounted — the same
# convention as the other `test_v2_*` scenarios. Build to run them:
#   cargo build --features libsql,webui-v2-beta
# --------------------------------------------------------------------------

V2_BASE = "/api/webchat/v2"
ATTACHMENT_MARKER = "IRONCLAW_ATTACHMENT_MARKER_4644"
# A minimal but structurally valid 1x1 PNG (header + IHDR/IDAT/IEND).
_PNG_1X1 = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/"
    "/i6AAAAAElFTkSuQmCC"
)


def _b64(data: bytes) -> str:
    return base64.b64encode(data).decode("ascii")


async def _require_v2(base_url: str) -> None:
    """Skip the test unless the v2 native routes are compiled in."""
    response = await api_get(base_url, f"{V2_BASE}/session", timeout=15)
    if response.status_code == 404:
        pytest.skip("webui-v2-beta routes not mounted (build with --features webui-v2-beta)")
    response.raise_for_status()


async def _create_thread_v2(base_url: str) -> str:
    response = await api_post(
        base_url,
        f"{V2_BASE}/threads",
        json={"client_action_id": str(uuid4())},
        timeout=15,
    )
    response.raise_for_status()
    return response.json()["thread"]["thread_id"]


async def _send_v2(base_url, thread_id, content, attachments=None):
    body = {"client_action_id": str(uuid4()), "content": content}
    if attachments:
        body["attachments"] = attachments
    return await api_post(
        base_url,
        f"{V2_BASE}/threads/{thread_id}/messages",
        json=body,
        timeout=30,
    )


async def _fetch_timeline_v2(base_url, thread_id) -> dict:
    response = await api_get(
        base_url,
        f"{V2_BASE}/threads/{thread_id}/timeline",
        timeout=15,
    )
    response.raise_for_status()
    return response.json()


async def _wait_for_v2_user_attachments(base_url, thread_id, *, timeout=30.0) -> list:
    for _ in range(int(timeout * 2)):
        timeline = await _fetch_timeline_v2(base_url, thread_id)
        for message in timeline.get("messages", []):
            if message.get("kind") in ("user", "user_message") and message.get("attachments"):
                return message["attachments"]
        await asyncio.sleep(0.5)
    raise AssertionError("Timed out waiting for a user message carrying attachments")


async def _wait_for_v2_assistant_reply(base_url, thread_id, *, timeout=45.0) -> str:
    for _ in range(int(timeout * 2)):
        timeline = await _fetch_timeline_v2(base_url, thread_id)
        for message in timeline.get("messages", []):
            if message.get("kind") in ("assistant", "assistant_message") and (
                message.get("content") or ""
            ).strip():
                return message["content"]
        await asyncio.sleep(0.5)
    raise AssertionError("Timed out waiting for a terminal assistant reply")


async def test_reborn_v2_attachments_land_and_persist_in_timeline(reborn_gateway_server):
    """Uploaded attachments land and the timeline returns their refs (survives refresh)."""
    await _require_v2(reborn_gateway_server)
    thread_id = await _create_thread_v2(reborn_gateway_server)

    attachments = [
        {"mime_type": "application/pdf", "filename": "report.pdf", "data_base64": _b64(b"%PDF-1.7 body")},
        {"mime_type": "text/csv", "filename": "data.csv", "data_base64": _b64(b"a,b\n1,2\n")},
        {"mime_type": "text/plain", "filename": "notes.txt", "data_base64": _b64(b"some plain notes")},
        {"mime_type": "image/png", "filename": "shot.png", "data_base64": _b64(_PNG_1X1)},
    ]
    response = await _send_v2(reborn_gateway_server, thread_id, "see attached", attachments)
    assert response.status_code in (200, 202), response.text

    refs = await _wait_for_v2_user_attachments(reborn_gateway_server, thread_id)
    by_name = {ref.get("filename"): ref for ref in refs}
    assert set(by_name) == {"report.pdf", "data.csv", "notes.txt", "shot.png"}, by_name
    assert by_name["shot.png"]["kind"] == "image"
    assert by_name["report.pdf"]["kind"] == "document"
    # The timeline ref carries a storage_key so a later turn can file_read it —
    # and the browser re-fetches this same timeline on refresh, so the cards
    # persist (the #3272 class). The bytes never ride in the ref.
    assert all(ref.get("storage_key") for ref in refs), refs


async def test_reborn_v2_attachment_text_reaches_model(reborn_gateway_server):
    """A document's extracted text is folded into the prompt the model sees."""
    await _require_v2(reborn_gateway_server)
    thread_id = await _create_thread_v2(reborn_gateway_server)

    document = f"Internal report. {ATTACHMENT_MARKER}. End of report.".encode()
    response = await _send_v2(
        reborn_gateway_server,
        thread_id,
        "summarize the attached document",
        [{"mime_type": "text/plain", "filename": "marker.txt", "data_base64": _b64(document)}],
    )
    assert response.status_code in (200, 202), response.text

    reply = await _wait_for_v2_assistant_reply(reborn_gateway_server, thread_id)
    # The mock LLM only emits this canned line when the marker (which lived
    # only inside the uploaded file) appears in the prompt — proving the
    # extracted text reached the model, not `[non_text_content]`.
    assert "read the attached document text" in reply.lower(), reply


async def test_reborn_v2_oversize_attachment_is_rejected(reborn_gateway_server):
    """An over-budget attachment is refused with a clear status, not silently dropped."""
    await _require_v2(reborn_gateway_server)
    thread_id = await _create_thread_v2(reborn_gateway_server)

    # 6 MiB decoded > the 5 MiB per-file budget; base64 (~8 MiB) stays under
    # the 14 MiB body limit, so the decode-time budget check is what fires.
    oversize = _b64(b"x" * (6 * 1024 * 1024))
    response = await _send_v2(
        reborn_gateway_server,
        thread_id,
        "too big",
        [{"mime_type": "text/plain", "filename": "big.txt", "data_base64": oversize}],
    )
    assert response.status_code in (400, 413, 422), response.text


async def test_reborn_v2_attachment_card_renders_and_survives_refresh(reborn_gateway_server, browser):
    """The SPA stages a file, renders its card in-thread, and keeps it after reload."""
    # Skip only when the v2 SPA isn't mounted (runtime probe), like the sibling
    # tests — never an unconditional skip. This test drives the reborn gateway
    # fixture (not the default `page` fixture's server), so it owns its context.
    await _require_v2(reborn_gateway_server)
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    try:
        await page.goto(f"{reborn_gateway_server}/v2/?token={AUTH_TOKEN}")
        # Stage a file through the composer's hidden picker, then send. Target
        # the multi-file picker specifically — a bare `input[type=file]` is
        # ambiguous on /v2 (the Settings toolbar has one too), which could attach
        # the file to the wrong input.
        await page.set_input_files(
            "input[type=file][multiple]",
            files=[{"name": "notes.txt", "mimeType": "text/plain", "buffer": b"hello from a file"}],
        )
        await expect(page.get_by_text("notes.txt")).to_be_visible()
        await page.locator('[data-testid="chat-composer"]').fill("look at the attached file")
        await page.get_by_role("button", name="Send message").click()

        # The card renders in the thread bubble...
        await expect(page.get_by_text("notes.txt")).to_be_visible(timeout=15000)
        # ...and survives a full reload (re-projected from the v2 timeline).
        await page.reload()
        await expect(page.get_by_text("notes.txt")).to_be_visible(timeout=15000)
    finally:
        await context.close()
