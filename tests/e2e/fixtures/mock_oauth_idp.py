"""Reusable mock OAuth 2.0 authorization server with PKCE support.

Implements the minimum surface needed for Reborn product-auth E2E tests:
  - GET  /authorize  — redirects to callback URL with ?code=&state=
  - POST /token      — issues a fake access_token + refresh_token

Security assertions this fixture supports:
  - PKCE S256 challenge round-trip (can be toggled off for negative tests)
  - State parameter round-trip
  - Fake token values never match real credentials (prefixed ``fake_``)

Usage
-----
The fixture is module-scoped. Import and use in your test file::

    from fixtures.mock_oauth_idp import start_mock_oauth_idp

    @pytest.fixture(scope="module")
    async def mock_idp():
        async for handle in start_mock_oauth_idp():
            yield handle

The yielded ``MockOAuthIdpHandle`` exposes:
  - ``base_url: str``           — e.g. ``http://127.0.0.1:PORT``
  - ``authorize_url: str``      — ``{base_url}/authorize``
  - ``token_url: str``          — ``{base_url}/token``
  - ``received_codes: list``    — authorization codes that passed /token
  - ``issued_tokens: list``     — access tokens issued
  - ``reset()``                 — clears state
"""

from __future__ import annotations

import hashlib
import base64
import secrets
from dataclasses import dataclass, field
from typing import AsyncIterator
from urllib.parse import parse_qs, urlparse

import httpx
from aiohttp import web


def pkce_challenge_for(verifier: str) -> str:
    """Return the S256 PKCE challenge for *verifier*."""
    digest = hashlib.sha256(verifier.encode()).digest()
    return base64.urlsafe_b64encode(digest).rstrip(b"=").decode()


def make_pkce_verifier_and_challenge() -> tuple[str, str]:
    """Return a random PKCE verifier and its S256 challenge."""
    verifier = secrets.token_urlsafe(32)
    return verifier, pkce_challenge_for(verifier)


@dataclass(frozen=True)
class MockOAuthCodeGrant:
    code: str
    redirect_uri: str
    verifier: str
    state: str


@dataclass
class MockOAuthIdpHandle:
    base_url: str
    received_codes: list[str] = field(default_factory=list)
    issued_tokens: list[str] = field(default_factory=list)
    # Maps refresh_token → client_id for RFC 6749 §10.4 binding validation.
    issued_refresh_tokens: dict[str, str] = field(default_factory=dict)
    _pending_codes: dict[str, dict] = field(default_factory=dict)

    @property
    def authorize_url(self) -> str:
        return f"{self.base_url}/authorize"

    @property
    def token_url(self) -> str:
        return f"{self.base_url}/token"

    def reset(self) -> None:
        self.received_codes.clear()
        self.issued_tokens.clear()
        self.issued_refresh_tokens.clear()
        self._pending_codes.clear()

    def make_authorization_url(
        self,
        *,
        client_id: str = "test-client",
        redirect_uri: str,
        state: str,
        code_challenge: str | None = None,
        scope: str = "openid email",
    ) -> str:
        """Build an authorization URL pointing at this mock IDP."""
        from urllib.parse import urlencode

        params = {
            "response_type": "code",
            "client_id": client_id,
            "redirect_uri": redirect_uri,
            "state": state,
            "scope": scope,
        }
        if code_challenge:
            params["code_challenge"] = code_challenge
            params["code_challenge_method"] = "S256"
        return f"{self.authorize_url}?{urlencode(params)}"


async def issue_oauth_code(
    handle: MockOAuthIdpHandle,
    *,
    client_id: str,
    redirect_uri: str,
    scope: str = "openid email",
) -> MockOAuthCodeGrant:
    """Issue a PKCE-bound authorization code from the mock IDP."""
    verifier, challenge = make_pkce_verifier_and_challenge()
    state = secrets.token_urlsafe(16)
    auth_url = handle.make_authorization_url(
        client_id=client_id,
        redirect_uri=redirect_uri,
        state=state,
        code_challenge=challenge,
        scope=scope,
    )
    async with httpx.AsyncClient(follow_redirects=False) as client:
        response = await client.get(auth_url, timeout=10)
    assert response.status_code in (302, 307), (
        f"expected redirect, got {response.status_code}"
    )
    location = response.headers.get("location", "")
    params = parse_qs(urlparse(location).query)
    assert params.get("state", [""])[0] == state, "state must round-trip"
    code = params["code"][0]
    assert code.startswith("fake_code_")
    return MockOAuthCodeGrant(
        code=code,
        redirect_uri=redirect_uri,
        verifier=verifier,
        state=state,
    )


async def start_mock_oauth_idp(*, port: int = 0) -> AsyncIterator[MockOAuthIdpHandle]:
    """Context manager that starts the mock IDP and yields a handle."""
    handle = MockOAuthIdpHandle(base_url="")  # filled after bind

    async def authorize(request: web.Request) -> web.Response:
        """Simulate the IdP authorization endpoint.

        In real flows the user sees a consent screen; here we auto-approve and
        redirect immediately so tests don't need browser interaction.
        """
        qs = request.rel_url.query
        redirect_uri = qs.get("redirect_uri", "")
        state = qs.get("state", "")
        code_challenge = qs.get("code_challenge")
        code_challenge_method = qs.get("code_challenge_method", "S256")

        if not redirect_uri or not state:
            return web.Response(status=400, text="missing redirect_uri or state")

        code = f"fake_code_{secrets.token_urlsafe(12)}"
        handle._pending_codes[code] = {
            "redirect_uri": redirect_uri,
            "code_challenge": code_challenge,
            "code_challenge_method": code_challenge_method,
        }

        from urllib.parse import urlencode
        params = urlencode({"code": code, "state": state})
        raise web.HTTPFound(location=f"{redirect_uri}?{params}")

    async def token(request: web.Request) -> web.Response:
        """Simulate the IdP token endpoint."""
        body = await request.post()
        grant_type = body.get("grant_type", "")
        code = body.get("code", "")
        redirect_uri = body.get("redirect_uri", "")
        code_verifier = body.get("code_verifier")

        if grant_type == "authorization_code":
            pending = handle._pending_codes.pop(code, None)
            if pending is None:
                return web.json_response({"error": "invalid_grant"}, status=400)

            # RFC 6749 §4.1.3 — redirect_uri must match the one from /authorize.
            if pending["redirect_uri"] and redirect_uri != pending["redirect_uri"]:
                return web.json_response(
                    {"error": "invalid_grant", "error_description": "redirect_uri mismatch"},
                    status=400,
                )

            # PKCE S256: verifier required when challenge was registered.
            expected_challenge = pending.get("code_challenge")
            if expected_challenge:
                if not code_verifier:
                    return web.json_response(
                        {"error": "invalid_grant", "error_description": "PKCE verifier missing"},
                        status=400,
                    )
                computed = pkce_challenge_for(code_verifier)
                if computed != expected_challenge:
                    return web.json_response(
                        {"error": "invalid_grant", "error_description": "PKCE mismatch"},
                        status=400,
                    )

            handle.received_codes.append(code)
            access_token = f"fake_access_{secrets.token_urlsafe(16)}"
            refresh_token = f"fake_refresh_{secrets.token_urlsafe(16)}"
            client_id = body.get("client_id", "")
            handle.issued_tokens.append(access_token)
            handle.issued_refresh_tokens[refresh_token] = client_id
            return web.json_response({
                "access_token": access_token,
                "refresh_token": refresh_token,
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": "openid email",
            })

        if grant_type == "refresh_token":
            refresh_token = body.get("refresh_token", "")
            stored_client = handle.issued_refresh_tokens.get(refresh_token)
            if stored_client is None or stored_client != body.get("client_id", ""):
                return web.json_response({"error": "invalid_grant"}, status=400)
            new_access = f"fake_access_{secrets.token_urlsafe(16)}"
            handle.issued_tokens.append(new_access)
            return web.json_response({
                "access_token": new_access,
                "token_type": "Bearer",
                "expires_in": 3600,
            })

        return web.json_response({"error": "unsupported_grant_type"}, status=400)

    async def state_view(request: web.Request) -> web.Response:
        return web.json_response({
            "received_codes": handle.received_codes,
            "issued_tokens": handle.issued_tokens,
        })

    async def reset_view(request: web.Request) -> web.Response:
        handle.reset()
        return web.json_response({"ok": True})

    app = web.Application()
    app.router.add_get("/authorize", authorize)
    app.router.add_post("/token", token)
    app.router.add_get("/__mock/state", state_view)
    app.router.add_post("/__mock/reset", reset_view)

    runner = web.AppRunner(app)
    await runner.setup()
    try:
        site = web.TCPSite(runner, "127.0.0.1", port)
        await site.start()
        actual_port = site._server.sockets[0].getsockname()[1]
        handle.base_url = f"http://127.0.0.1:{actual_port}"
        yield handle
    finally:
        await runner.cleanup()
