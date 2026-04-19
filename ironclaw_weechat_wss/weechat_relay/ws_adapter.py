#!/usr/bin/env python3
"""
WeeChat WebSocket Adapter for IronClaw

Maintains a persistent WebSocket connection to WeeChat's relay API and
exposes a local HTTP API (mirroring WeeChat's REST format) that the
IronClaw WASM channel can poll. This gives real-time message delivery
(WebSocket latency) while the WASM still uses simple HTTP requests.

Architecture:
    WASM (poll every 3s) --> HTTP GET /api/buffers/<name>/lines --> this adapter
    this adapter <--> WebSocket <--> WeeChat relay
    WASM (send response) --> HTTP POST /api/input --> WeeChat (direct)

Usage:
    python3 ws_adapter.py [--relay-url URL] [--relay-password PW] [--port PORT]
                         [--message-delay SECONDS]

    Or via environment variables:
    RELAY_URL=http://127.0.0.1:9001 RELAY_PASSWORD=secret python3 ws_adapter.py
    MESSAGE_DELAY_SECONDS=3 python3 ws_adapter.py  # add 3s delay to all messages

Dependencies:
    pip install aiohttp

WeeChat setup:
    /relay add api 9001
    /set relay.network.password "secret"
    /set relay.network.bind_address "127.0.0.1"
"""
import argparse
import asyncio
import base64
import json
import logging
import os
import sys
from collections import defaultdict, deque
from pathlib import Path
from typing import Optional
from urllib.parse import unquote

try:
    import aiohttp
    from aiohttp import web, ClientSession, WSMsgType
except ImportError:
    print("Missing dependency: pip install aiohttp", file=sys.stderr)
    sys.exit(1)

log = logging.getLogger("weechat_ws")

VERSION = "0.1.0"
MAX_LINES_PER_BUFFER = 500  # Max buffered lines per IRC buffer


# ---------------------------------------------------------------------------
# Shared state (lives in the long-running adapter process)
# ---------------------------------------------------------------------------

class AdapterState:
    def __init__(self):
        # Recent lines per buffer: {full_name: deque of LineInfo dicts}
        self.line_buffer: dict[str, deque] = defaultdict(lambda: deque(maxlen=MAX_LINES_PER_BUFFER))
        # Cached buffer list from WeeChat
        self.buffer_list: list = []
        # WebSocket connection status
        self.ws_connected: bool = False
        self.ws_error: Optional[str] = None
        # WeeChat version info (from /api/version)
        self.weechat_version: str = "unknown"
        self.relay_api_version: str = "unknown"
        # Relay config (set at startup)
        self.relay_url: str = ""
        self.relay_password: str = ""
        # Message delay in seconds (0 = disabled)
        self.message_delay_s: float = 0.0

#ii
state = AdapterState()


# ---------------------------------------------------------------------------
# Auth helpers
# ---------------------------------------------------------------------------

def make_auth_header(password: str) -> str:
    token = base64.b64encode(f"plain:{password}".encode()).decode()
    return f"Basic {token}"


def check_auth(request: web.Request) -> bool:
    """Verify the Authorization header matches the configured relay password."""
    if not state.relay_password:
        return True  # No password configured — allow all local requests
    auth = request.headers.get("Authorization", "")
    expected = make_auth_header(state.relay_password)
    return auth == expected


# ---------------------------------------------------------------------------
# WeeChat HTTP helpers (used by adapter internals)
# ---------------------------------------------------------------------------

async def weechat_get(session: ClientSession, path: str, timeout: float = 5.0):
    """GET from WeeChat relay API."""
    url = f"{state.relay_url.rstrip('/')}{path}"
    auth = make_auth_header(state.relay_password)
    try:
        async with session.get(url, headers={"Authorization": auth}, timeout=aiohttp.ClientTimeout(total=timeout)) as resp:
            return resp.status, await resp.read()
    except Exception as e:
        return None, str(e).encode()


async def refresh_buffer_list(session: ClientSession):
    """Fetch current buffer list and version from WeeChat HTTP API."""
    status, body = await weechat_get(session, "/api/buffers")
    if status == 200:
        try:
            data = json.loads(body)
            # WeeChat v2 returns a bare array
            if isinstance(data, list):
                state.buffer_list = data
            elif isinstance(data, dict):
                state.buffer_list = data.get("buffers", [])
            log.debug(f"Refreshed buffer list: {len(state.buffer_list)} buffers")
        except json.JSONDecodeError as e:
            log.warning(f"Failed to parse buffer list: {e}")

    status, body = await weechat_get(session, "/api/version", timeout=3.0)
    if status == 200:
        try:
            data = json.loads(body)
            state.weechat_version = data.get("weechat_version", "unknown")
            state.relay_api_version = data.get("relay_api_version", "unknown")
        except json.JSONDecodeError:
            pass


# ---------------------------------------------------------------------------
# WebSocket client loop
# ---------------------------------------------------------------------------

async def ws_client_loop():
    """
    Connect to WeeChat's WebSocket relay, subscribe to buffer events, and
    process incoming messages. Reconnects automatically on disconnect.
    """
    relay_url = state.relay_url.rstrip("/")
    ws_url = relay_url.replace("http://", "ws://").replace("https://", "wss://") + "/api"
    auth = make_auth_header(state.relay_password)
    attempt = 0

    while True:
        attempt += 1
        try:
            async with ClientSession() as session:
                log.info(f"Connecting to WS at {ws_url} (attempt {attempt})")
                async with session.ws_connect(
                    ws_url,
                    headers={"Authorization": auth},
                    heartbeat=None,
                    max_msg_size=0,  # No size limit
                ) as ws:
                    state.ws_connected = True
                    state.ws_error = None

                    # Seed the buffer list and version info first so we can log the banner
                    await refresh_buffer_list(session)
                    log.info(f"WS connected — relay {state.weechat_version}, API v{state.relay_api_version}")

                    # Subscribe to buffer_line_added events
                    subscribe = {
                        "request_id": "ironclaw-sync",
                        "request": "POST /api/sync",
                        "body": {
                            "sync": {
                                "nicks": False,
                                "input": False,
                                "colors": False,
                                "buffers": True,
                                "lines": True,
                            }
                        },
                    }
                    await ws.send_str(json.dumps(subscribe))
                    log.debug("Sent sync subscription")

                    async for msg in ws:
                        if msg.type == WSMsgType.TEXT:
                            await handle_ws_event(msg.data, session)
                        elif msg.type == WSMsgType.BINARY:
                            log.debug("Ignoring binary WS message")
                        elif msg.type in (WSMsgType.ERROR, WSMsgType.CLOSED):
                            log.info(f"WS disconnected — reconnecting in 5s ({msg.type})")
                            break

                    state.ws_connected = False

        except asyncio.CancelledError:
            log.info("WebSocket loop cancelled")
            return
        except Exception as e:
            state.ws_connected = False
            state.ws_error = str(e)
            log.warning(f"WS disconnected — reconnecting in 5s ({e})")
            await asyncio.sleep(5)


async def handle_ws_event(raw: str, session: ClientSession):
    """Parse and dispatch a single WebSocket event from WeeChat."""
    try:
        msg = json.loads(raw)
    except json.JSONDecodeError:
        log.debug(f"Non-JSON WS frame: {raw[:80]}")
        return

    # WeeChat relay API v0.4.1 uses "event_name", "buffer_id", and "body"
    event = msg.get("event_name") or msg.get("event") or msg.get("id")
    data = msg.get("body") or msg.get("data") or {}
    buffer_id = msg.get("buffer_id")
    log.info(f"WS event: event={event!r} buffer_id={buffer_id!r} body_preview={str(data)[:80]}")

    if event == "buffer_line_added":
        # API v0.4.1: body IS the line object; buffer identified by top-level buffer_id
        line_info = data if isinstance(data, dict) else {}
        # Look up buffer full_name from buffer_id
        full_name = None
        if buffer_id is not None:
            buf = next((b for b in state.buffer_list if b.get("id") == buffer_id), None)
            if buf:
                full_name = buf.get("full_name") or buf.get("name")
        if full_name and line_info:
            tags = line_info.get("tags_array", line_info.get("tags", []))
            nick = next((t[5:] for t in tags if t.startswith("nick_")), "?")
            preview = str(line_info.get("message", ""))[:40]
            log.info(f"buffer_line_added: buffer={full_name} nick={nick} msg={preview!r}")
            if state.message_delay_s > 0:
                await asyncio.sleep(state.message_delay_s)
            state.line_buffer[full_name].append(line_info)
        else:
            # Unknown buffer — refresh list so future lines can be resolved
            log.info(f"buffer_line_added: unknown buffer_id={buffer_id!r} — refreshing buffer list")
            asyncio.create_task(refresh_buffer_list(session))

    elif event == "buffer_opened":
        asyncio.create_task(refresh_buffer_list(session))
        log.info(f"Buffer opened: {data.get('full_name', data.get('name', '?'))} — refreshing buffer list")

    elif event == "buffer_title_changed":
        # Fired when a buffer is created or its title changes; body contains full buffer info
        buf_id = data.get("id")
        buf_name = data.get("full_name") or data.get("name")
        if buf_id and buf_name:
            existing = next((b for b in state.buffer_list if b.get("id") == buf_id), None)
            if existing:
                existing.update(data)
            else:
                state.buffer_list.append(data)
                log.info(f"buffer_title_changed: registered new buffer {buf_name} (id={buf_id})")

    elif event == "buffer_closed":
        full_name = data.get("full_name") or data.get("name")
        if full_name and full_name in state.line_buffer:
            del state.line_buffer[full_name]
        asyncio.create_task(refresh_buffer_list(session))

    elif msg.get("request_id") == "ironclaw-sync":
        # Subscription acknowledgement
        code = msg.get("code", 0)
        log.info(f"Sync subscription acknowledged: code={code} full={msg!r}")

    else:
        log.debug(f"Unhandled WS event: {event!r} keys={list(msg.keys())}")


# ---------------------------------------------------------------------------
# HTTP API handlers (mirror WeeChat REST API format)
# ---------------------------------------------------------------------------

async def handle_version(request: web.Request) -> web.Response:
    return web.json_response({
        "weechat_version": state.weechat_version,
        "relay_api_version": state.relay_api_version,
        "adapter_version": VERSION,
        "ws_connected": state.ws_connected,
    })


async def handle_config(request: web.Request) -> web.Response:
    """GET /api/config — serve local channel config for the WASM to read on startup."""
    config_path = Path(__file__).parent / "weechat_local_config.json"
    try:
        with open(config_path) as f:
            data = json.load(f)
        return web.json_response(data)
    except FileNotFoundError:
        return web.json_response({})
    except Exception as e:
        log.warning(f"Failed to read weechat_local_config.json: {e}")
        return web.json_response({})


async def handle_health(request: web.Request) -> web.Response:
    return web.json_response({
        "status": "ok" if state.ws_connected else "degraded",
        "ws_connected": state.ws_connected,
        "ws_error": state.ws_error,
        "buffered_buffers": len(state.line_buffer),
        "buffer_list_count": len(state.buffer_list),
    })


async def handle_buffers(request: web.Request) -> web.Response:
    if not check_auth(request):
        return web.json_response({"error": "Unauthorized"}, status=401)
    log.debug(f"GET /api/buffers → {len(state.buffer_list)} buffers")
    # Return bare array (WeeChat v2 format)
    return web.json_response(state.buffer_list)


async def handle_single_buffer(request: web.Request) -> web.Response:
    """GET /api/buffers/{name}?lines=-N — return buffer object with embedded lines (WeeChat format)."""
    if not check_auth(request):
        return web.json_response({"error": "Unauthorized"}, status=401)

    raw_name = request.match_info.get("buffer_name", "")
    buffer_name = unquote(raw_name)

    lines_param = request.rel_url.query.get("lines", "")
    limit = abs(int(lines_param)) if lines_param else 50

    # Find buffer info
    buf_info = next((b for b in state.buffer_list if b.get("name") == buffer_name or b.get("full_name") == buffer_name), None)
    result = dict(buf_info) if buf_info else {"name": buffer_name}

    buf = state.line_buffer.get(buffer_name)
    if buf:
        lines = list(buf)[-limit:]
        result["lines"] = list(reversed(lines))
    else:
        result["lines"] = []

    log.debug(f"GET /api/buffers/{buffer_name} → {len(result['lines'])} lines")
    return web.json_response(result)


async def handle_buffer_lines(request: web.Request) -> web.Response:
    if not check_auth(request):
        return web.json_response({"error": "Unauthorized"}, status=401)

    # Buffer name is URL-encoded in the path (# → %23)
    raw_name = request.match_info.get("buffer_name", "")
    buffer_name = unquote(raw_name)

    # Support both ?limit=N (legacy) and ?lines=-N (WeeChat API convention)
    lines_param = request.rel_url.query.get("lines", "")
    if lines_param:
        limit = abs(int(lines_param))
    else:
        limit = int(request.rel_url.query.get("limit", "50"))

    buf = state.line_buffer.get(buffer_name)
    if buf is None:
        log.debug(f"GET /api/buffers/{buffer_name}/lines → 0 lines buffered (unknown buffer)")
        # Unknown buffer — return empty (WASM handles this gracefully)
        return web.json_response([])

    # Return newest-first (WeeChat API default), capped at limit
    lines = list(buf)[-limit:]
    lines_newest_first = list(reversed(lines))
    log.debug(f"GET /api/buffers/{buffer_name}/lines → {len(buf)} lines buffered, returning {len(lines_newest_first)}")
    return web.json_response(lines_newest_first)


async def handle_input(request: web.Request) -> web.Response:
    """
    Proxy POST /api/input to WeeChat's HTTP API.
    The WASM calls relay_url directly for sends, but this endpoint exists
    so the WASM can optionally route everything through the adapter.
    """
    if not check_auth(request):
        return web.json_response({"error": "Unauthorized"}, status=401)

    try:
        body = await request.json()
    except Exception:
        return web.json_response({"error": "Invalid JSON body"}, status=400)

    target_url = f"{state.relay_url.rstrip('/')}/api/input"
    auth = make_auth_header(state.relay_password)

    if state.message_delay_s > 0:
        log.debug(f"Delaying outbound message by {state.message_delay_s}s")
        await asyncio.sleep(state.message_delay_s)

    try:
        async with ClientSession() as session:
            async with session.post(
                target_url,
                headers={"Authorization": auth, "Content-Type": "application/json"},
                json=body,
                timeout=aiohttp.ClientTimeout(total=5),
            ) as resp:
                resp_body = await resp.read()
                log.debug(f"POST /api/input → proxied to WeeChat, status {resp.status}")
                return web.Response(
                    status=resp.status,
                    body=resp_body,
                    content_type="application/json",
                )
    except Exception as e:
        log.warning(f"Failed to proxy /api/input: {e}")
        return web.json_response({"error": str(e)}, status=502)


# ---------------------------------------------------------------------------
# App setup
# ---------------------------------------------------------------------------

def make_app() -> web.Application:
    app = web.Application()
    app.router.add_get("/api/config", handle_config)
    app.router.add_get("/api/version", handle_version)
    app.router.add_get("/api/health", handle_health)
    app.router.add_get("/api/buffers", handle_buffers)
    app.router.add_get(r"/api/buffers/{buffer_name:.+}/lines", handle_buffer_lines)
    app.router.add_get(r"/api/buffers/{buffer_name:.+}", handle_single_buffer)
    app.router.add_post("/api/input", handle_input)
    return app


async def run(relay_url: str, password: str, port: int, message_delay: float = 0.0):
    state.relay_url = relay_url.rstrip("/")
    state.relay_password = password
    state.message_delay_s = message_delay

    app = make_app()

    # Start WebSocket client in background
    ws_task = asyncio.create_task(ws_client_loop())

    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", port)
    await site.start()

    log.info(f"WeeChat WS adapter listening on http://127.0.0.1:{port}")
    log.info(f"Relay: {relay_url}  |  WebSocket: {relay_url.replace('http://', 'ws://').replace('https://', 'wss://')}/api  |  Auth: {'yes' if password else 'no'}")
    if message_delay > 0:
        log.info(f"Message delay enabled: {message_delay}s (applies to both inbound and outbound)")

    try:
        await asyncio.Event().wait()
    finally:
        ws_task.cancel()
        await runner.cleanup()


def main():
    parser = argparse.ArgumentParser(
        description="WeeChat WebSocket Adapter — bridges WeeChat WS events to a local HTTP API"
    )
    parser.add_argument(
        "--relay-url",
        default=os.environ.get("RELAY_URL", "http://127.0.0.1:9001"),
        help="WeeChat relay HTTP URL (default: http://127.0.0.1:9001)",
    )
    parser.add_argument(
        "--relay-password",
        default=os.environ.get("RELAY_PASSWORD", ""),
        help="WeeChat relay password (or set RELAY_PASSWORD env var)",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=int(os.environ.get("ADAPTER_PORT", "6681")),
        help="Local HTTP port for this adapter (default: 6681)",
    )
    parser.add_argument(
        "--message-delay",
        type=float,
        default=float(os.environ.get("MESSAGE_DELAY_SECONDS", "0")),
        metavar="SECONDS",
        help="Delay applied to both inbound and outbound messages (default: 0)",
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        default=os.environ.get("DEBUG", "") != "",
        help="Enable debug logging",
    )
    args = parser.parse_args()

    logging.basicConfig(
        level=logging.DEBUG if args.debug else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    if not args.relay_password:
        log.warning("No relay password set — adapter will accept unauthenticated requests")

    try:
        asyncio.run(run(args.relay_url, args.relay_password, args.port, args.message_delay))
    except KeyboardInterrupt:
        log.info("Adapter stopped")


if __name__ == "__main__":
    main()
