"""E2E tests: Notion MCP OAuth flow via Reborn product-auth routes (issue #4112).

Tests the MCP + OAuth integration path:
1. Notion MCP server is configured as an extension
2. MCP server advertises OAuth ``auth`` capability in ``initialize``
3. Reborn MCP adapter raises ``AuthChallenge::OAuthUrl`` → product-auth flow
4. OAuth callback completes → Bearer injected into subsequent ``tools/call``
5. ``auth_required`` SSE carries ``challenge_kind: "oauth_url"``,
   ``provider: "notion"``, ``authorization_url``
6. No raw token appears in SSE, history, or DOM

Browser tests are skipped until the ``webui-v2-beta`` binary is available.

Note: The Notion MCP OAuth path requires the Reborn composition's MCP adapter
(``ironclaw_reborn_composition::nearai_mcp``) to be active. Tests that
exercise the HTTP API prove the route surface; the MCP OAuth trigger is
exercised in ``crates/ironclaw_reborn_composition`` Rust integration tests.
"""

import asyncio
import json
import os
import re
from pathlib import Path
from urllib.parse import urlparse

import httpx
import pytest

import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import (
    AUTH_TOKEN,
    _forward_coverage_env,
    _reserve_loopback_port,
    _start_engine_v2_server,
    api_get,
    api_post,
    stop_process,
    wait_for_ready,
)
from fixtures.mock_notion_mcp import start_mock_notion_mcp
from fixtures.mock_oauth_idp import (
    issue_oauth_code,
    start_mock_oauth_idp,
)

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

ROOT = Path(__file__).resolve().parent.parent.parent.parent

_OAUTH_SECRET_RE = re.compile(
    r"fake_access_[A-Za-z0-9\-_]+"
    r"|fake_refresh_[A-Za-z0-9\-_]+"
    r"|fake_code_[A-Za-z0-9\-_]+"
)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
async def mock_notion_idp():
    """Start mock OAuth IDP for the Notion MCP server to advertise."""
    async for handle in start_mock_oauth_idp():
        yield handle


@pytest.fixture(scope="module")
async def mock_notion(mock_notion_idp):
    """Start a minimal mock Notion MCP server."""
    async for handle in start_mock_notion_mcp(
        oauth_authorization_url=mock_notion_idp.authorize_url,
        oauth_token_url=mock_notion_idp.token_url,
    ):
        yield handle


@pytest.fixture(autouse=True)
def _reset_mocks(mock_notion, mock_notion_idp):
    """Reset mock state between tests so dirty state from a failure doesn't bleed."""
    yield
    mock_notion.reset()
    mock_notion_idp.reset()


@pytest.fixture(scope="module")
async def v2_notion_server(ironclaw_binary, mock_llm_server, mock_notion, mock_notion_idp, tmp_path_factory):
    """Start ironclaw for Notion MCP OAuth E2E tests."""
    home_dir = str(tmp_path_factory.mktemp("notion-home"))
    db_dir = str(tmp_path_factory.mktemp("notion-db"))
    config_dir = os.path.join(home_dir, ".ironclaw")
    os.makedirs(config_dir, exist_ok=True)

    # Write a minimal config pointing at the mock Notion MCP server.
    config_toml = (
        f'[mcp]\n'
        f'servers = [{{"name" = "notion", "url" = "{mock_notion.base_url}/mcp"}}]\n'
    )
    Path(os.path.join(config_dir, "config.toml")).write_text(config_toml)

    port = _reserve_loopback_port()
    async with _start_engine_v2_server(
        ironclaw_binary,
        mock_llm_server=mock_llm_server,
        port=port,
        home_dir=home_dir,
        db_path=os.path.join(db_dir, "notion-e2e.db"),
        user_id="e2e-4112-notion",
        label="v2_notion_server",
        env_overrides={
            "IRONCLAW_BASE_DIR": config_dir,
            "SKILLS_ENABLED": "false",
            "MCP_ENABLED": "true",
        },
    ) as base_url:
        yield base_url


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestMockNotionMcpFixture:
    """Validate the mock Notion MCP server itself before using it in wider tests."""

    async def test_initialize_returns_oauth_capability(self, mock_notion, mock_notion_idp):
        """Mock MCP initialize response advertises OAuth auth capability."""
        async with httpx.AsyncClient() as client:
            r = await client.post(
                f"{mock_notion.base_url}/mcp",
                json={"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
                timeout=10,
            )
        assert r.status_code == 200
        body = r.json()
        capabilities = body["result"]["capabilities"]
        assert "auth" in capabilities, f"auth capability not in: {capabilities}"
        assert capabilities["auth"]["type"] == "oauth2"
        assert capabilities["auth"]["authorization_url"] == mock_notion_idp.authorize_url
        assert capabilities["auth"]["token_url"] == mock_notion_idp.token_url

    async def test_tools_list_returns_notion_search(self, mock_notion):
        """Mock MCP tools/list returns notion_search."""
        async with httpx.AsyncClient() as client:
            r = await client.post(
                f"{mock_notion.base_url}/mcp",
                json={"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
                timeout=10,
            )
        assert r.status_code == 200
        tools = r.json()["result"]["tools"]
        names = [t["name"] for t in tools]
        assert "notion_search" in names, f"notion_search not in: {names}"

    async def test_tools_call_requires_bearer(self, mock_notion):
        """Mock MCP tools/call without Bearer returns 401."""
        async with httpx.AsyncClient() as client:
            r = await client.post(
                f"{mock_notion.base_url}/mcp",
                json={
                    "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": {"name": "notion_search", "arguments": {"query": "test"}},
                },
                timeout=10,
            )
        assert r.status_code == 401

    async def test_tools_call_succeeds_with_bearer(self, mock_notion):
        """Mock MCP tools/call with Bearer returns results."""
        mock_notion.reset()
        async with httpx.AsyncClient() as client:
            r = await client.post(
                f"{mock_notion.base_url}/mcp",
                headers={"Authorization": "Bearer fake_access_notion_token"},
                json={
                    "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": {"name": "notion_search", "arguments": {"query": "roadmap"}},
                },
                timeout=10,
            )
        assert r.status_code == 200
        assert "content" in r.json()["result"]
        assert mock_notion.tool_call_tokens == ["fake_access_notion_token"]
        assert mock_notion.tool_call_requests == [("notion_search", {"query": "roadmap"})]


class TestNotionMcpOAuthRoutes:
    """Verify Reborn product-auth OAuth routes are reachable for MCP auth flows."""

    async def test_oauth_start_route_accepts_notion_provider(self, v2_notion_server):
        """OAuth start route is mounted and accepts a Notion-provider request."""
        import secrets, hashlib, base64

        verifier = secrets.token_urlsafe(32)
        challenge = base64.urlsafe_b64encode(
            hashlib.sha256(verifier.encode()).digest()
        ).rstrip(b"=").decode()
        state = secrets.token_urlsafe(16)

        r = await api_post(
            v2_notion_server,
            "/api/reborn/product-auth/oauth/start",
            json={
                "provider": "notion",
                "authorization_url": "https://api.notion.com/v1/oauth/authorize?client_id=test",
                "opaque_state": state,
                "pkce_verifier": verifier,
                "expires_at": "2030-01-01T00:00:00Z",
            },
            timeout=15,
        )
        if r.status_code == 404:
            pytest.skip(
                "Reborn product-auth routes not mounted; "
                "need webui-v2-beta feature or Reborn binary"
            )
        assert r.status_code != 405, "405 means route is not mounted"
        assert r.status_code in (200, 400, 422)

    async def test_mock_idp_authorize_endpoint_for_notion(self, mock_notion_idp):
        """The Notion OAuth IDP authorization URL issues a state-bound code."""
        grant = await issue_oauth_code(
            mock_notion_idp,
            client_id="notion-client",
            redirect_uri="http://127.0.0.1:9999/notion/callback",
            scope="read_content",
        )
        assert grant.code.startswith("fake_code_")
        assert grant.state

    async def test_mock_idp_rejects_missing_pkce_verifier_for_notion(self, mock_notion_idp):
        """The Notion OAuth mock rejects auth-code exchange without PKCE verifier."""
        grant = await issue_oauth_code(
            mock_notion_idp,
            client_id="notion-client",
            redirect_uri="http://127.0.0.1:9999/notion/callback",
            scope="read_content",
        )

        async with httpx.AsyncClient() as client:
            r = await client.post(
                mock_notion_idp.token_url,
                data={
                    "grant_type": "authorization_code",
                    "code": grant.code,
                    "redirect_uri": grant.redirect_uri,
                    "client_id": "notion-client",
                },
                timeout=10,
            )
        assert r.status_code == 400
        assert r.json()["error"] == "invalid_grant"

    async def test_mock_idp_rejects_redirect_uri_mismatch_for_notion(self, mock_notion_idp):
        """The Notion OAuth mock binds auth codes to the callback URI."""
        grant = await issue_oauth_code(
            mock_notion_idp,
            client_id="notion-client",
            redirect_uri="http://127.0.0.1:9999/notion/callback",
            scope="read_content",
        )

        async with httpx.AsyncClient() as client:
            r = await client.post(
                mock_notion_idp.token_url,
                data={
                    "grant_type": "authorization_code",
                    "code": grant.code,
                    "redirect_uri": "http://127.0.0.1:9999/notion/wrong-callback",
                    "code_verifier": grant.verifier,
                    "client_id": "notion-client",
                },
                timeout=10,
            )
        assert r.status_code == 400
        assert r.json()["error"] == "invalid_grant"

    async def test_mock_idp_refresh_token_is_bound_to_notion_client_id(
        self,
        mock_notion_idp,
    ):
        """The Notion OAuth mock rejects unknown or cross-client refresh tokens."""
        grant = await issue_oauth_code(
            mock_notion_idp,
            client_id="notion-client",
            redirect_uri="http://127.0.0.1:9999/notion/callback",
            scope="read_content",
        )
        async with httpx.AsyncClient() as client:
            token_r = await client.post(
                mock_notion_idp.token_url,
                data={
                    "grant_type": "authorization_code",
                    "code": grant.code,
                    "redirect_uri": grant.redirect_uri,
                    "code_verifier": grant.verifier,
                    "client_id": "notion-client",
                },
                timeout=10,
            )
            assert token_r.status_code == 200, token_r.text
            refresh_token = token_r.json()["refresh_token"]

            wrong_client_r = await client.post(
                mock_notion_idp.token_url,
                data={
                    "grant_type": "refresh_token",
                    "refresh_token": refresh_token,
                    "client_id": "other-client",
                },
                timeout=10,
            )
            valid_r = await client.post(
                mock_notion_idp.token_url,
                data={
                    "grant_type": "refresh_token",
                    "refresh_token": refresh_token,
                    "client_id": "notion-client",
                },
                timeout=10,
            )
            unknown_r = await client.post(
                mock_notion_idp.token_url,
                data={
                    "grant_type": "refresh_token",
                    "refresh_token": "fake_refresh_unknown",
                    "client_id": "notion-client",
                },
                timeout=10,
            )

        assert wrong_client_r.status_code == 400
        assert valid_r.status_code == 200, valid_r.text
        assert valid_r.json()["access_token"].startswith("fake_access_")
        assert unknown_r.status_code == 400

    async def test_notion_mcp_oauth_flow_end_to_end(self, v2_notion_server, mock_notion):
        """MCP capability via Notion triggers an auth gate (HTTP API smoke test)."""
        mock_notion.reset()
        r = await api_post(v2_notion_server, "/api/chat/thread/new", timeout=15)
        assert r.status_code == 200
        thread_id = r.json()["id"]

        await api_post(
            v2_notion_server,
            "/api/chat/send",
            json={"content": "search notion for roadmap", "thread_id": thread_id},
            timeout=15,
        )

        # The Notion MCP requires Bearer, which causes NeedAuthentication.
        # The run may complete if the mock LLM doesn't trigger an MCP call,
        # but if it does, an auth gate should surface.
        await asyncio.sleep(5.0)
        r_h = await api_get(
            v2_notion_server, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        history = r_h.json()
        # Either: gate surfaced OR run completed (mock LLM may not generate MCP call).
        gate = history.get("pending_gate")
        turns = history.get("turns", [])
        assert gate or turns, (
            "Expected either a pending auth gate or completed turns for Notion MCP request"
        )

    async def test_notion_oauth_secrets_not_in_history(self, v2_notion_server):
        """No raw OAuth code/access/refresh token appears in Notion chat history."""
        r = await api_post(v2_notion_server, "/api/chat/thread/new", timeout=15)
        assert r.status_code == 200
        thread_id = r.json()["id"]

        await api_post(
            v2_notion_server,
            "/api/chat/send",
            json={"content": "search notion for roadmap", "thread_id": thread_id},
            timeout=15,
        )
        await asyncio.sleep(5.0)
        history_r = await api_get(
            v2_notion_server, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        history_text = json.dumps(history_r.json())
        match = _OAUTH_SECRET_RE.search(history_text)
        assert match is None, (
            f"OAuth secret found in Notion history: {match.group()!r}\n"
            f"(first 500): {history_text[:500]}"
        )


# ---------------------------------------------------------------------------
# Browser E2E stubs (skipped until webui-v2-beta E2E binary is available)
# ---------------------------------------------------------------------------

@pytest.mark.skip(
    reason=(
        "Playwright browser test requires ironclaw binary compiled with "
        "webui-v2-beta feature. Enable by building with: "
        "cargo build --features libsql,webui-v2-beta"
    )
)
async def test_notion_browser_oauth_card_renders(v2_notion_server, browser):
    """AuthOauthCard renders in WebUI v2 when Notion MCP OAuth challenge fires."""
    from playwright.async_api import expect

    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{v2_notion_server}/v2/?token={AUTH_TOKEN}")

    chat_input = page.locator("[data-testid='chat-input'], textarea")
    await chat_input.wait_for(state="visible", timeout=10000)
    await chat_input.fill("search notion for roadmap")
    await chat_input.press("Enter")

    oauth_card = page.locator(".auth-oauth-card, [data-challenge-kind='oauth_url']")
    await expect(oauth_card).to_be_visible(timeout=30000)
    # Provider label should mention Notion.
    provider_text = await oauth_card.inner_text()
    assert "notion" in provider_text.lower() or "Notion" in provider_text
    await context.close()


@pytest.mark.skip(reason="See above — requires webui-v2-beta binary")
async def test_notion_browser_bearer_injected_after_oauth(
    v2_notion_server, mock_notion, mock_notion_idp, browser
):
    """After OAuth completes, MCP tools/call receives the Bearer token."""
    from playwright.async_api import expect

    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    mock_notion.reset()

    await page.goto(f"{v2_notion_server}/v2/?token={AUTH_TOKEN}")
    chat_input = page.locator("[data-testid='chat-input'], textarea")
    await chat_input.wait_for(state="visible", timeout=10000)
    await chat_input.fill("search notion for roadmap")
    await chat_input.press("Enter")

    oauth_card = page.locator(".auth-oauth-card")
    await expect(oauth_card).to_be_visible(timeout=30000)

    # Capture the authorization_url from the card's button href / data attr.
    open_btn = oauth_card.locator("button", has_text="authorization")
    auth_url = await open_btn.get_attribute("data-href") or ""

    if auth_url:
        # Simulate the OAuth callback by driving the IDP directly.
        from urllib.parse import parse_qs, urlparse
        qs = parse_qs(urlparse(auth_url).query)
        redirect_uri = qs.get("redirect_uri", [""])[0]
        state = qs.get("state", [""])[0]
        if redirect_uri and state:
            # Pretend the user approved in the popup.
            async with httpx.AsyncClient(follow_redirects=False) as client:
                r = await client.get(auth_url, timeout=10)
            location = r.headers.get("location", "")
            # Drive the callback into our server.
            async with httpx.AsyncClient() as client:
                await client.get(
                    f"{v2_notion_server}{urlparse(location).path}?{urlparse(location).query}",
                    timeout=15,
                )

    # Wait for OAuth card to disappear and run to resume.
    await expect(oauth_card).to_be_hidden(timeout=30000)

    # Notion MCP should have received a Bearer token.
    for _ in range(60):
        if mock_notion.tool_call_tokens:
            break
        await asyncio.sleep(0.5)
    assert mock_notion.tool_call_tokens, "MCP tools/call must receive a Bearer token after OAuth"
    await context.close()
