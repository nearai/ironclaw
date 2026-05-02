"""Mock Gmail API v1 for workflow-canary scenarios.

Mirrors `telegram_mock.py` / `sheets_mock.py` shape: single-port
aiohttp, ``MOCK_GMAIL_PORT=<n>`` on stdout, test hooks under
``/__mock/``. Routes via
``IRONCLAW_TEST_HTTP_REMAP=gmail.googleapis.com=<mock_url>``.

Surface (subset of Gmail v1 actually exercised by IronClaw's gmail
tool / mock LLM):

- ``GET /gmail/v1/users/{userId}/messages`` — list messages, optional
  ``q=<query>`` filter applied to subject + snippet.
- ``GET /gmail/v1/users/{userId}/messages/{messageId}`` — fetch a
  single message including ``payload.headers`` and ``snippet``.

Test hooks:

- ``POST /__mock/seed_messages`` — replace the message list. Each
  message ``{"id", "subject", "from", "snippet"}`` is rendered into a
  Gmail-shaped payload with ``payload.headers`` populated.
- ``GET /__mock/captured`` — drain captured calls.
- ``POST /__mock/reset`` — clear and re-seed canary defaults.
"""

from __future__ import annotations

import argparse
import asyncio
import time
from typing import Any

from aiohttp import web

DEFAULT_MESSAGES: list[dict[str, str]] = [
    {
        "id": "msg-canary-lead",
        "subject": "Interested in your enterprise tier",
        "from": "Jane Lead <jane.lead@acme.example>",
        "snippet": "Hi — Acme Corp is evaluating vendors for Q2.",
    },
    {
        "id": "msg-canary-newsletter",
        "subject": "Weekly digest",
        "from": "newsletter@example.com",
        "snippet": "This week in tech: nothing actionable.",
    },
    {
        "id": "msg-canary-receipt",
        "subject": "Your receipt from Coffee Shop",
        "from": "no-reply@coffee.example",
        "snippet": "Thank you for your purchase: $4.50.",
    },
]


def _new_state() -> dict[str, Any]:
    return {
        "messages": list(DEFAULT_MESSAGES),
        "captured": [],
    }


def _capture(state: dict[str, Any], request: web.Request) -> None:
    state["captured"].append(
        {
            "method": request.method,
            "path": request.path,
            "query": dict(request.query),
            "ts": time.time(),
        }
    )


def _to_gmail_payload(msg: dict[str, str]) -> dict[str, Any]:
    return {
        "id": msg["id"],
        "threadId": msg.get("threadId", msg["id"]),
        "labelIds": ["INBOX", "UNREAD"],
        "snippet": msg.get("snippet", ""),
        "payload": {
            "mimeType": "text/plain",
            "headers": [
                {"name": "Subject", "value": msg.get("subject", "")},
                {"name": "From", "value": msg.get("from", "")},
                {"name": "Date", "value": "Tue, 28 Apr 2026 03:00:00 +0000"},
            ],
        },
    }


async def messages_list(request: web.Request) -> web.Response:
    state = request.app["state"]
    _capture(state, request)
    q = (request.query.get("q") or "").lower()
    items = state["messages"]
    if q and "is:unread" not in q:
        # Coarse filter for q-based searches (canary uses is:unread or
        # plain text). Real Gmail does much more, but a substring match
        # is enough for canary fixtures.
        items = [
            m for m in items
            if q in (m.get("subject", "") + " " + m.get("snippet", "")).lower()
        ]
    return web.json_response(
        {
            "messages": [
                {"id": m["id"], "threadId": m.get("threadId", m["id"])}
                for m in items
            ],
            "resultSizeEstimate": len(items),
        }
    )


async def messages_get(request: web.Request) -> web.Response:
    state = request.app["state"]
    _capture(state, request)
    message_id = request.match_info["message_id"]
    for m in state["messages"]:
        if m["id"] == message_id:
            return web.json_response(_to_gmail_payload(m))
    return web.json_response(
        {"error": {"code": 404, "message": "Not Found"}}, status=404
    )


async def seed_messages(request: web.Request) -> web.Response:
    state = request.app["state"]
    body = await request.json()
    state["messages"] = list(body.get("messages") or [])
    return web.json_response(
        {"ok": True, "message_count": len(state["messages"])}
    )


async def list_captured(request: web.Request) -> web.Response:
    state = request.app["state"]
    return web.json_response({"ok": True, "captured": list(state["captured"])})


async def reset_state(request: web.Request) -> web.Response:
    request.app["state"] = _new_state()
    return web.json_response({"ok": True})


@web.middleware
async def _request_logger(request: web.Request, handler):
    print(
        f"[gmail_mock] {request.method} {request.path}"
        f"{('?' + request.query_string) if request.query_string else ''}",
        flush=True,
    )
    return await handler(request)


def make_app() -> web.Application:
    app = web.Application(middlewares=[_request_logger])
    app["state"] = _new_state()
    app.router.add_get("/gmail/v1/users/{user_id}/messages", messages_list)
    app.router.add_get(
        "/gmail/v1/users/{user_id}/messages/{message_id}", messages_get
    )
    app.router.add_post("/__mock/seed_messages", seed_messages)
    app.router.add_get("/__mock/captured", list_captured)
    app.router.add_post("/__mock/reset", reset_state)
    return app


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Mock Gmail API v1 for IronClaw workflow-canary."
    )
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()

    app = make_app()
    runner = web.AppRunner(app)

    async def _run() -> None:
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        sockets = site._server.sockets if site._server else None
        bound = sockets[0].getsockname()[1] if sockets else args.port
        print(f"MOCK_GMAIL_PORT={bound}", flush=True)
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
