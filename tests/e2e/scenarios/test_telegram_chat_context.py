"""Playwright regression coverage for Telegram-aware chat after activation."""

import asyncio
import json
import os
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import time
from pathlib import Path

import httpx
import pytest
from playwright.async_api import async_playwright

from conftest import _forward_coverage_env, _reserve_loopback_sockets, _stop_process
from helpers import (
    AUTH_TOKEN,
    HTTP_WEBHOOK_SECRET,
    OWNER_SCOPE_ID,
    SEL,
    wait_for_ready,
)

ROOT = Path(__file__).resolve().parents[3]


def _main_worktree_root() -> Path | None:
    try:
        output = subprocess.check_output(
            ["git", "worktree", "list", "--porcelain"],
            cwd=ROOT,
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except Exception:
        return None

    for line in output.splitlines():
        if line.startswith("worktree "):
            return Path(line.split(" ", 1)[1])
    return None


def _locate_telegram_artifacts() -> tuple[Path, Path]:
    roots = [ROOT]
    main_root = _main_worktree_root()
    if main_root is not None and main_root not in roots:
        roots.append(main_root)

    for base in roots:
        channel_dir = base / "channels-src" / "telegram"
        caps_path = channel_dir / "telegram.capabilities.json"
        flat_wasm = channel_dir / "telegram.wasm"
        if flat_wasm.exists() and caps_path.exists():
            return flat_wasm, caps_path

        for wasm_path in channel_dir.glob("target/*/release/telegram_channel.wasm"):
            if wasm_path.exists() and caps_path.exists():
                return wasm_path, caps_path

    raise FileNotFoundError("Could not locate bundled Telegram channel artifacts")


def _seed_telegram_owner_binding(db_path: Path) -> None:
    conn = sqlite3.connect(db_path)
    try:
        conn.execute(
            """
            INSERT OR REPLACE INTO settings (user_id, key, value, updated_at)
            VALUES (?, ?, ?, CURRENT_TIMESTAMP)
            """,
            (
                OWNER_SCOPE_ID,
                "channels.wasm_channel_owner_ids.telegram",
                json.dumps(424242),
            ),
        )
        conn.execute(
            """
            INSERT OR REPLACE INTO settings (user_id, key, value, updated_at)
            VALUES (?, ?, ?, CURRENT_TIMESTAMP)
            """,
            (
                OWNER_SCOPE_ID,
                "channels.wasm_channel_bot_usernames.telegram",
                json.dumps("test_hot_bot"),
            ),
        )
        conn.commit()
    finally:
        conn.close()


async def _wait_for_authed_threads(base_url: str, timeout: float = 30.0) -> None:
    deadline = time.monotonic() + timeout
    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            try:
                response = await client.get(
                    f"{base_url}/api/chat/threads",
                    headers=headers,
                    timeout=5,
                )
                if response.status_code == 200:
                    return
            except (httpx.ConnectError, httpx.ReadError, httpx.TimeoutException):
                pass
            await asyncio.sleep(0.5)
    raise TimeoutError(f"Authenticated threads endpoint not ready after {timeout}s")


async def _send_and_get_response(
    page,
    message: str,
    *,
    expected_fragment: str,
    timeout: int = 20000,
) -> str:
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    assistant_sel = SEL["message_assistant"]
    before_count = await page.locator(assistant_sel).count()

    await chat_input.fill(message)
    await chat_input.press("Enter")

    expected_count = before_count + 1
    await page.wait_for_function(
        """({ assistantSelector, expectedCount, expectedFragment }) => {
            const messages = document.querySelectorAll(assistantSelector);
            if (messages.length < expectedCount) return false;
            const text = (messages[messages.length - 1].innerText || '').trim().toLowerCase();
            return text.includes(expectedFragment.toLowerCase());
        }""",
        arg={
            "assistantSelector": assistant_sel,
            "expectedCount": expected_count,
            "expectedFragment": expected_fragment,
        },
        timeout=timeout,
    )

    return await page.locator(assistant_sel).last.inner_text()


@pytest.fixture(scope="module")
async def telegram_prompt_server(ironclaw_binary, mock_llm_server):
    reserved = _reserve_loopback_sockets(2)
    gateway_port = reserved[0].getsockname()[1]
    http_port = reserved[1].getsockname()[1]

    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-telegram-home-")
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-telegram-db-")
    wasm_tools_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-telegram-tools-")
    wasm_channels_tmpdir = tempfile.TemporaryDirectory(
        prefix="ironclaw-e2e-telegram-channels-"
    )

    db_path = Path(db_tmpdir.name) / "telegram-context.db"
    wasm_channels_dir = Path(wasm_channels_tmpdir.name)
    telegram_wasm, telegram_caps = _locate_telegram_artifacts()
    shutil.copy2(telegram_wasm, wasm_channels_dir / "telegram.wasm")
    shutil.copy2(telegram_caps, wasm_channels_dir / "telegram.capabilities.json")

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_tmpdir.name,
        "IRONCLAW_BASE_DIR": os.path.join(home_tmpdir.name, ".ironclaw"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": OWNER_SCOPE_ID,
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": str(db_path),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "true",
        "WASM_TOOLS_DIR": wasm_tools_tmpdir.name,
        "WASM_CHANNELS_DIR": wasm_channels_tmpdir.name,
        "ONBOARD_COMPLETED": "true",
        "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
        "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
    }
    _forward_coverage_env(env)

    for sock in reserved:
        if sock.fileno() != -1:
            sock.close()

    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary,
        "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )

    base_url = f"http://127.0.0.1:{gateway_port}"
    startup_kill_attempted = False
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=60)
        _seed_telegram_owner_binding(db_path)
        await _wait_for_authed_threads(base_url, timeout=30)
        yield base_url
    except TimeoutError:
        if proc.returncode is None:
            startup_kill_attempted = True
            await _stop_process(proc, timeout=2)
        returncode = proc.returncode
        stderr_bytes = b""
        if proc.stderr:
            try:
                stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
            except (asyncio.TimeoutError, Exception):
                pass
        stderr_text = stderr_bytes.decode("utf-8", errors="replace")
        pytest.fail(
            "telegram prompt server failed to start on "
            f"port {gateway_port} (returncode={returncode}).\nstderr:\n{stderr_text}"
        )
    finally:
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()
        if proc.returncode is None:
            if startup_kill_attempted:
                await _stop_process(proc, timeout=2)
            else:
                await _stop_process(proc, sig=signal.SIGINT, timeout=10)
                if proc.returncode is None:
                    await _stop_process(proc, timeout=2)
        home_tmpdir.cleanup()
        db_tmpdir.cleanup()
        wasm_tools_tmpdir.cleanup()
        wasm_channels_tmpdir.cleanup()


@pytest.fixture
async def telegram_prompt_page(telegram_prompt_server):
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(headless=True)
        context = await browser.new_context(viewport={"width": 1280, "height": 720})
        page = await context.new_page()
        await page.goto(f"{telegram_prompt_server}/?token={AUTH_TOKEN}")
        await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
        yield page
        await context.close()
        await browser.close()


async def test_chat_recognizes_active_telegram_channel(telegram_prompt_page):
    response = await _send_and_get_response(
        telegram_prompt_page,
        "is telegram connected?",
        expected_fragment="telegram is already connected",
    )

    lower = response.lower()
    assert "already connected" in lower
    assert "activate telegram" not in lower
    assert "please activate telegram" not in lower


async def test_chat_does_not_loop_back_to_telegram_setup(telegram_prompt_page):
    response = await _send_and_get_response(
        telegram_prompt_page,
        "message me on telegram",
        expected_fragment="already active",
    )

    lower = response.lower()
    assert "already active" in lower
    assert "enable it again" not in lower
    assert "please activate telegram" not in lower
