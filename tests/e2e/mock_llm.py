"""Mock OpenAI-compatible LLM server for E2E tests.

Serves OpenAI-compatible endpoints for chat completions and model listing.
Supports both streaming and non-streaming responses, plus function calling
via TOOL_CALL_PATTERNS.
"""

import argparse
import asyncio
import json
import re
import time
import uuid
from aiohttp import web

CANNED_RESPONSES = [
    (re.compile(r"hello|hi|hey", re.IGNORECASE), "Hello! How can I help you today?"),
    (re.compile(r"2\s*\+\s*2|two plus two", re.IGNORECASE), "The answer is 4."),
    (re.compile(r"skill|install", re.IGNORECASE), "I can help you with skills management."),
    (re.compile(r"html.?test|injection.?test", re.IGNORECASE),
     'Here is some content: <script>alert("xss")</script> and <img src=x onerror="alert(1)">'
     ' and <iframe src="javascript:alert(2)"></iframe> end of content.'),
]
DEFAULT_RESPONSE = "I understand your request."

TOOL_CALL_PATTERNS = [
    (re.compile(r"echo (.+)", re.IGNORECASE), "echo", lambda m: {"message": m.group(1)}),
    (re.compile(r"what time|current time", re.IGNORECASE), "time", lambda _: {"operation": "now"}),
    (
        re.compile(
            r"create lightweight owner routine (?P<name>[a-z0-9][a-z0-9_-]*)",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Owner-scope routine {m.group('name')}",
            "trigger_type": "manual",
            "prompt": f"Confirm that {m.group('name')} executed.",
            "action_type": "lightweight",
            "use_tools": False,
        },
    ),
    (
        re.compile(
            r"create full[- ]job owner routine (?P<name>[a-z0-9][a-z0-9_-]*)",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Owner-scope full-job routine {m.group('name')}",
            "trigger_type": "manual",
            "prompt": f"Complete the routine job for {m.group('name')}.",
            "action_type": "full_job",
        },
    ),
    (
        re.compile(
            r"create event routine (?P<name>[a-z0-9][a-z0-9_-]*) "
            r"channel (?P<channel>[a-z0-9_-]+) pattern (?P<pattern>[a-z0-9_|-]+)",
            re.IGNORECASE,
        ),
        "routine_create",
        lambda m: {
            "name": m.group("name"),
            "description": f"Event routine {m.group('name')}",
            "trigger_type": "event",
            "event_channel": None if m.group("channel").lower() == "any" else m.group("channel"),
            "event_pattern": m.group("pattern"),
            "prompt": f"Acknowledge that {m.group('name')} fired.",
            "action_type": "lightweight",
            "use_tools": False,
            "cooldown_secs": 0,
        },
    ),
    (
        re.compile(r"list owner routines", re.IGNORECASE),
        "routine_list",
        lambda _: {},
    ),
]


def _last_user_content(messages: list[dict]) -> str:
    for msg in reversed(messages):
        if msg.get("role") == "user":
            content = msg.get("content", "")
            if isinstance(content, list):
                content = " ".join(
                    p.get("text", "") for p in content if p.get("type") == "text"
                )
            return content
    return ""


def match_response(messages: list[dict]) -> str:
    content = _last_user_content(messages)
    for pattern, response in CANNED_RESPONSES:
        if pattern.search(content):
            return response
    return DEFAULT_RESPONSE


def match_tool_call(messages: list[dict], has_tools: bool) -> dict | None:
    if not has_tools:
        return None
    content = _last_user_content(messages)
    for pattern, tool_name, args_fn in TOOL_CALL_PATTERNS:
        m = pattern.search(content)
        if m:
            return {"tool_name": tool_name, "arguments": args_fn(m)}
    return None


def _extract_tool_name(msg: dict) -> str:
    """Extract tool name from a message, checking both 'name' field and XML content."""
    name = msg.get("name")
    if name:
        return name
    # ironclaw wraps tool output as <tool_output name="...">
    content = msg.get("content", "")
    m = re.search(r'<tool_output\s+name="([^"]+)"', content)
    if m:
        return m.group(1)
    return "unknown"


def _find_tool_result(messages: list[dict]) -> dict | None:
    """Find a pending tool result that appears after the last user message.

    Only returns a tool result if it's a fresh result the agent is waiting
    for the LLM to summarize (i.e., it follows the most recent user message).
    This prevents stale tool results from earlier conversation turns from
    being re-processed.
    """
    # Find the position of the last user message
    last_user_idx = -1
    for i in range(len(messages) - 1, -1, -1):
        if messages[i].get("role") == "user":
            last_user_idx = i
            break

    # Only look for tool results after the last user message
    for i in range(len(messages) - 1, last_user_idx, -1):
        if messages[i].get("role") == "tool":
            return {"name": _extract_tool_name(messages[i]),
                    "content": messages[i].get("content", "")}
    return None


def _make_base(completion_id: str) -> dict:
    return {"id": completion_id, "object": "chat.completion.chunk",
            "created": int(time.time()), "model": "mock-model"}


async def _send_sse(resp: web.StreamResponse, data: dict):
    await resp.write(f"data: {json.dumps(data)}\n\n".encode())


async def chat_completions(request: web.Request) -> web.StreamResponse:
    """Handle POST /v1/chat/completions and /chat/completions."""
    body = await request.json()
    messages = body.get("messages", [])
    stream = body.get("stream", False)
    has_tools = bool(body.get("tools"))
    cid = f"mock-{uuid.uuid4().hex[:8]}"

    # Tool result in messages -> text summary
    tr = _find_tool_result(messages)
    if tr:
        text = f"The {tr['name']} tool returned: {tr['content']}"
        if not stream:
            return _text_response(cid, text)
        return await _stream_text(request, cid, text)

    # Tool-call pattern match
    tc = match_tool_call(messages, has_tools)
    if tc:
        if not stream:
            return _tool_call_response(cid, tc)
        return await _stream_tool_call(request, cid, tc)

    # Default text response
    text = match_response(messages)
    if not stream:
        return _text_response(cid, text)
    return await _stream_text(request, cid, text)


def _text_response(cid: str, text: str) -> web.Response:
    return web.json_response({
        "id": cid, "object": "chat.completion", "created": int(time.time()),
        "model": "mock-model",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": text},
                      "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": len(text.split()), "total_tokens": 15},
    })


def _tool_call_response(cid: str, tc: dict) -> web.Response:
    return web.json_response({
        "id": cid, "object": "chat.completion", "created": int(time.time()),
        "model": "mock-model",
        "choices": [{"index": 0, "message": {
            "role": "assistant", "content": None,
            "tool_calls": [{"id": f"call_{uuid.uuid4().hex[:8]}", "type": "function",
                            "function": {"name": tc["tool_name"],
                                         "arguments": json.dumps(tc["arguments"])}}],
        }, "finish_reason": "tool_calls"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
    })


async def _stream_text(request: web.Request, cid: str, text: str) -> web.StreamResponse:
    resp = web.StreamResponse(status=200, headers={
        "Content-Type": "text/event-stream", "Cache-Control": "no-cache"})
    await resp.prepare(request)
    base = _make_base(cid)
    chunk = {**base, "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""},
                                   "finish_reason": None}]}
    await _send_sse(resp, chunk)
    for i, word in enumerate(text.split(" ")):
        chunk["choices"][0]["delta"] = {"content": word if i == 0 else f" {word}"}
        await _send_sse(resp, chunk)
    chunk["choices"][0]["delta"] = {}
    chunk["choices"][0]["finish_reason"] = "stop"
    await _send_sse(resp, chunk)
    await resp.write(b"data: [DONE]\n\n")
    return resp


async def _stream_tool_call(request: web.Request, cid: str, tc: dict) -> web.StreamResponse:
    resp = web.StreamResponse(status=200, headers={
        "Content-Type": "text/event-stream", "Cache-Control": "no-cache"})
    await resp.prepare(request)
    call_id = f"call_{uuid.uuid4().hex[:8]}"
    base = _make_base(cid)
    # First chunk: role + tool call header with empty arguments
    chunk = {**base, "choices": [{"index": 0, "delta": {
        "role": "assistant",
        "tool_calls": [{"index": 0, "id": call_id, "type": "function",
                        "function": {"name": tc["tool_name"], "arguments": ""}}],
    }, "finish_reason": None}]}
    await _send_sse(resp, chunk)
    # Second chunk: arguments payload
    chunk["choices"][0]["delta"] = {
        "tool_calls": [{"index": 0, "function": {"arguments": json.dumps(tc["arguments"])}}]}
    await _send_sse(resp, chunk)
    # Final chunk: finish reason
    chunk["choices"][0]["delta"] = {}
    chunk["choices"][0]["finish_reason"] = "tool_calls"
    await _send_sse(resp, chunk)
    await resp.write(b"data: [DONE]\n\n")
    return resp


async def oauth_exchange(request: web.Request) -> web.Response:
    """Mock OAuth token exchange proxy for E2E tests.

    Accepts form params (code, redirect_uri, code_verifier) and returns
    a fake token response. Called by ironclaw's exchange_via_proxy() when
    IRONCLAW_OAUTH_EXCHANGE_URL is set.
    """
    data = await request.post()
    code = data.get("code", "")
    return web.json_response({
        "access_token": f"mock-token-{code}",
        "refresh_token": "mock-refresh-token",
        "expires_in": 3600,
    })


async def models(_request: web.Request) -> web.Response:
    return web.json_response({
        "object": "list",
        "data": [{"id": "mock-model", "object": "model", "owned_by": "test"}],
    })


# ── Mock MCP Server ──────────────────────────────────────────────────────────
#
# Simulates an MCP server that requires OAuth.  Unauthenticated requests get
# 401 + WWW-Authenticate (standard MCP flow) or 400 "Authorization header is
# badly formatted" (GitHub-style).  Authenticated requests return valid
# JSON-RPC responses for initialize and tools/list.


async def mcp_endpoint(request: web.Request) -> web.Response:
    """Handle POST /mcp — JSON-RPC MCP endpoint requiring Bearer auth."""
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer ") or len(auth.split(" ", 1)[1].strip()) == 0:
        # Return 401 with WWW-Authenticate header for OAuth discovery
        resource_meta_url = f"http://127.0.0.1:{request.app['port']}/.well-known/oauth-protected-resource"
        return web.Response(
            status=401,
            headers={"WWW-Authenticate": f'Bearer resource_metadata="{resource_meta_url}"'},
            text="Unauthorized",
        )
    return await _mcp_handle_authed(request)


async def mcp_endpoint_400(request: web.Request) -> web.Response:
    """Handle POST /mcp-400 — MCP endpoint that returns 400 (GitHub-style).

    Simulates GitHub's MCP server which returns 400 "Authorization header
    is badly formatted" instead of 401 when auth is missing or invalid.
    """
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer ") or len(auth.split(" ", 1)[1].strip()) == 0:
        return web.Response(
            status=400,
            text="bad request: Authorization header is badly formatted",
        )
    return await _mcp_handle_authed(request)


async def _mcp_handle_authed(request: web.Request) -> web.Response:
    """Handle an authenticated MCP JSON-RPC request."""
    body = await request.json()
    method = body.get("method", "")
    req_id = body.get("id")

    if method == "initialize":
        return web.json_response({
            "jsonrpc": "2.0", "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mock-mcp", "version": "1.0.0"},
            },
        })
    if method == "notifications/initialized":
        return web.json_response({"jsonrpc": "2.0", "id": req_id, "result": {}})
    if method == "tools/list":
        return web.json_response({
            "jsonrpc": "2.0", "id": req_id,
            "result": {"tools": [{
                "name": "mock_search",
                "description": "A mock search tool for testing",
                "inputSchema": {"type": "object", "properties": {
                    "query": {"type": "string"},
                }},
            }]},
        })
    return web.json_response({"jsonrpc": "2.0", "id": req_id, "error": {
        "code": -32601, "message": f"Method not found: {method}",
    }})


async def mcp_protected_resource(request: web.Request) -> web.Response:
    """GET /.well-known/oauth-protected-resource[/{path}] — RFC 9728 discovery.

    Production code appends the MCP server path after the well-known suffix
    (e.g. /.well-known/oauth-protected-resource/mcp-400), so this handler
    accepts an optional tail and returns a resource matching the request.
    """
    port = request.app["port"]
    tail = request.match_info.get("tail", "mcp")
    return web.json_response({
        "resource": f"http://127.0.0.1:{port}/{tail}",
        "authorization_servers": [f"http://127.0.0.1:{port}"],
    })


async def mcp_auth_server_metadata(request: web.Request) -> web.Response:
    """GET /.well-known/oauth-authorization-server[/{path}] — OAuth metadata."""
    port = request.app["port"]
    base = f"http://127.0.0.1:{port}"
    return web.json_response({
        "issuer": base,
        "authorization_endpoint": f"{base}/oauth/authorize",
        "token_endpoint": f"{base}/oauth/token",
        "registration_endpoint": f"{base}/oauth/register",
        "scopes_supported": ["read", "write"],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
    })


async def mcp_oauth_register(request: web.Request) -> web.Response:
    """POST /oauth/register — Dynamic Client Registration."""
    body = await request.json()
    return web.json_response({
        "client_id": "mock-mcp-client-id",
        "client_name": body.get("client_name", "IronClaw"),
        "redirect_uris": body.get("redirect_uris", []),
    })


async def mcp_oauth_token(request: web.Request) -> web.Response:
    """POST /oauth/token — Token endpoint for MCP OAuth."""
    data = await request.post()
    code = data.get("code", "")
    return web.json_response({
        "access_token": f"mcp-token-{code}",
        "token_type": "Bearer",
        "expires_in": 3600,
    })


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()
    app = web.Application()
    # Register both /v1/ and non-/v1/ paths (rig-core omits the /v1/ prefix)
    app.router.add_post("/v1/chat/completions", chat_completions)
    app.router.add_post("/chat/completions", chat_completions)
    app.router.add_get("/v1/models", models)
    app.router.add_get("/models", models)
    app.router.add_post("/oauth/exchange", oauth_exchange)
    # Mock MCP server endpoints
    app.router.add_post("/mcp", mcp_endpoint)
    app.router.add_post("/mcp-400", mcp_endpoint_400)
    app.router.add_get("/.well-known/oauth-protected-resource", mcp_protected_resource)
    app.router.add_get("/.well-known/oauth-protected-resource/{tail:.*}", mcp_protected_resource)
    app.router.add_get("/.well-known/oauth-authorization-server", mcp_auth_server_metadata)
    app.router.add_get("/.well-known/oauth-authorization-server/{tail:.*}", mcp_auth_server_metadata)
    app.router.add_post("/oauth/register", mcp_oauth_register)
    app.router.add_post("/oauth/token", mcp_oauth_token)

    async def start():
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        app["port"] = port  # used by MCP handlers
        print(f"MOCK_LLM_PORT={port}", flush=True)
        await asyncio.Event().wait()

    asyncio.run(start())


if __name__ == "__main__":
    main()
