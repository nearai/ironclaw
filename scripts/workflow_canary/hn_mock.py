"""Mock Hacker News for workflow-canary scenarios.

Single-port aiohttp; announces via ``MOCK_HN_PORT=<n>`` on stdout.
Routes via
``IRONCLAW_TEST_HTTP_REMAP=news.ycombinator.com=<mock_url>`` set in
``run_workflow_canary.py``.

Surface:

- ``GET /newest`` — returns deterministic HTML containing seeded
  posts. Each post is rendered close enough to HN's actual structure
  (``athing`` rows + ``subline`` text) that any scraper which can
  parse real HN can parse this fixture, and includes a
  ``<!-- canary-hn-feed -->`` marker so downstream mocks can
  disambiguate canary fixture content from arbitrary http responses.

Test-only hooks:

- ``POST /__mock/seed_posts`` — replace the post list. Body shape:
  ``{"posts": [{"title": "...", "url": "...", "by": "...",
                 "comments_url": "..."}]}``
- ``GET /__mock/captured`` — drain captured GETs.
- ``POST /__mock/reset`` — clear state and re-seed defaults.
"""

from __future__ import annotations

import argparse
import asyncio
import time
from typing import Any

from aiohttp import web

DEFAULT_POSTS: list[dict[str, str]] = [
    {
        "title": "Show HN: Canary Post Alpha",
        "url": "https://example.com/alpha",
        "by": "canary_alpha",
    },
    {
        "title": "Show HN: Canary Post Beta",
        "url": "https://example.com/beta",
        "by": "canary_beta",
    },
]


def _new_state() -> dict[str, Any]:
    return {
        "posts": list(DEFAULT_POSTS),
        "captured": [],
    }


def _render_html(posts: list[dict[str, str]]) -> str:
    rows: list[str] = []
    for i, p in enumerate(posts, start=1):
        rows.append(
            f"""<tr class='athing' id='canary{i}'>
  <td class='title'><span class='rank'>{i}.</span></td>
  <td class='votelinks'></td>
  <td class='title'><span class='titleline'>
    <a href="{p['url']}">{p['title']}</a>
    <span class='sitebit comhead'> ({p.get('site', 'example.com')})</span>
  </span></td>
</tr>
<tr><td colspan='2'></td><td class='subtext'><span class='subline'>
  <span class='score'>1 point</span>
  by <a href='user?id={p['by']}' class='hnuser'>{p['by']}</a>
  <span class='age'>1 minute ago</span>
  | <a href='item?id=canary{i}'>discuss</a>
</span></td></tr>"""
        )
    return f"""<!DOCTYPE html>
<html lang='en'>
<head><title>Hacker News (canary mock)</title></head>
<body>
<!-- canary-hn-feed -->
<table id='hnmain'>
{''.join(rows)}
</table>
</body>
</html>
"""


async def newest(request: web.Request) -> web.Response:
    state = request.app["state"]
    state["captured"].append(
        {
            "method": request.method,
            "path": request.path,
            "query": dict(request.query),
            "ts": time.time(),
        }
    )
    return web.Response(
        text=_render_html(state["posts"]),
        content_type="text/html",
    )


async def seed_posts(request: web.Request) -> web.Response:
    state = request.app["state"]
    body = await request.json()
    state["posts"] = list(body.get("posts") or [])
    return web.json_response({"ok": True, "post_count": len(state["posts"])})


async def list_captured(request: web.Request) -> web.Response:
    state = request.app["state"]
    return web.json_response({"ok": True, "captured": list(state["captured"])})


async def reset_state(request: web.Request) -> web.Response:
    request.app["state"] = _new_state()
    return web.json_response({"ok": True})


@web.middleware
async def _request_logger(request: web.Request, handler):
    print(
        f"[hn_mock] {request.method} {request.path}"
        f"{('?' + request.query_string) if request.query_string else ''}",
        flush=True,
    )
    return await handler(request)


def make_app() -> web.Application:
    app = web.Application(middlewares=[_request_logger])
    app["state"] = _new_state()
    app.router.add_get("/newest", newest)
    app.router.add_get("/", newest)
    app.router.add_post("/__mock/seed_posts", seed_posts)
    app.router.add_get("/__mock/captured", list_captured)
    app.router.add_post("/__mock/reset", reset_state)
    return app


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Mock Hacker News for IronClaw workflow-canary."
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
        print(f"MOCK_HN_PORT={bound}", flush=True)
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
