# E2E Testing Infrastructure Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Python + Playwright E2E testing framework that exercises the IronClaw web gateway through a real browser against the real binary with a mock LLM backend.

**Architecture:** pytest session fixtures start a mock OpenAI-compat HTTP server and the ironclaw binary (libSQL in-memory, gateway enabled), then per-test Playwright browser instances navigate to the gateway and make DOM assertions.

**Tech Stack:** Python 3.11+, pytest, pytest-asyncio, playwright, aiohttp

**Design doc:** `docs/plans/2026-02-24-e2e-infrastructure-design.md`

---

### Task 1: Project scaffolding and pyproject.toml

**Files:**
- Create: `tests/e2e/pyproject.toml`
- Create: `tests/e2e/scenarios/__init__.py`

**Step 1: Create pyproject.toml**

```toml
[project]
name = "ironclaw-e2e"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = [
    "pytest>=8.0",
    "pytest-asyncio>=0.23",
    "pytest-playwright>=0.5",
    "playwright>=1.40",
    "aiohttp>=3.9",
    "httpx>=0.27",
]

[project.optional-dependencies]
vision = [
    "anthropic>=0.40",
]

[tool.pytest.ini_options]
asyncio_mode = "auto"
timeout = 120
```

**Step 2: Create empty __init__.py**

Create `tests/e2e/scenarios/__init__.py` as an empty file.

**Step 3: Verify install works**

Run:
```bash
cd tests/e2e && pip install -e . && playwright install chromium
```
Expected: Clean install, no errors.

**Step 4: Commit**

```bash
git add tests/e2e/pyproject.toml tests/e2e/scenarios/__init__.py
git commit -m "scaffold: E2E test project with pyproject.toml"
```

---

### Task 2: Mock LLM server

**Files:**
- Create: `tests/e2e/mock_llm.py`

**Step 1: Write the mock LLM server**

The server must:
- Listen on `127.0.0.1` with a port passed via `--port` CLI arg (default 0 for OS-assigned)
- Print `MOCK_LLM_PORT={port}` to stdout on startup (for fixture to parse)
- Handle `POST /v1/chat/completions` with both streaming and non-streaming modes
- Handle `GET /v1/models` for health checks
- Pattern-match the last user message to select canned responses
- Support `stream: true` with proper SSE chunk format (critical for IronClaw's streaming)

```python
"""Mock OpenAI-compatible LLM server for E2E tests."""

import argparse
import json
import re
import time
import uuid

from aiohttp import web

CANNED_RESPONSES = [
    (re.compile(r"hello|hi|hey", re.IGNORECASE), "Hello! How can I help you today?"),
    (re.compile(r"2\s*\+\s*2|two plus two", re.IGNORECASE), "The answer is 4."),
    (re.compile(r"skill|install", re.IGNORECASE), "I can help you with skills management."),
]
DEFAULT_RESPONSE = "I understand your request."


def match_response(messages: list[dict]) -> str:
    """Find canned response for the last user message."""
    for msg in reversed(messages):
        if msg.get("role") == "user":
            content = msg.get("content", "")
            # Handle content that may be a list (multi-modal)
            if isinstance(content, list):
                content = " ".join(
                    part.get("text", "") for part in content if part.get("type") == "text"
                )
            for pattern, response in CANNED_RESPONSES:
                if pattern.search(content):
                    return response
            return DEFAULT_RESPONSE
    return DEFAULT_RESPONSE


async def chat_completions(request: web.Request) -> web.StreamResponse:
    """Handle POST /v1/chat/completions."""
    body = await request.json()
    messages = body.get("messages", [])
    stream = body.get("stream", False)
    response_text = match_response(messages)
    completion_id = f"mock-{uuid.uuid4().hex[:8]}"

    if not stream:
        return web.json_response({
            "id": completion_id,
            "object": "chat.completion",
            "created": int(time.time()),
            "model": "mock-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": response_text},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": len(response_text.split()), "total_tokens": 15},
        })

    # Streaming response: split into word-boundary chunks
    resp = web.StreamResponse(
        status=200,
        headers={"Content-Type": "text/event-stream", "Cache-Control": "no-cache"},
    )
    await resp.prepare(request)

    # First chunk: role
    chunk = {
        "id": completion_id,
        "object": "chat.completion.chunk",
        "created": int(time.time()),
        "model": "mock-model",
        "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": None}],
    }
    await resp.write(f"data: {json.dumps(chunk)}\n\n".encode())

    # Content chunks: split on spaces
    words = response_text.split(" ")
    for i, word in enumerate(words):
        text = word if i == 0 else f" {word}"
        chunk["choices"][0]["delta"] = {"content": text}
        await resp.write(f"data: {json.dumps(chunk)}\n\n".encode())

    # Final chunk: finish_reason
    chunk["choices"][0]["delta"] = {}
    chunk["choices"][0]["finish_reason"] = "stop"
    await resp.write(f"data: {json.dumps(chunk)}\n\n".encode())
    await resp.write(b"data: [DONE]\n\n")

    return resp


async def models(_request: web.Request) -> web.Response:
    """Handle GET /v1/models."""
    return web.json_response({
        "object": "list",
        "data": [{"id": "mock-model", "object": "model", "owned_by": "test"}],
    })


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()

    app = web.Application()
    app.router.add_post("/v1/chat/completions", chat_completions)
    app.router.add_get("/v1/models", models)

    # Use aiohttp's runner to get the actual bound port
    import asyncio

    async def start():
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        # Extract the actual port from the bound socket
        port = site._server.sockets[0].getsockname()[1]
        print(f"MOCK_LLM_PORT={port}", flush=True)
        # Block forever
        await asyncio.Event().wait()

    asyncio.run(start())


if __name__ == "__main__":
    main()
```

**Step 2: Verify it starts and responds**

Run:
```bash
python tests/e2e/mock_llm.py --port 18080 &
curl -s http://127.0.0.1:18080/v1/models | python -m json.tool
curl -s -X POST http://127.0.0.1:18080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"What is 2+2?"}],"model":"mock"}'
kill %1
```

Expected: Models endpoint returns `{"data": [{"id": "mock-model", ...}]}`. Chat returns response containing "4".

**Step 3: Verify streaming**

```bash
python tests/e2e/mock_llm.py --port 18080 &
curl -sN -X POST http://127.0.0.1:18080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Hello"}],"model":"mock","stream":true}'
kill %1
```

Expected: SSE chunks ending with `data: [DONE]`.

**Step 4: Commit**

```bash
git add tests/e2e/mock_llm.py
git commit -m "feat: mock OpenAI-compat LLM server for E2E tests"
```

---

### Task 3: Helpers module

**Files:**
- Create: `tests/e2e/helpers.py`

**Step 1: Write helpers**

```python
"""Shared helpers for E2E tests."""

import asyncio
import re
import time

import httpx

# ── DOM Selectors ────────────────────────────────────────────────────────
# Keep all selectors in one place so changes to the frontend only need
# one update.

SEL = {
    # Auth
    "auth_screen": "#auth-screen",
    "token_input": "#token-input",
    # Connection
    "sse_status": "#sse-status",
    # Tabs
    "tab_button": '.tab-bar button[data-tab="{tab}"]',
    "tab_panel": "#tab-{tab}",
    # Chat
    "chat_input": "#chat-input",
    "chat_messages": "#chat-messages",
    "message_user": "#chat-messages .message.user",
    "message_assistant": "#chat-messages .message.assistant",
    # Skills
    "skill_search_input": "#skill-search-input",
    "skill_search_results": "#skill-search-results",
    "skill_search_result": ".skill-search-result",
    "skill_installed": "#installed-skills .ext-card",
}

TABS = ["chat", "memory", "jobs", "routines", "extensions", "skills"]

# Auth token used across all tests
AUTH_TOKEN = "changeme"


async def wait_for_ready(url: str, *, timeout: float = 60, interval: float = 0.5):
    """Poll a URL until it returns 200 or timeout."""
    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            try:
                resp = await client.get(url, timeout=5)
                if resp.status_code == 200:
                    return
            except (httpx.ConnectError, httpx.ReadError, httpx.TimeoutException):
                pass
            await asyncio.sleep(interval)
    raise TimeoutError(f"Service at {url} not ready after {timeout}s")


async def wait_for_port_line(process, pattern: str, *, timeout: float = 60) -> int:
    """Read process stdout line by line until a port-bearing line matches."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        try:
            line = await asyncio.wait_for(process.stdout.readline(), timeout=remaining)
        except asyncio.TimeoutError:
            break
        decoded = line.decode("utf-8", errors="replace").strip()
        if match := re.search(pattern, decoded):
            return int(match.group(1))
    raise TimeoutError(f"Port pattern '{pattern}' not found in stdout after {timeout}s")
```

**Step 2: Commit**

```bash
git add tests/e2e/helpers.py
git commit -m "feat: E2E helpers with DOM selectors and port discovery"
```

---

### Task 4: conftest.py fixtures

**Files:**
- Create: `tests/e2e/conftest.py`

**Step 1: Write the fixtures**

Key details from codebase research:
- IronClaw logs `Web UI: http://{host}:{port}/` to stdout (main.rs:508) using the config port, not the bound port. So we must use a fixed port, not port 0.
- Health endpoint: `GET /api/health` (public, no auth required)
- Auth via `?token=` query parameter for the frontend auto-auth flow
- The frontend hides `#auth-screen` when token is valid and SSE connects

```python
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
```

**Step 2: Commit**

```bash
git add tests/e2e/conftest.py
git commit -m "feat: E2E conftest with session fixtures for mock LLM and ironclaw"
```

---

### Task 5: Scenario 1 -- Connection and tab navigation

**Files:**
- Create: `tests/e2e/scenarios/test_connection.py`

**Step 1: Write the test**

```python
"""Scenario 1: Connection, auth, and tab navigation."""

import pytest
from helpers import AUTH_TOKEN, SEL, TABS


async def test_page_loads_and_connects(page):
    """After auth, the app shows Connected status and all tabs."""
    # Connection status
    status = page.locator(SEL["sse_status"])
    await status.wait_for(state="visible", timeout=10000)
    text = await status.text_content()
    assert text is not None
    assert "connect" in text.lower(), f"Expected 'Connected', got '{text}'"

    # All 6 main tabs visible
    for tab in TABS:
        btn = page.locator(SEL["tab_button"].format(tab=tab))
        assert await btn.is_visible(), f"Tab button '{tab}' not visible"


async def test_tab_navigation(page):
    """Clicking each tab shows its panel."""
    for tab in TABS:
        btn = page.locator(SEL["tab_button"].format(tab=tab))
        await btn.click()
        panel = page.locator(SEL["tab_panel"].format(tab=tab))
        await panel.wait_for(state="visible", timeout=5000)

    # Return to Chat tab
    await page.locator(SEL["tab_button"].format(tab="chat")).click()
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)


async def test_auth_rejection(page, ironclaw_server):
    """Navigating without a token shows the auth screen."""
    # Open a new page without the token
    new_page = await page.context.new_page()
    await new_page.goto(ironclaw_server)
    auth_screen = new_page.locator(SEL["auth_screen"])
    await auth_screen.wait_for(state="visible", timeout=10000)
    await new_page.close()
```

**Step 2: Verify test runs (may fail if ironclaw isn't built yet -- that's OK)**

```bash
cd tests/e2e && python -m pytest scenarios/test_connection.py -v --timeout=120
```

Expected: Tests pass if ironclaw is built, or skip/fail gracefully if not.

**Step 3: Commit**

```bash
git add tests/e2e/scenarios/test_connection.py
git commit -m "feat: E2E scenario 1 -- connection and tab navigation tests"
```

---

### Task 6: Scenario 2 -- Chat message round-trip

**Files:**
- Create: `tests/e2e/scenarios/test_chat.py`

**Step 1: Write the test**

```python
"""Scenario 2: Chat message round-trip via SSE streaming."""

import pytest
from helpers import SEL


async def test_send_message_and_receive_response(page):
    """Type a message, receive a streamed response from mock LLM."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Send message
    await chat_input.fill("What is 2+2?")
    await chat_input.press("Enter")

    # Wait for assistant response
    assistant_msg = page.locator(SEL["message_assistant"]).last
    await assistant_msg.wait_for(state="visible", timeout=15000)

    # Verify user message
    user_msgs = page.locator(SEL["message_user"])
    assert await user_msgs.count() >= 1
    last_user = user_msgs.last
    user_text = await last_user.text_content()
    assert "2+2" in user_text or "2 + 2" in user_text

    # Verify assistant response contains "4" (from mock LLM canned response)
    assistant_text = await assistant_msg.text_content()
    assert "4" in assistant_text, f"Expected '4' in response, got: '{assistant_text}'"


async def test_multiple_messages(page):
    """Send two messages, verify both get responses."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # First message
    await chat_input.fill("Hello")
    await chat_input.press("Enter")

    # Wait for first response
    await page.locator(SEL["message_assistant"]).first.wait_for(
        state="visible", timeout=15000
    )

    # Second message
    await chat_input.fill("What is 2+2?")
    await chat_input.press("Enter")

    # Wait for second response (at least 2 assistant messages)
    await page.wait_for_function(
        """() => document.querySelectorAll('#chat-messages .message.assistant').length >= 2""",
        timeout=15000,
    )

    # Verify counts
    user_count = await page.locator(SEL["message_user"]).count()
    assistant_count = await page.locator(SEL["message_assistant"]).count()
    assert user_count >= 2, f"Expected >= 2 user messages, got {user_count}"
    assert assistant_count >= 2, f"Expected >= 2 assistant messages, got {assistant_count}"


async def test_empty_message_not_sent(page):
    """Pressing Enter with empty input should not create a message."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    initial_count = await page.locator(f"{SEL['message_user']}, {SEL['message_assistant']}").count()

    # Press Enter with empty input
    await chat_input.press("Enter")

    # Wait a moment and verify no new messages
    await page.wait_for_timeout(2000)
    final_count = await page.locator(f"{SEL['message_user']}, {SEL['message_assistant']}").count()
    assert final_count == initial_count, "Empty message should not create new messages"
```

**Step 2: Commit**

```bash
git add tests/e2e/scenarios/test_chat.py
git commit -m "feat: E2E scenario 2 -- chat message round-trip tests"
```

---

### Task 7: Scenario 3 -- Skills lifecycle

**Files:**
- Create: `tests/e2e/scenarios/test_skills.py`

**Step 1: Write the test**

Note: These tests depend on ClawHub being reachable. They're marked with `@pytest.mark.skipif` if the registry is down.

```python
"""Scenario 3: Skills search, install, and remove lifecycle."""

import pytest
from helpers import SEL


async def test_skills_tab_visible(page):
    """Skills tab shows the search interface."""
    await page.locator(SEL["tab_button"].format(tab="skills")).click()
    panel = page.locator(SEL["tab_panel"].format(tab="skills"))
    await panel.wait_for(state="visible", timeout=5000)

    search_input = page.locator(SEL["skill_search_input"])
    assert await search_input.is_visible(), "Skills search input not visible"


async def test_skills_search(page):
    """Search ClawHub for skills and verify results appear."""
    await page.locator(SEL["tab_button"].format(tab="skills")).click()

    search_input = page.locator(SEL["skill_search_input"])
    await search_input.fill("markdown")
    await search_input.press("Enter")

    # Wait for results (ClawHub may be slow)
    try:
        results = page.locator(SEL["skill_search_result"])
        await results.first.wait_for(state="visible", timeout=20000)
    except Exception:
        pytest.skip("ClawHub registry unreachable or returned no results")

    count = await results.count()
    assert count >= 1, "Expected at least 1 search result"


async def test_skills_install_and_remove(page):
    """Install a skill from search results, then remove it."""
    await page.locator(SEL["tab_button"].format(tab="skills")).click()

    # Search
    search_input = page.locator(SEL["skill_search_input"])
    await search_input.fill("markdown")
    await search_input.press("Enter")

    try:
        results = page.locator(SEL["skill_search_result"])
        await results.first.wait_for(state="visible", timeout=20000)
    except Exception:
        pytest.skip("ClawHub registry unreachable or returned no results")

    # Auto-accept confirm dialogs
    await page.evaluate("window.confirm = () => true")

    # Install first result
    install_btn = results.first.locator("button", has_text="Install")
    if await install_btn.count() == 0:
        pytest.skip("No installable skills found in results")
    await install_btn.click()

    # Wait for install to complete (installed list updates)
    # The UI should show the skill in the installed section
    await page.wait_for_timeout(5000)

    # Check if any installed skills exist now
    installed = page.locator(SEL["skill_installed"])
    installed_count = await installed.count()
    if installed_count == 0:
        # Try scrolling or waiting longer
        await page.wait_for_timeout(5000)
        installed_count = await installed.count()

    assert installed_count >= 1, "Skill should appear in installed list after install"

    # Remove the skill
    remove_btn = installed.first.locator("button", has_text="Remove")
    if await remove_btn.count() > 0:
        await remove_btn.click()
        await page.wait_for_timeout(3000)

        # Verify removed
        new_count = await page.locator(SEL["skill_installed"]).count()
        assert new_count < installed_count, "Skill should be removed from installed list"
```

**Step 2: Commit**

```bash
git add tests/e2e/scenarios/test_skills.py
git commit -m "feat: E2E scenario 3 -- skills search, install, remove tests"
```

---

### Task 8: CI workflow

**Files:**
- Create: `.github/workflows/e2e.yml`

**Step 1: Write the workflow**

```yaml
name: E2E Tests
on:
  schedule:
    - cron: "0 6 * * 1"  # Weekly Monday 6 AM UTC
  workflow_dispatch:
  pull_request:
    paths:
      - "src/channels/web/**"
      - "tests/e2e/**"

jobs:
  e2e:
    name: Browser E2E
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - uses: actions/cache@v4
        with:
          path: |
            target
            ~/.cargo/registry
          key: e2e-${{ runner.os }}-${{ hashFiles('Cargo.lock') }}

      - name: Build ironclaw (libsql)
        run: cargo build --no-default-features --features libsql

      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"

      - name: Install E2E dependencies
        run: |
          cd tests/e2e
          pip install -e .
          playwright install --with-deps chromium

      - name: Run E2E tests
        run: pytest tests/e2e/ -v --timeout=120

      - name: Upload screenshots on failure
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: e2e-screenshots
          path: tests/e2e/screenshots/
          if-no-files-found: ignore
```

**Step 2: Commit**

```bash
git add .github/workflows/e2e.yml
git commit -m "ci: add weekly E2E test workflow with Playwright"
```

---

### Task 9: README

**Files:**
- Create: `tests/e2e/README.md`

**Step 1: Write the README**

```markdown
# IronClaw E2E Tests

Browser-level end-to-end tests for the IronClaw web gateway using Python + Playwright.

## Prerequisites

- Python 3.11+
- Rust toolchain (for building ironclaw)
- Chromium (installed via Playwright)

## Setup

```bash
cd tests/e2e
pip install -e .
playwright install chromium
```

## Build ironclaw

The tests need the ironclaw binary built with libsql support:

```bash
cargo build --no-default-features --features libsql
```

## Run tests

```bash
# From repo root
pytest tests/e2e/ -v

# Run a single scenario
pytest tests/e2e/scenarios/test_chat.py -v

# With visible browser (not headless)
HEADED=1 pytest tests/e2e/scenarios/test_connection.py -v
```

## Architecture

Tests start two subprocesses:
1. **Mock LLM** (`mock_llm.py`) -- fake OpenAI-compat server with canned responses
2. **IronClaw** -- the real binary with gateway enabled, pointing to the mock LLM

Then Playwright drives a headless Chromium browser against the gateway, making DOM assertions.

## Scenarios

| File | What it tests |
|------|--------------|
| `test_connection.py` | Auth, tab navigation, connection status |
| `test_chat.py` | Send message, SSE streaming, response rendering |
| `test_skills.py` | ClawHub search, skill install/remove |

## Adding new scenarios

1. Create `tests/e2e/scenarios/test_<name>.py`
2. Use the `page` fixture for a fresh browser page
3. Use selectors from `helpers.py` (update `SEL` dict if new elements are needed)
4. Keep tests deterministic -- use the mock LLM, not real providers
```

**Step 2: Commit**

```bash
git add tests/e2e/README.md
git commit -m "docs: E2E test README with setup and usage instructions"
```

---

### Task 10: Integration test -- run all scenarios end-to-end

**Step 1: Build ironclaw**

```bash
cargo build --no-default-features --features libsql
```

**Step 2: Run the full E2E suite**

```bash
pytest tests/e2e/ -v --timeout=120
```

Expected: All tests in `test_connection.py` and `test_chat.py` pass. `test_skills.py` tests pass or skip (if ClawHub is unreachable).

**Step 3: Fix any issues discovered during the run**

Common issues to watch for:
- Port conflicts: change `MOCK_LLM_PORT` or `GATEWAY_PORT` in conftest.py
- Timing: increase wait timeouts if SSE streaming is slow
- Selectors: update `SEL` dict in helpers.py if frontend elements changed
- Onboarding wizard: ensure `ONBOARD_COMPLETED=true` prevents wizard from blocking

**Step 4: Final commit with any fixes**

```bash
git add -A tests/e2e/
git commit -m "fix: E2E test adjustments from integration run"
```

---

## Summary

| Task | Files | Description |
|------|-------|-------------|
| 1 | pyproject.toml, __init__.py | Project scaffolding |
| 2 | mock_llm.py | Mock OpenAI-compat server |
| 3 | helpers.py | Selectors and utilities |
| 4 | conftest.py | pytest fixtures |
| 5 | test_connection.py | Scenario 1: connection/tabs |
| 6 | test_chat.py | Scenario 2: chat round-trip |
| 7 | test_skills.py | Scenario 3: skills lifecycle |
| 8 | e2e.yml | CI workflow |
| 9 | README.md | Documentation |
| 10 | (integration run) | Verify everything works |
