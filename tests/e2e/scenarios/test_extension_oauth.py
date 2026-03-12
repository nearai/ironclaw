"""Extension OAuth round-trip e2e tests.

Tests the full internal OAuth callback pipeline: install gmail → configure
(get auth_url) → simulate OAuth callback → verify token stored. Uses gateway
callback mode + mock token exchange (no real Google login).

The conftest sets IRONCLAW_OAUTH_CALLBACK_URL (non-loopback, forces gateway
mode) and IRONCLAW_OAUTH_EXCHANGE_URL (points to mock_llm.py's /oauth/exchange).
"""

from urllib.parse import parse_qs, urlparse

import httpx
import pytest

from helpers import api_get, api_post

# Module-level state
_gmail_installed = False
_auth_url = None
_csrf_state = None


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


# ── Section A: Install + OAuth Initiation ────────────────────────────────


async def test_oauth_install_gmail(ironclaw_server):
    """Install gmail from registry for OAuth testing."""
    global _gmail_installed
    await _ensure_removed(ironclaw_server, "gmail")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "gmail"},
        timeout=180,
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is True, f"Install failed: {data.get('message', '')}"
    _gmail_installed = True


async def test_oauth_configure_returns_auth_url(ironclaw_server):
    """Configure with empty secrets returns an OAuth auth_url."""
    global _auth_url, _csrf_state
    if not _gmail_installed:
        pytest.skip("gmail not installed")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is True, f"Configure failed: {data.get('message', '')}"

    _auth_url = data.get("auth_url")
    assert _auth_url is not None, f"Expected auth_url in response: {data}"
    assert "accounts.google.com" in _auth_url, (
        f"auth_url should point to Google: {_auth_url}"
    )

    # Extract CSRF state from the auth URL
    parsed = urlparse(_auth_url)
    qs = parse_qs(parsed.query)
    assert "state" in qs, f"auth_url should contain state param: {_auth_url}"
    _csrf_state = qs["state"][0]
    assert len(_csrf_state) > 0


async def test_oauth_activate_returns_auth_url(ironclaw_server):
    """Activate on un-authenticated gmail returns auth_url."""
    if not _gmail_installed:
        pytest.skip("gmail not installed")

    r = await api_post(
        ironclaw_server, "/api/extensions/gmail/activate", timeout=30
    )
    assert r.status_code == 200
    data = r.json()
    # Activation may fail with auth_url or succeed with auth_url
    auth_url = data.get("auth_url")
    assert auth_url is not None, f"Expected auth_url in activate response: {data}"


# ── Section B: Internal OAuth Round-Trip ─────────────────────────────────


async def test_oauth_callback_exchanges_token(ironclaw_server):
    """Simulate OAuth callback with mock code — verifies token exchange."""
    global _csrf_state
    if not _csrf_state:
        pytest.skip("No CSRF state from configure step")

    # Re-configure to get a fresh pending flow (previous configure may have
    # been consumed by the activate test above)
    r = await api_post(
        ironclaw_server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    data = r.json()
    auth_url = data.get("auth_url")
    if auth_url:
        parsed = urlparse(auth_url)
        qs = parse_qs(parsed.query)
        _csrf_state = qs["state"][0]

    # Hit the OAuth callback endpoint directly (public route, no auth header).
    # The callback handler looks up the pending flow by state, calls
    # exchange_via_proxy() which hits mock_llm.py's /oauth/exchange, and
    # stores the returned fake token.
    async with httpx.AsyncClient() as client:
        r = await client.get(
            f"{ironclaw_server}/oauth/callback",
            params={"code": "mock_auth_code", "state": _csrf_state},
            timeout=30,
            follow_redirects=True,
        )

    assert r.status_code == 200, f"Callback returned {r.status_code}: {r.text[:300]}"
    body = r.text.lower()
    # The landing page says "<name> Connected" on success, "failed" on error
    assert "connected" in body or "success" in body, (
        f"Callback HTML should indicate success: {r.text[:500]}"
    )


async def test_oauth_callback_replay_rejected(ironclaw_server):
    """Replaying the same callback is rejected (flow consumed on first use)."""
    if not _csrf_state:
        pytest.skip("No CSRF state")

    async with httpx.AsyncClient() as client:
        r = await client.get(
            f"{ironclaw_server}/oauth/callback",
            params={"code": "mock_auth_code", "state": _csrf_state},
            timeout=10,
            follow_redirects=True,
        )

    # Should fail — the flow was already consumed
    body = r.text.lower()
    assert "error" in body or "fail" in body or "expired" in body or r.status_code >= 400, (
        f"Replay should be rejected, got status={r.status_code}: {r.text[:500]}"
    )


async def test_oauth_callback_invalid_state(ironclaw_server):
    """Callback with bogus state is rejected."""
    async with httpx.AsyncClient() as client:
        r = await client.get(
            f"{ironclaw_server}/oauth/callback",
            params={"code": "x", "state": "totally-bogus-state-value"},
            timeout=10,
            follow_redirects=True,
        )

    body = r.text.lower()
    assert "error" in body or "fail" in body or "expired" in body or r.status_code >= 400, (
        f"Invalid state should be rejected, got status={r.status_code}: {r.text[:500]}"
    )


async def test_oauth_extension_authenticated(ironclaw_server):
    """After OAuth callback, gmail shows authenticated=True."""
    if not _gmail_installed:
        pytest.skip("gmail not installed")

    ext = await _get_extension(ironclaw_server, "gmail")
    assert ext is not None, "gmail not in extensions list"
    assert ext["authenticated"] is True, (
        f"gmail should be authenticated after OAuth callback: {ext}"
    )


async def test_oauth_tools_registered(ironclaw_server):
    """After OAuth authentication, gmail tools appear in tools endpoint."""
    if not _gmail_installed:
        pytest.skip("gmail not installed")

    ext = await _get_extension(ironclaw_server, "gmail")
    assert ext is not None
    # Check the extension's tools array
    tools = ext.get("tools", [])
    assert len(tools) > 0, (
        f"gmail should have tools registered after auth: {ext}"
    )


# ── Section C: Cleanup ──────────────────────────────────────────────────


async def test_cleanup_gmail(ironclaw_server):
    """Remove gmail (cleanup for other test files)."""
    await _ensure_removed(ironclaw_server, "gmail")
    ext = await _get_extension(ironclaw_server, "gmail")
    assert ext is None, "gmail should be removed"
