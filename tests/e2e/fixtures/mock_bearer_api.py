"""Reusable mock Bearer-auth API server.

Extracted from ``test_skill_oauth_flow.py::_start_mock_api`` into a shared
fixture so GitHub PAT, GSuite, and Notion tests can all reuse it.

Endpoints
---------
GET  /repos/{owner}/{repo}/issues   — list issues (Bearer-gated)
POST /repos/{owner}/{repo}/issues   — create issue (Bearer-gated)
GET  /user                          — returns fake user info (Bearer-gated)
GET  /__mock/received-tokens        — returns all tokens seen so far
POST /__mock/reset                  — clears the token list
POST /__mock/set-require-auth       — toggle auth requirement (default: true)

Usage
-----
::
    from fixtures.mock_bearer_api import start_mock_bearer_api

    @pytest.fixture(scope="module")
    async def mock_api():
        async for handle in start_mock_bearer_api():
            yield handle
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import AsyncIterator

from aiohttp import web


@dataclass
class MockBearerApiHandle:
    base_url: str
    # _mock_host is set by test fixtures that need to advertise the host
    # portion of base_url to external components (e.g. skill config writers).
    _mock_host: str | None = field(default=None, repr=False)
    received_tokens: list[str] = field(default_factory=list)
    require_auth: bool = True

    def reset(self) -> None:
        self.received_tokens.clear()
        self.require_auth = True


async def start_mock_bearer_api(*, port: int = 0) -> AsyncIterator[MockBearerApiHandle]:
    handle = MockBearerApiHandle(base_url="")

    def _check_auth(request: web.Request) -> str | None:
        """Return token string if auth OK, None if missing/wrong and auth required."""
        if not handle.require_auth:
            return "bypass"
        auth = request.headers.get("Authorization", "")
        if not auth.startswith("Bearer "):
            return None
        return auth[len("Bearer "):]

    async def issues_get(request: web.Request) -> web.Response:
        token = _check_auth(request)
        if token is None:
            return web.json_response({"message": "Bad credentials"}, status=401)
        handle.received_tokens.append(token)
        return web.json_response([
            {"number": 1, "title": "First issue", "state": "open"},
        ])

    async def issues_post(request: web.Request) -> web.Response:
        token = _check_auth(request)
        if token is None:
            return web.json_response({"message": "Bad credentials"}, status=401)
        handle.received_tokens.append(token)
        body = await request.json()
        return web.json_response({
            "number": 42,
            "title": body.get("title", ""),
            "html_url": "https://github.com/test/repo/issues/42",
            "state": "open",
        }, status=201)

    async def user_get(request: web.Request) -> web.Response:
        token = _check_auth(request)
        if token is None:
            return web.json_response({"message": "Bad credentials"}, status=401)
        handle.received_tokens.append(token)
        return web.json_response({"login": "fake-user", "name": "Fake User"})

    async def received_tokens(request: web.Request) -> web.Response:
        return web.json_response({"tokens": handle.received_tokens})

    async def reset(request: web.Request) -> web.Response:
        handle.reset()
        return web.json_response({"ok": True})

    async def set_require_auth(request: web.Request) -> web.Response:
        body = await request.json()
        handle.require_auth = bool(body.get("require_auth", True))
        return web.json_response({"require_auth": handle.require_auth})

    app = web.Application()
    app.router.add_get("/repos/{owner}/{repo}/issues", issues_get)
    app.router.add_post("/repos/{owner}/{repo}/issues", issues_post)
    app.router.add_get("/user", user_get)
    app.router.add_get("/__mock/received-tokens", received_tokens)
    app.router.add_post("/__mock/reset", reset)
    app.router.add_post("/__mock/set-require-auth", set_require_auth)

    runner = web.AppRunner(app)
    await runner.setup()
    try:
        site = web.TCPSite(runner, "127.0.0.1", port)
        await site.start()
        actual_port = site._server.sockets[0].getsockname()[1]
        from urllib.parse import urlparse
        handle.base_url = f"http://127.0.0.1:{actual_port}"
        handle._mock_host = urlparse(handle.base_url).netloc
        yield handle
    finally:
        await runner.cleanup()
