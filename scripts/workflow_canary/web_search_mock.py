"""Mock web search backend for workflow-canary scenarios.

Single-port aiohttp; ``MOCK_WEB_SEARCH_PORT=<n>`` on stdout. Routes
via ``IRONCLAW_TEST_HTTP_REMAP=api.search.brave.com=<mock_url>`` (the
canary's chosen web-search backend host — Brave Search v3 shape).

Surface:

- ``GET /res/v1/web/search`` — returns deterministic seeded results
  with title / url / description for each match. Filterable by ``q``
  query parameter (substring match against seeded title + description).

Test hooks:

- ``POST /__mock/seed_results`` — replace the result list.
- ``GET /__mock/captured`` — drain captured calls.
- ``POST /__mock/reset`` — clear and re-seed canary defaults.
"""

from __future__ import annotations

import argparse
import asyncio
import time
from typing import Any

from aiohttp import web

DEFAULT_RESULTS: list[dict[str, str]] = [
    {
        "title": "Acme Corp — Series B fintech",
        "url": "https://acme.example/about",
        "description": (
            "Acme Corp is a fintech in Series B, headquartered in NYC, "
            "raised $40M in 2025 from Sequoia and a16z."
        ),
    },
    {
        "title": "Acme Corp announces enterprise tier",
        "url": "https://news.example/acme-enterprise",
        "description": (
            "The newly-launched enterprise tier targets vendors needing "
            "compliance and SLA guarantees."
        ),
    },
]


def _new_state() -> dict[str, Any]:
    return {
        "results": list(DEFAULT_RESULTS),
        "captured": [],
    }


async def web_search(request: web.Request) -> web.Response:
    state = request.app["state"]
    state["captured"].append(
        {
            "method": request.method,
            "path": request.path,
            "query": dict(request.query),
            "ts": time.time(),
        }
    )
    q = (request.query.get("q") or "").lower()
    items = state["results"]
    if q:
        items = [
            r for r in items
            if q in (r.get("title", "") + " " + r.get("description", "")).lower()
        ] or items  # fallback to all results if filter is too restrictive
    return web.json_response(
        {
            "type": "search",
            "web": {
                "type": "search",
                "results": [
                    {
                        "type": "search_result",
                        "title": r["title"],
                        "url": r["url"],
                        "description": r["description"],
                    }
                    for r in items
                ],
            },
        }
    )


async def seed_results(request: web.Request) -> web.Response:
    state = request.app["state"]
    body = await request.json()
    state["results"] = list(body.get("results") or [])
    return web.json_response({"ok": True, "result_count": len(state["results"])})


async def list_captured(request: web.Request) -> web.Response:
    state = request.app["state"]
    return web.json_response({"ok": True, "captured": list(state["captured"])})


async def reset_state(request: web.Request) -> web.Response:
    request.app["state"] = _new_state()
    return web.json_response({"ok": True})


@web.middleware
async def _request_logger(request: web.Request, handler):
    print(
        f"[web_search_mock] {request.method} {request.path}"
        f"{('?' + request.query_string) if request.query_string else ''}",
        flush=True,
    )
    return await handler(request)


def make_app() -> web.Application:
    app = web.Application(middlewares=[_request_logger])
    app["state"] = _new_state()
    app.router.add_get("/res/v1/web/search", web_search)
    app.router.add_post("/__mock/seed_results", seed_results)
    app.router.add_get("/__mock/captured", list_captured)
    app.router.add_post("/__mock/reset", reset_state)
    return app


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Mock web-search backend for IronClaw workflow-canary."
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
        print(f"MOCK_WEB_SEARCH_PORT={bound}", flush=True)
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
