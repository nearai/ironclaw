"""Mock Google Calendar API v3 for workflow-canary scenarios.

Mirrors `telegram_mock.py` / `sheets_mock.py`: single-port aiohttp,
``MOCK_CALENDAR_PORT=<n>`` on stdout, test hooks under ``/__mock/``.
Routes via
``IRONCLAW_TEST_HTTP_REMAP=www.googleapis.com=<mock_url>``
(comma-joined with the other mocks' entries) set in
``run_workflow_canary.py``.

Surface (subset of Calendar v3 actually exercised by IronClaw's
google_calendar tool):

- ``GET /calendar/v3/calendars/{calendarId}/events`` — events.list
  with optional ``timeMin`` / ``timeMax`` / ``maxResults`` filters.
- ``POST /calendar/v3/calendars/{calendarId}/events`` — events.insert
  (used by lifecycle scenarios that round-trip a created event).
- ``GET /calendar/v3/calendars/{calendarId}/events/{eventId}`` —
  events.get for self-cleanup probes.
- ``DELETE /calendar/v3/calendars/{calendarId}/events/{eventId}`` —
  events.delete for self-cleanup probes.

Test-only hooks:

- ``POST /__mock/seed_events`` — pre-create events on a calendar
  (``{"calendar_id": "primary", "events": [...]}``)
- ``GET /__mock/calendars`` — drain all calendars + events
- ``GET /__mock/captured`` — drain captured API calls for assertions
- ``POST /__mock/reset`` — clear all state between probes

Calendar shares the ``www.googleapis.com`` host root with several
other Google services. The Calendar paths begin with ``/calendar/v3/``
so the host-level remap doesn't clash with Sheets (which is at
``sheets.googleapis.com``) or Gmail (``gmail.googleapis.com``).
"""

from __future__ import annotations

import argparse
import asyncio
import time
import uuid
from typing import Any

from aiohttp import web


def _new_state() -> dict[str, Any]:
    return {
        # calendar_id → {events: dict[event_id → event]}
        "calendars": {"primary": {"events": {}}},
        "captured": [],
    }


def _capture(state: dict[str, Any], request: web.Request, body: Any) -> None:
    state["captured"].append(
        {
            "method": request.method,
            "path": request.path,
            "query": dict(request.query),
            "body": body,
            "ts": time.time(),
        }
    )


def _ensure_calendar(state: dict[str, Any], calendar_id: str) -> dict[str, Any]:
    cal = state["calendars"].get(calendar_id)
    if cal is None:
        cal = {"events": {}}
        state["calendars"][calendar_id] = cal
    return cal


# ── Calendar v3 handlers ────────────────────────────────────────────


async def events_list(request: web.Request) -> web.Response:
    state = request.app["state"]
    calendar_id = request.match_info["calendar_id"]
    _capture(state, request, None)

    cal = state["calendars"].get(calendar_id)
    if cal is None:
        return web.json_response({"items": [], "kind": "calendar#events"})

    items = list(cal["events"].values())
    # Apply timeMin / timeMax / maxResults if provided (string compare
    # works for ISO 8601 lexicographically).
    time_min = request.query.get("timeMin")
    time_max = request.query.get("timeMax")
    if time_min:
        items = [e for e in items if (e.get("start", {}).get("dateTime") or "") >= time_min]
    if time_max:
        items = [e for e in items if (e.get("start", {}).get("dateTime") or "") <= time_max]
    try:
        max_results = int(request.query.get("maxResults") or "250")
    except ValueError:
        max_results = 250
    items = items[:max_results]

    return web.json_response(
        {
            "kind": "calendar#events",
            "summary": calendar_id,
            "items": items,
        }
    )


async def events_insert(request: web.Request) -> web.Response:
    state = request.app["state"]
    calendar_id = request.match_info["calendar_id"]
    try:
        body = await request.json()
    except Exception:
        body = {}
    _capture(state, request, body)
    cal = _ensure_calendar(state, calendar_id)
    event_id = body.get("id") or f"canary-event-{uuid.uuid4().hex[:8]}"
    event = {
        "kind": "calendar#event",
        "id": event_id,
        "status": "confirmed",
        "summary": body.get("summary", ""),
        "description": body.get("description", ""),
        "start": body.get("start", {}),
        "end": body.get("end", {}),
        "attendees": body.get("attendees", []),
    }
    cal["events"][event_id] = event
    return web.json_response(event)


async def events_get(request: web.Request) -> web.Response:
    state = request.app["state"]
    calendar_id = request.match_info["calendar_id"]
    event_id = request.match_info["event_id"]
    _capture(state, request, None)
    cal = state["calendars"].get(calendar_id) or {"events": {}}
    event = cal["events"].get(event_id)
    if event is None:
        return web.json_response(
            {"error": {"code": 404, "message": "Not Found"}}, status=404
        )
    return web.json_response(event)


async def events_delete(request: web.Request) -> web.Response:
    state = request.app["state"]
    calendar_id = request.match_info["calendar_id"]
    event_id = request.match_info["event_id"]
    _capture(state, request, None)
    cal = state["calendars"].get(calendar_id) or {"events": {}}
    if event_id in cal["events"]:
        del cal["events"][event_id]
        return web.Response(status=204)
    return web.json_response(
        {"error": {"code": 404, "message": "Not Found"}}, status=404
    )


# ── Test-only hooks ─────────────────────────────────────────────────


async def seed_events(request: web.Request) -> web.Response:
    state = request.app["state"]
    body = await request.json()
    calendar_id = body.get("calendar_id", "primary")
    events = body.get("events") or []
    cal = _ensure_calendar(state, calendar_id)
    seeded = []
    for raw in events:
        event_id = raw.get("id") or f"canary-event-{uuid.uuid4().hex[:8]}"
        event = {
            "kind": "calendar#event",
            "id": event_id,
            "status": raw.get("status", "confirmed"),
            "summary": raw.get("summary", ""),
            "description": raw.get("description", ""),
            "start": raw.get("start", {}),
            "end": raw.get("end", {}),
            "attendees": raw.get("attendees", []),
            "location": raw.get("location", ""),
        }
        cal["events"][event_id] = event
        seeded.append(event_id)
    return web.json_response({"ok": True, "seeded": seeded})


async def list_calendars(request: web.Request) -> web.Response:
    state = request.app["state"]
    return web.json_response(
        {
            "ok": True,
            "calendars": {
                cid: {"events": list(cal["events"].values())}
                for cid, cal in state["calendars"].items()
            },
        }
    )


async def list_captured(request: web.Request) -> web.Response:
    state = request.app["state"]
    return web.json_response({"ok": True, "captured": list(state["captured"])})


async def reset_state(request: web.Request) -> web.Response:
    request.app["state"] = _new_state()
    return web.json_response({"ok": True})


# ── Routing ─────────────────────────────────────────────────────────


@web.middleware
async def _request_logger(request: web.Request, handler):
    print(
        f"[calendar_mock] {request.method} {request.path}"
        f"{('?' + request.query_string) if request.query_string else ''}",
        flush=True,
    )
    return await handler(request)


def make_app() -> web.Application:
    app = web.Application(middlewares=[_request_logger])
    app["state"] = _new_state()

    app.router.add_get(
        "/calendar/v3/calendars/{calendar_id}/events", events_list
    )
    app.router.add_post(
        "/calendar/v3/calendars/{calendar_id}/events", events_insert
    )
    app.router.add_get(
        "/calendar/v3/calendars/{calendar_id}/events/{event_id}", events_get
    )
    app.router.add_delete(
        "/calendar/v3/calendars/{calendar_id}/events/{event_id}", events_delete
    )

    app.router.add_post("/__mock/seed_events", seed_events)
    app.router.add_get("/__mock/calendars", list_calendars)
    app.router.add_get("/__mock/captured", list_captured)
    app.router.add_post("/__mock/reset", reset_state)
    return app


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Mock Google Calendar API v3 for IronClaw workflow-canary."
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
        print(f"MOCK_CALENDAR_PORT={bound}", flush=True)
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
