"""pytest fixtures for E2E tests.

Session-scoped: build binary, start mock LLM, start ironclaw.
Function-scoped: fresh Playwright browser page per test.
"""

import asyncio
import os
import signal
import subprocess
import sys
from pathlib import Path

import pytest

from helpers import AUTH_TOKEN, wait_for_port_line, wait_for_ready

# Project root (two levels up from tests/e2e/)
ROOT = Path(__file__).resolve().parent.parent.parent

# Ports: use high fixed ports to avoid conflicts with development instances
MOCK_LLM_PORT = 18_199
GATEWAY_PORT = 18_200


@pytest.fixture(scope="session")
def ironclaw_binary():
    """Ensure ironclaw binary is built. Returns the binary path."""
    binary = ROOT / "target" / "debug" / "ironclaw"
    if not binary.exists():
        print("Building ironclaw (this may take a while)...")
        subprocess.run(
            ["cargo", "build", "--no-default-features", "--features", "libsql"],
            cwd=ROOT,
            check=True,
            timeout=600,
        )
    assert binary.exists(), f"Binary not found at {binary}"
    return str(binary)


@pytest.fixture(scope="session")
def event_loop():
    """Create a session-scoped event loop for async fixtures."""
    loop = asyncio.new_event_loop()
    yield loop
    loop.close()


@pytest.fixture(scope="session")
async def mock_llm_server():
    """Start the mock LLM server. Yields the base URL."""
    server_script = Path(__file__).parent / "mock_llm.py"
    proc = await asyncio.create_subprocess_exec(
        sys.executable, str(server_script), "--port", str(MOCK_LLM_PORT),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    try:
        port = await wait_for_port_line(proc, r"MOCK_LLM_PORT=(\d+)", timeout=10)
        url = f"http://127.0.0.1:{port}"
        await wait_for_ready(f"{url}/v1/models", timeout=10)
        yield url
    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            await asyncio.wait_for(proc.wait(), timeout=5)
        except asyncio.TimeoutError:
            proc.kill()


@pytest.fixture(scope="session")
async def ironclaw_server(ironclaw_binary, mock_llm_server):
    """Start the ironclaw gateway. Yields the base URL."""
    env = {
        **os.environ,
        "RUST_LOG": "ironclaw=info",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(GATEWAY_PORT),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-tester",
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": ":memory:",
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        # Prevent onboarding wizard from triggering
        "ONBOARD_COMPLETED": "true",
    }
    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )
    base_url = f"http://127.0.0.1:{GATEWAY_PORT}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=60)
        yield base_url
    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            await asyncio.wait_for(proc.wait(), timeout=5)
        except asyncio.TimeoutError:
            proc.kill()


@pytest.fixture
async def page(ironclaw_server):
    """Fresh Playwright browser page, navigated to the gateway with auth."""
    from playwright.async_api import async_playwright

    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        context = await browser.new_context(viewport={"width": 1280, "height": 720})
        pg = await context.new_page()
        await pg.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}")
        # Wait for the app to initialize (auth screen hidden, SSE connected)
        await pg.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
        yield pg
        await context.close()
        await browser.close()
