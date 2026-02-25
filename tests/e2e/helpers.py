"""Shared helpers for E2E tests."""

import asyncio
import re
import time

import httpx

# -- DOM Selectors --------------------------------------------------------
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
    "skill_installed": "#skills-list .ext-card",
}

TABS = ["chat", "memory", "jobs", "routines", "extensions", "skills"]

# Auth token used across all tests
AUTH_TOKEN = "e2e-test-token"


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
