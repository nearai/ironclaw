"""Mock Telegram Bot API for workflow-canary scenarios.

Implements the subset of `https://api.telegram.org/bot<token>/<method>`
endpoints that IronClaw's telegram channel + tool actually call:

- `getMe` (token validation)
- `getUpdates` (long-poll for incoming messages)
- `sendMessage` (outbound chat messages — recorded for assertions)
- `sendChatAction` (typing indicator — recorded but mostly no-op)
- `setWebhook` / `deleteWebhook` (webhook lifecycle — recorded)
- `getFile` (file metadata — returns a stub)

Plus test-only hooks under `/__mock/...`:
- `POST /__mock/inject_message` — push a fake user message into the
  next `getUpdates` response so a scenario can simulate a Telegram
  user sending text to the bot
- `GET /__mock/sent_messages` — drain the queue of every
  `sendMessage`/etc. that IronClaw posted, for end-to-end assertions
- `POST /__mock/reset` — clear all state between probes

The server starts on `--port 0` (kernel-assigned) and prints the bound
port on stdout as `MOCK_TELEGRAM_PORT=<n>` so `wait_for_port_line` in
`scripts/live_canary/common.py` can discover it the same way it does
mock_llm.

IronClaw's WASM telegram tool routes API calls through
`IRONCLAW_TEST_HTTP_REMAP=api.telegram.org=<mock_url>`, so this server
just has to look like the Bot API on `/bot<TOKEN>/...` paths.
"""

from __future__ import annotations

import argparse
import asyncio
import time
import uuid
from typing import Any

from aiohttp import web

# Conventions for the mock state shared across requests.
# Stored on `app["state"]` so multiple worker processes wouldn't see
# each other's queues — the canary runs single-process, single-port,
# so this is fine.

DEFAULT_BOT_USERNAME = "ironclaw_canary_bot"
DEFAULT_BOT_FIRST_NAME = "IronClaw Canary"
DEFAULT_BOT_ID = 7700700700  # arbitrary int; Bot API IDs are positive
DEFAULT_USER_ID = 8800800800  # the simulated Telegram user
DEFAULT_USER_FIRST_NAME = "Canary Tester"
DEFAULT_USER_USERNAME = "canary_tester"
DEFAULT_CHAT_ID = DEFAULT_USER_ID  # private chat id == user id


def _new_state() -> dict[str, Any]:
    return {
        # Outbound messages IronClaw sent us. Each entry is a dict with
        # method, chat_id, text, payload (raw request JSON), ts.
        "sent": [],
        # Pending incoming messages to deliver on the next `getUpdates`
        # call. Each entry is a Telegram Update object.
        "pending_updates": [],
        # Monotonically incrementing update_id; Telegram requires this
        # to be strictly increasing so the client can ack with `offset`.
        "next_update_id": 1_000_000_000,
        # Tracks the offset the last getUpdates call requested, so we
        # can drop already-acked updates.
        "last_acked_offset": 0,
        # Webhook URL if setWebhook has been called.
        "webhook_url": None,
        # Token expected on every /bot<token>/ request. Not actually
        # validated against a real Bot API — we accept anything that
        # has the prefix `bot` followed by some characters.
        "accepted_tokens": set(),
    }


def _bot_descriptor() -> dict[str, Any]:
    return {
        "id": DEFAULT_BOT_ID,
        "is_bot": True,
        "first_name": DEFAULT_BOT_FIRST_NAME,
        "username": DEFAULT_BOT_USERNAME,
        "can_join_groups": True,
        "can_read_all_group_messages": False,
        "supports_inline_queries": False,
    }


def _user_descriptor() -> dict[str, Any]:
    return {
        "id": DEFAULT_USER_ID,
        "is_bot": False,
        "first_name": DEFAULT_USER_FIRST_NAME,
        "username": DEFAULT_USER_USERNAME,
        "language_code": "en",
    }


def _chat_descriptor() -> dict[str, Any]:
    return {
        "id": DEFAULT_CHAT_ID,
        "first_name": DEFAULT_USER_FIRST_NAME,
        "username": DEFAULT_USER_USERNAME,
        "type": "private",
    }


def _build_update(state: dict[str, Any], text: str) -> dict[str, Any]:
    update_id = state["next_update_id"]
    state["next_update_id"] = update_id + 1
    return {
        "update_id": update_id,
        "message": {
            "message_id": int(time.time() * 1000) % 1_000_000_000,
            "from": _user_descriptor(),
            "chat": _chat_descriptor(),
            "date": int(time.time()),
            "text": text,
        },
    }


def _ok(result: Any) -> web.Response:
    return web.json_response({"ok": True, "result": result})


def _err(description: str, *, status: int = 400) -> web.Response:
    return web.json_response(
        {"ok": False, "error_code": status, "description": description},
        status=status,
    )


# ── Bot API handlers ────────────────────────────────────────────────


async def get_me(request: web.Request) -> web.Response:
    return _ok(_bot_descriptor())


async def get_updates(request: web.Request) -> web.Response:
    """Long-poll endpoint. Returns immediately with whatever's queued.

    The real Telegram Bot API blocks for up to `timeout` seconds when
    there's no data; we don't bother — IronClaw's polling loop iterates
    fine with empty results, and zero-latency polling makes tests
    deterministic.
    """
    state = request.app["state"]
    try:
        body = await request.json()
    except Exception:
        body = {}
    # IronClaw sometimes passes via query string instead of body.
    offset = int(body.get("offset", request.query.get("offset", "0")) or 0)

    if offset > 0:
        state["last_acked_offset"] = offset
        state["pending_updates"] = [
            u for u in state["pending_updates"] if u["update_id"] >= offset
        ]

    pending = list(state["pending_updates"])
    return _ok(pending)


async def send_message(request: web.Request) -> web.Response:
    state = request.app["state"]
    try:
        payload = await request.json()
    except Exception:
        payload = {}
    chat_id = payload.get("chat_id")
    text = payload.get("text", "")
    state["sent"].append(
        {
            "method": "sendMessage",
            "chat_id": chat_id,
            "text": text,
            "payload": payload,
            "ts": time.time(),
        }
    )
    # Echo back a Message object shaped like the real Bot API.
    return _ok(
        {
            "message_id": int(time.time() * 1000) % 1_000_000_000,
            "from": _bot_descriptor(),
            "chat": _chat_descriptor(),
            "date": int(time.time()),
            "text": text,
        }
    )


async def send_chat_action(request: web.Request) -> web.Response:
    state = request.app["state"]
    try:
        payload = await request.json()
    except Exception:
        payload = {}
    state["sent"].append(
        {
            "method": "sendChatAction",
            "chat_id": payload.get("chat_id"),
            "action": payload.get("action"),
            "payload": payload,
            "ts": time.time(),
        }
    )
    return _ok(True)


async def set_webhook(request: web.Request) -> web.Response:
    state = request.app["state"]
    try:
        payload = await request.json()
    except Exception:
        payload = {}
    state["webhook_url"] = payload.get("url")
    return _ok(True)


async def delete_webhook(request: web.Request) -> web.Response:
    state = request.app["state"]
    state["webhook_url"] = None
    return _ok(True)


async def get_file(request: web.Request) -> web.Response:
    file_id = request.query.get("file_id", "stub")
    return _ok(
        {
            "file_id": file_id,
            "file_unique_id": file_id,
            "file_size": 0,
            "file_path": f"voice/{file_id}.ogg",
        }
    )


# ── Test-only hooks ─────────────────────────────────────────────────


async def inject_message(request: web.Request) -> web.Response:
    """Push a simulated incoming message onto the next getUpdates response."""
    state = request.app["state"]
    try:
        body = await request.json()
    except Exception:
        body = {}
    text = body.get("text", "")
    if not text:
        return _err("text is required", status=422)
    update = _build_update(state, text)
    state["pending_updates"].append(update)
    return web.json_response({"ok": True, "update": update})


async def list_sent(request: web.Request) -> web.Response:
    state = request.app["state"]
    return web.json_response({"ok": True, "messages": list(state["sent"])})


async def reset_state(request: web.Request) -> web.Response:
    request.app["state"] = _new_state()
    return web.json_response({"ok": True})


# ── Routing ─────────────────────────────────────────────────────────


def _bot_route(method: str):
    """Build a handler that routes `/bot<token>/<method>` to `handler`.

    Telegram's API path is `/bot<token>/<method>`; aiohttp routes
    are static, so we capture the token via a path parameter and
    discard it (any token is accepted by the mock).
    """
    handlers = {
        "getMe": get_me,
        "getUpdates": get_updates,
        "sendMessage": send_message,
        "sendChatAction": send_chat_action,
        "setWebhook": set_webhook,
        "deleteWebhook": delete_webhook,
        "getFile": get_file,
    }
    handler = handlers[method]

    async def _route(request: web.Request) -> web.Response:
        # Token is in the path but we don't validate it.
        return await handler(request)

    return _route


@web.middleware
async def _request_logger(request: web.Request, handler):
    """Log every inbound request so the gateway log captures whether
    the gateway's HTTP remap is delivering traffic to us during a
    canary run. Cheap, single-process, no perf impact at the volumes
    the canary generates."""
    print(
        f"[telegram_mock] {request.method} {request.path}"
        f"{('?' + request.query_string) if request.query_string else ''}",
        flush=True,
    )
    return await handler(request)


def make_app() -> web.Application:
    app = web.Application(middlewares=[_request_logger])
    app["state"] = _new_state()

    # Bot API methods (IronClaw uses both POST and GET for getUpdates)
    for method in (
        "getMe",
        "getUpdates",
        "sendMessage",
        "sendChatAction",
        "setWebhook",
        "deleteWebhook",
        "getFile",
    ):
        handler = _bot_route(method)
        app.router.add_post(f"/bot{{token}}/{method}", handler)
        app.router.add_get(f"/bot{{token}}/{method}", handler)

    # Test-only hooks
    app.router.add_post("/__mock/inject_message", inject_message)
    app.router.add_get("/__mock/sent_messages", list_sent)
    app.router.add_post("/__mock/reset", reset_state)

    return app


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Mock Telegram Bot API for IronClaw workflow-canary tests."
    )
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()

    app = make_app()
    runner = web.AppRunner(app)

    async def _run() -> None:
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        # Discover the bound port and announce it.
        sockets = site._server.sockets if site._server else None
        bound = sockets[0].getsockname()[1] if sockets else args.port
        print(f"MOCK_TELEGRAM_PORT={bound}", flush=True)
        # Sleep until interrupted.
        try:
            while True:
                await asyncio.sleep(3600)
        except asyncio.CancelledError:
            pass

    try:
        asyncio.run(_run())
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
