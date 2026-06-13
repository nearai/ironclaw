"""Minimal mock Notion MCP server for product-auth E2E tests.

Implements the MCP protocol surface needed to exercise the Reborn host's
Notion MCP OAuth flow:

  - POST /mcp  (JSON-RPC 2.0, ``initialize`` / ``tools/list`` / ``tools/call``)
  - Bearer-gated ``tools/call`` — returns 401 JSON-RPC error without auth
  - OAuth auth_required metadata injected in ``initialize`` response so the
    Reborn MCP adapter knows to trigger a product-auth OAuth flow. Tests can
    inject a mock IDP's authorization/token URLs when starting the server.

Recorded calls are exposed for assertion::

    handle.tool_call_tokens   — list of Bearer tokens seen on tools/call
    handle.tool_call_requests — list of (tool_name, params) tuples

Usage
-----
::
    from fixtures.mock_notion_mcp import start_mock_notion_mcp

    @pytest.fixture(scope="module")
    async def mock_notion():
        async for handle in start_mock_notion_mcp():
            yield handle
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import AsyncIterator

from aiohttp import web


@dataclass
class MockNotionMcpHandle:
    base_url: str
    oauth_authorization_url: str | None = None
    oauth_token_url: str | None = None
    tool_call_tokens: list[str] = field(default_factory=list)
    tool_call_requests: list[tuple[str, dict]] = field(default_factory=list)

    def reset(self) -> None:
        self.tool_call_tokens.clear()
        self.tool_call_requests.clear()


def _json_rpc_error(request_id, code: int, message: str) -> dict:
    return {"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}}


def _json_rpc_ok(request_id, result) -> dict:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


async def start_mock_notion_mcp(
    *,
    port: int = 0,
    oauth_authorization_url: str | None = None,
    oauth_token_url: str | None = None,
) -> AsyncIterator[MockNotionMcpHandle]:
    handle = MockNotionMcpHandle(
        base_url="",
        oauth_authorization_url=oauth_authorization_url,
        oauth_token_url=oauth_token_url,
    )

    async def mcp_handler(request: web.Request) -> web.Response:
        body = await request.json()
        method = body.get("method", "")
        req_id = body.get("id")

        if method == "initialize":
            # Advertise OAuth auth_required so the Reborn MCP adapter
            # raises an AuthChallenge::OAuthUrl and creates a product-auth flow.
            return web.json_response(_json_rpc_ok(req_id, {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "mock-notion-mcp", "version": "0.1.0"},
                "capabilities": {
                    "tools": {},
                    "auth": {
                        "type": "oauth2",
                        "authorization_url": (
                            handle.oauth_authorization_url
                            or f"{handle.base_url}/authorize"
                        ),
                        "token_url": handle.oauth_token_url or f"{handle.base_url}/token",
                        "scopes": ["read_content"],
                    },
                },
            }))

        if method == "tools/list":
            return web.json_response(_json_rpc_ok(req_id, {
                "tools": [
                    {
                        "name": "notion_search",
                        "description": "Search Notion pages",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"query": {"type": "string"}},
                            "required": ["query"],
                        },
                    }
                ]
            }))

        if method == "tools/call":
            # Check Bearer auth
            auth = request.headers.get("Authorization", "")
            if not auth.startswith("Bearer "):
                return web.json_response(
                    _json_rpc_error(req_id, -32001, "Unauthorized: missing Bearer token"),
                    status=401,
                )
            token = auth[len("Bearer "):]
            handle.tool_call_tokens.append(token)

            params = body.get("params", {})
            tool_name = params.get("name", "")
            tool_args = params.get("arguments", {})
            handle.tool_call_requests.append((tool_name, tool_args))

            return web.json_response(_json_rpc_ok(req_id, {
                "content": [
                    {"type": "text", "text": json.dumps([
                        {"id": "page-1", "title": "Meeting notes", "url": "https://notion.so/p1"},
                        {"id": "page-2", "title": "Project roadmap", "url": "https://notion.so/p2"},
                    ])}
                ]
            }))

        return web.json_response(
            _json_rpc_error(req_id, -32601, f"Method not found: {method}"),
            status=404,
        )

    async def state_view(request: web.Request) -> web.Response:
        return web.json_response({
            "tool_call_tokens": handle.tool_call_tokens,
            "tool_call_requests": [
                {"name": name, "args": args}
                for name, args in handle.tool_call_requests
            ],
        })

    async def reset_view(request: web.Request) -> web.Response:
        handle.reset()
        return web.json_response({"ok": True})

    app = web.Application()
    app.router.add_post("/mcp", mcp_handler)
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
