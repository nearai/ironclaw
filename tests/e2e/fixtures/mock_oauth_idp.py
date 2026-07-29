"""Reusable mock OAuth 2.0 / OIDC authorization server with PKCE support.

Implements the minimum surface needed for Reborn product-auth E2E tests:
  - GET  /authorize  — redirects to callback URL with ?code=&state=
  - POST /token      — issues a fake access_token + refresh_token
  - optional queued OIDC profiles — adds a Google-shaped ``id_token`` so
    WebUI SSO tests can log in distinct users through the real provider path

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
import json
import secrets
import time
from collections import deque
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


@dataclass(frozen=True)
class MockOidcProfile:
    """One identity returned by the next authorization-code exchange."""

    subject: str
    email: str
    display_name: str
    hosted_domain: str | None = None


@dataclass
class MockOAuthIdpHandle:
    base_url: str
    received_codes: list[str] = field(default_factory=list)
    issued_tokens: list[str] = field(default_factory=list)
    # Maps refresh_token → client_id for RFC 6749 §10.4 binding validation.
    issued_refresh_tokens: dict[str, str] = field(default_factory=dict)
    _pending_codes: dict[str, dict] = field(default_factory=dict)
    _initial_oidc_profiles: tuple[MockOidcProfile, ...] = field(default_factory=tuple)
    _oidc_profiles: deque[MockOidcProfile] = field(default_factory=deque)

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
        self._oidc_profiles = deque(self._initial_oidc_profiles)

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


def _google_id_token(profile: MockOidcProfile, client_id: str) -> str:
    """Build a non-secret JWT-shaped ID token for insecure test decoding.

    The production Google provider validates the claims and algorithm after
    receiving the token over the configured token endpoint, but deliberately
    does not verify this response token's signature. The mock still includes a
    non-empty signature segment so the fixture has a valid JWT wire shape.
    """

    def encode_json(value: dict[str, object]) -> str:
        raw = json.dumps(value, separators=(",", ":"), sort_keys=True).encode()
        return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()

    claims: dict[str, object] = {
        "sub": profile.subject,
        "aud": client_id,
        "iss": "https://accounts.google.com",
        "email": profile.email,
        "email_verified": True,
        "name": profile.display_name,
        "exp": int(time.time()) + 3600,
    }
    if profile.hosted_domain:
        claims["hd"] = profile.hosted_domain
    header = encode_json({"alg": "RS256", "typ": "JWT"})
    payload = encode_json(claims)
    signature = base64.urlsafe_b64encode(b"mock-oidc-signature").rstrip(b"=").decode()
    return f"{header}.{payload}.{signature}"


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


async def start_mock_oauth_idp(
    *,
    port: int = 0,
    oidc_profiles: tuple[MockOidcProfile, ...] = (),
) -> AsyncIterator[MockOAuthIdpHandle]:
    """Context manager that starts the mock IDP and yields a handle."""
    handle = MockOAuthIdpHandle(
        base_url="",
        _initial_oidc_profiles=oidc_profiles,
        _oidc_profiles=deque(oidc_profiles),
    )  # base_url filled after bind

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
        client_id = qs.get("client_id", "")

        if not redirect_uri or not state or not client_id:
            return web.Response(
                status=400,
                text="missing client_id, redirect_uri, or state",
            )

        oidc_profile = None
        if handle._initial_oidc_profiles:
            if not handle._oidc_profiles:
                return web.Response(status=409, text="no mock OIDC profiles remain")
            oidc_profile = handle._oidc_profiles.popleft()

        code = f"fake_code_{secrets.token_urlsafe(12)}"
        handle._pending_codes[code] = {
            "client_id": client_id,
            "redirect_uri": redirect_uri,
            "code_challenge": code_challenge,
            "code_challenge_method": code_challenge_method,
            "oidc_profile": oidc_profile,
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
            submitted_client_id = body.get("client_id", "")
            if submitted_client_id and submitted_client_id != pending["client_id"]:
                return web.json_response(
                    {"error": "invalid_grant", "error_description": "client_id mismatch"},
                    status=400,
                )
            # Some product-auth fixtures model a public client and omit
            # client_id at exchange. Retain the authorization request's
            # binding in that case; reject only an explicitly different id.
            client_id = submitted_client_id or pending["client_id"]

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
            handle.issued_tokens.append(access_token)
            handle.issued_refresh_tokens[refresh_token] = client_id
            response = {
                "access_token": access_token,
                "refresh_token": refresh_token,
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": "openid email",
            }
            oidc_profile = pending.get("oidc_profile")
            if oidc_profile is not None:
                response["id_token"] = _google_id_token(oidc_profile, client_id)
            return web.json_response(response)

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
