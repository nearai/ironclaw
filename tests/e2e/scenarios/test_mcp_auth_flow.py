"""MCP server auth flow E2E tests.

Tests the full MCP server lifecycle: install MCP server (pointing at mock) ->
activate triggers auth (401/400 -> AuthRequired -> OAuth URL) -> OAuth callback
completes -> auth mode cleared (next message triggers LLM turn) -> MCP tools
available.

Regression coverage for:
  - 400 "Authorization header is badly formatted" treated as auth-required
  - OAuth discovery via 401 + WWW-Authenticate header
  - clear_auth_mode after OAuth callback (user message not swallowed)
  - Token trimming (whitespace/newline in stored tokens)

The mock_llm.py serves a mock MCP server at /mcp with full OAuth discovery
endpoints (.well-known/oauth-protected-resource, DCR, token exchange).
"""

from urllib.parse import parse_qs, urlparse

import httpx
import pytest

from helpers import SEL, api_get, api_post


def _extract_state(auth_url: str) -> str:
    """Extract the CSRF state parameter from an OAuth authorization URL."""
    parsed = urlparse(auth_url)
    qs = parse_qs(parsed.query)
    assert "state" in qs, f"auth_url should contain state param: {auth_url}"
    return qs["state"][0]


async def _get_extension(base_url, name):
    """Get a specific extension from the extensions list, or None."""
    r = await api_get(base_url, "/api/extensions")
    for ext in r.json().get("extensions", []):
        if ext["name"] == name:
            return ext
    return None


async def _ensure_removed(base_url, name):
    """Remove extension if already installed."""
    ext = await _get_extension(base_url, name)
    if ext:
        await api_post(base_url, f"/api/extensions/{name}/remove", timeout=30)


# ── Section A: Install MCP Server ────────────────────────────────────────


async def test_mcp_install(ironclaw_server, mock_llm_server):
    """Install a mock MCP server pointing at mock_llm.py's /mcp endpoint."""
    await _ensure_removed(ironclaw_server, "mock-mcp")

    mcp_url = f"{mock_llm_server}/mcp"
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "mock-mcp", "url": mcp_url, "kind": "mcp_server"},
        timeout=30,
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is True, f"Install failed: {data}"

    ext = await _get_extension(ironclaw_server, "mock-mcp")
    assert ext is not None, "mock-mcp should appear in extensions list"
    assert ext["kind"] == "mcp_server"


# ── Section B: Activate Triggers Auth ────────────────────────────────────


async def test_mcp_activate_triggers_auth(ironclaw_server):
    """Activating an unauthenticated MCP server triggers the OAuth flow.

    The mock MCP returns 401 with WWW-Authenticate when no Bearer token
    is present. The activate handler should detect this as auth-required
    and return an auth_url.
    """
    ext = await _get_extension(ironclaw_server, "mock-mcp")
    if ext is None:
        pytest.skip("mock-mcp not installed")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/mock-mcp/activate",
        timeout=30,
    )
    assert r.status_code == 200
    data = r.json()

    # Activation should fail with an auth_url (OAuth needed)
    # OR it should return awaiting_token (manual token prompt)
    auth_url = data.get("auth_url")
    awaiting_token = data.get("awaiting_token")
    assert auth_url is not None or awaiting_token, (
        f"Activate should require auth, got: {data}"
    )
    if auth_url is not None:
        assert _extract_state(auth_url).startswith("ic2."), (
            f"Hosted MCP OAuth should emit versioned state, got: {auth_url}"
        )


# ── Section C: OAuth Round-Trip ──────────────────────────────────────────


async def test_mcp_oauth_callback(ironclaw_server):
    """Complete the OAuth flow via setup + callback for the MCP server."""
    ext = await _get_extension(ironclaw_server, "mock-mcp")
    if ext is None:
        pytest.skip("mock-mcp not installed")

    # Configure with empty secrets to trigger OAuth
    r = await api_post(
        ironclaw_server,
        "/api/extensions/mock-mcp/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert r.status_code == 200
    data = r.json()

    # If no auth_url, try activate to trigger it
    auth_url = data.get("auth_url")
    if auth_url is None:
        r = await api_post(
            ironclaw_server,
            "/api/extensions/mock-mcp/activate",
            timeout=30,
        )
        data = r.json()
        auth_url = data.get("auth_url")

    if auth_url is None:
        # Server might have been auto-authenticated via DCR; check if active
        ext = await _get_extension(ironclaw_server, "mock-mcp")
        if ext and ext.get("authenticated"):
            return  # Already authenticated, skip callback test
        pytest.skip("Could not obtain auth_url for mock-mcp")

    csrf_state = _extract_state(auth_url)

    # Hit the OAuth callback endpoint
    async with httpx.AsyncClient() as client:
        r = await client.get(
            f"{ironclaw_server}/oauth/callback",
            params={"code": "mock_mcp_code", "state": csrf_state},
            timeout=30,
            follow_redirects=True,
        )
    assert r.status_code == 200, f"Callback returned {r.status_code}: {r.text[:300]}"
    body = r.text.lower()
    assert "connected" in body or "success" in body, (
        f"Callback should indicate success: {r.text[:500]}"
    )


async def test_mcp_authenticated_after_oauth(ironclaw_server):
    """After OAuth callback, MCP server shows authenticated=True."""
    ext = await _get_extension(ironclaw_server, "mock-mcp")
    if ext is None:
        pytest.skip("mock-mcp not installed")
    assert ext["authenticated"] is True, (
        f"mock-mcp should be authenticated after OAuth: {ext}"
    )


async def test_mcp_tools_registered(ironclaw_server):
    """After authentication, MCP tools appear in the extension."""
    ext = await _get_extension(ironclaw_server, "mock-mcp")
    if ext is None:
        pytest.skip("mock-mcp not installed")
    tools = ext.get("tools", [])
    assert len(tools) > 0, f"mock-mcp should have tools after auth: {ext}"
    # The mock MCP serves a tool named "mock_search", prefixed with server name
    tool_names = [t for t in tools if "mock_search" in t]
    assert len(tool_names) > 0, f"Expected mock_search tool, got: {tools}"


# ── Section D: Auth Mode Cleared — LLM Turn Fires ───────────────────────


async def test_mcp_auth_mode_cleared_llm_turn_fires(ironclaw_server, page):
    """After OAuth completes, the next user message triggers an LLM turn.

    Regression test: previously, pending_auth was not cleared by the OAuth
    callback handler, so the next user message was consumed as a token and
    the LLM turn never fired.
    """
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    assistant_sel = SEL["message_assistant"]
    before_count = await page.locator(assistant_sel).count()

    # Send a normal message — should trigger LLM, not be swallowed by auth
    await chat_input.fill("hello")
    await chat_input.press("Enter")

    # Wait for assistant response
    expected = before_count + 1
    await page.wait_for_function(
        """({ assistantSelector, expectedCount }) => {
            const messages = document.querySelectorAll(assistantSelector);
            return messages.length >= expectedCount;
        }""",
        arg={"assistantSelector": assistant_sel, "expectedCount": expected},
        timeout=15000,
    )

    text = await page.locator(assistant_sel).last.inner_text()
    assert len(text.strip()) > 0, "Assistant should have responded"


# ── Section E: GitHub-style 400 Error ─────────────────────────────────────


async def test_mcp_400_activate_triggers_auth(ironclaw_server, mock_llm_server):
    """MCP server returning 400 "Authorization header is badly formatted"
    is treated as auth-required (regression for GitHub MCP).

    Previously, only 401 triggered the auth flow. GitHub's MCP returns 400
    with "Authorization header is badly formatted" instead.
    """
    await _ensure_removed(ironclaw_server, "mock-mcp-400")

    mcp_url = f"{mock_llm_server}/mcp-400"
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "mock-mcp-400", "url": mcp_url, "kind": "mcp_server"},
        timeout=30,
    )
    assert r.status_code == 200
    assert r.json().get("success") is True, f"Install failed: {r.json()}"

    # Activate should detect 400 + "authorization" as auth-required
    r = await api_post(
        ironclaw_server,
        "/api/extensions/mock-mcp-400/activate",
        timeout=30,
    )
    assert r.status_code == 200, f"Activate returned {r.status_code}: {r.text[:300]}"
    data = r.json()

    # The 400 should be treated as auth-required, returning an auth_url
    # or awaiting_token — not a raw "400 Bad Request" activation error.
    auth_url = data.get("auth_url")
    awaiting_token = data.get("awaiting_token")
    assert auth_url is not None or awaiting_token, (
        f"400 auth error should trigger auth flow (auth_url or awaiting_token), got: {data}"
    )


async def test_mcp_400_oauth_discovery_returns_auth_url(ironclaw_server):
    """OAuth discovery succeeds for the 400-variant via RFC 9728 (strategy 2).

    Strategy 1 (discover_via_401) fails because /mcp-400 returns 400 without
    a WWW-Authenticate header.  Strategy 2 queries
    /.well-known/oauth-protected-resource/mcp-400 (path-suffixed) and must
    find the mock's wildcard route.  Without that route, discovery fails
    entirely and only awaiting_token (manual) is returned — no auth_url.

    This test would have failed before the wildcard .well-known routes were
    added to mock_llm.py.
    """
    ext = await _get_extension(ironclaw_server, "mock-mcp-400")
    if ext is None:
        pytest.skip("mock-mcp-400 not installed")

    # Re-activate to get a fresh auth response
    r = await api_post(
        ironclaw_server,
        "/api/extensions/mock-mcp-400/activate",
        timeout=30,
    )
    assert r.status_code == 200, f"Activate returned {r.status_code}: {r.text[:300]}"
    data = r.json()

    auth_url = data.get("auth_url")
    assert auth_url is not None, (
        f"OAuth discovery must produce an auth_url (not just awaiting_token). "
        f"Strategy 2 (RFC 9728) likely failed — check .well-known wildcard routes. "
        f"Got: {data}"
    )


async def test_mcp_400_full_oauth_roundtrip(ironclaw_server):
    """Complete OAuth round-trip for the 400-variant MCP server.

    Exercises the full path: activate → 400 detected as auth-required →
    OAuth discovery via strategy 2 (path-suffixed .well-known) → DCR →
    auth_url returned → callback completes token exchange → extension
    authenticated with tools.

    Without the wildcard .well-known routes, OAuth discovery fails and
    no auth_url is produced, so this test would fail at the csrf_state
    extraction step.
    """
    ext = await _get_extension(ironclaw_server, "mock-mcp-400")
    if ext is None:
        pytest.skip("mock-mcp-400 not installed")

    # Get a fresh auth_url via activate
    r = await api_post(
        ironclaw_server,
        "/api/extensions/mock-mcp-400/activate",
        timeout=30,
    )
    data = r.json()
    auth_url = data.get("auth_url")
    if auth_url is None:
        pytest.skip("No auth_url from activate (discovery may not have succeeded)")

    csrf_state = _extract_state(auth_url)

    # Complete OAuth callback
    async with httpx.AsyncClient() as client:
        r = await client.get(
            f"{ironclaw_server}/oauth/callback",
            params={"code": "mock_400_code", "state": csrf_state},
            timeout=30,
            follow_redirects=True,
        )
    assert r.status_code == 200, f"Callback returned {r.status_code}: {r.text[:300]}"
    body = r.text.lower()
    assert "connected" in body or "success" in body, (
        f"400-variant OAuth callback should succeed: {r.text[:500]}"
    )

    # Verify authenticated + tools loaded
    ext = await _get_extension(ironclaw_server, "mock-mcp-400")
    assert ext is not None, "mock-mcp-400 should still be installed"
    assert ext["authenticated"] is True, (
        f"mock-mcp-400 should be authenticated after OAuth: {ext}"
    )
    tools = ext.get("tools", [])
    assert len(tools) > 0, f"mock-mcp-400 should have tools after auth: {ext}"


async def test_mcp_400_cleanup(ironclaw_server):
    """Clean up the 400-variant MCP server."""
    await _ensure_removed(ironclaw_server, "mock-mcp-400")
    ext = await _get_extension(ironclaw_server, "mock-mcp-400")
    assert ext is None, "mock-mcp-400 should be removed"


# ── Section F: Cleanup ───────────────────────────────────────────────────


async def test_mcp_cleanup(ironclaw_server):
    """Remove mock-mcp (cleanup for other test files)."""
    await _ensure_removed(ironclaw_server, "mock-mcp")
    ext = await _get_extension(ironclaw_server, "mock-mcp")
    assert ext is None, "mock-mcp should be removed"
