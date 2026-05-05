"""Mock Google Sheets API v4 for workflow-canary scenarios.

Mirrors `scripts/workflow_canary/telegram_mock.py`'s shape: single-port
aiohttp, ``MOCK_SHEETS_PORT=<n>`` on stdout, test hooks under
``/__mock/``. Routes via
``IRONCLAW_TEST_HTTP_REMAP=sheets.googleapis.com=<mock_url>`` set in
``run_workflow_canary.py``.

Surface (subset of Sheets v4 actually exercised by IronClaw's tools):

- ``POST /v4/spreadsheets`` — create a new spreadsheet
- ``POST /v4/spreadsheets/{id}/values/{range}:append`` — append rows
  (the canonical "expected a sequence" failure surface from
  Scripts 1 + 5 FAIL CRITERIA)
- ``GET /v4/spreadsheets/{id}/values/{range}`` — read rows back

Test-only hooks:

- ``POST /__mock/seed_spreadsheet`` — pre-create a sheet with headers
- ``GET /__mock/spreadsheets`` — drain all created spreadsheets
- ``GET /__mock/captured`` — drain every API call we received (method,
  path, body) for assertion-side inspection
- ``POST /__mock/reset`` — clear all state between probes

The mock validates ``values`` is a list-of-lists (refusing strings),
which is exactly the contract IronClaw's google-sheets WASM tool
expects to satisfy. A regression where the tool sends a string instead
of an array would surface as a 400 from this mock — same shape as the
real ``"expected a sequence"`` error from real Google.
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
        "spreadsheets": {},  # id → {properties, sheets, values_by_range}
        "captured": [],       # every inbound API call for assertions
    }


def _new_spreadsheet(
    spreadsheet_id: str | None = None,
    title: str = "Untitled spreadsheet",
    headers: list[str] | None = None,
) -> dict[str, Any]:
    sid = spreadsheet_id or str(uuid.uuid4())
    sheet_id = 0  # default sheet id
    sheet_title = "Sheet1"
    values: list[list[str]] = []
    if headers:
        values.append(list(headers))
    return {
        "spreadsheetId": sid,
        "properties": {"title": title},
        "sheets": [
            {
                "properties": {
                    "sheetId": sheet_id,
                    "title": sheet_title,
                    "index": 0,
                    "sheetType": "GRID",
                }
            }
        ],
        "spreadsheetUrl": f"http://canary.local/spreadsheets/{sid}",
        "_rows": values,  # internal storage; not in Google's response shape
    }


def _drop_internal_fields(spreadsheet: dict[str, Any]) -> dict[str, Any]:
    """Strip internal-only fields (prefixed with `_`) before returning to
    callers — keeps the response shape Google-compatible."""
    return {k: v for k, v in spreadsheet.items() if not k.startswith("_")}


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


# ── Bot API equivalent handlers ─────────────────────────────────────


async def create_spreadsheet(request: web.Request) -> web.Response:
    state = request.app["state"]
    try:
        body = await request.json()
    except Exception:
        body = {}
    _capture(state, request, body)
    title = (body.get("properties") or {}).get("title", "Untitled spreadsheet")
    spreadsheet = _new_spreadsheet(title=title)
    state["spreadsheets"][spreadsheet["spreadsheetId"]] = spreadsheet
    return web.json_response(_drop_internal_fields(spreadsheet))


async def values_append(request: web.Request) -> web.Response:
    """Mirrors `spreadsheets.values.append`. The "expected a sequence"
    error from real Google fires when `values` is not a list-of-lists;
    we enforce the same contract here so the canary catches the same
    regression shape."""
    state = request.app["state"]
    spreadsheet_id = request.match_info["spreadsheet_id"]
    range_a1 = request.match_info["range"]
    try:
        body = await request.json()
    except Exception:
        body = {}
    _capture(state, request, body)

    values = body.get("values")
    if not isinstance(values, list) or not all(
        isinstance(row, list) for row in values
    ):
        return web.json_response(
            {
                "error": {
                    "code": 400,
                    "message": (
                        "Invalid value at 'data.values' — expected a "
                        "sequence of sequences"
                    ),
                    "status": "INVALID_ARGUMENT",
                }
            },
            status=400,
        )

    spreadsheet = state["spreadsheets"].get(spreadsheet_id)
    if spreadsheet is None:
        # Auto-create on append — Sheets does NOT do this in production,
        # but the canary's deterministic seeding makes lazy creation
        # convenient. Production-shape behavior would be to 404; flip
        # this to strict via env var if we ever need to test that path.
        spreadsheet = _new_spreadsheet(spreadsheet_id=spreadsheet_id)
        state["spreadsheets"][spreadsheet_id] = spreadsheet

    rows = spreadsheet["_rows"]
    rows.extend(values)
    updated_rows_count = len(values)
    updated_columns_count = max((len(r) for r in values), default=0)

    return web.json_response(
        {
            "spreadsheetId": spreadsheet_id,
            "tableRange": range_a1,
            "updates": {
                "spreadsheetId": spreadsheet_id,
                "updatedRange": range_a1,
                "updatedRows": updated_rows_count,
                "updatedColumns": updated_columns_count,
                "updatedCells": updated_rows_count * updated_columns_count,
            },
        }
    )


async def values_get(request: web.Request) -> web.Response:
    state = request.app["state"]
    spreadsheet_id = request.match_info["spreadsheet_id"]
    range_a1 = request.match_info["range"]
    _capture(state, request, None)

    spreadsheet = state["spreadsheets"].get(spreadsheet_id)
    if spreadsheet is None:
        return web.json_response(
            {
                "error": {
                    "code": 404,
                    "message": f"Requested entity was not found: {spreadsheet_id}",
                    "status": "NOT_FOUND",
                }
            },
            status=404,
        )

    return web.json_response(
        {
            "range": range_a1,
            "majorDimension": "ROWS",
            "values": list(spreadsheet["_rows"]),
        }
    )


async def get_spreadsheet(request: web.Request) -> web.Response:
    state = request.app["state"]
    spreadsheet_id = request.match_info["spreadsheet_id"]
    _capture(state, request, None)
    spreadsheet = state["spreadsheets"].get(spreadsheet_id)
    if spreadsheet is None:
        return web.json_response(
            {
                "error": {
                    "code": 404,
                    "message": f"Requested entity was not found: {spreadsheet_id}",
                    "status": "NOT_FOUND",
                }
            },
            status=404,
        )
    return web.json_response(_drop_internal_fields(spreadsheet))


# ── Test-only hooks ─────────────────────────────────────────────────


async def seed_spreadsheet(request: web.Request) -> web.Response:
    state = request.app["state"]
    body = await request.json()
    spreadsheet_id = body.get("spreadsheet_id") or str(uuid.uuid4())
    headers = body.get("headers") or []
    title = body.get("title", "Canary seeded sheet")
    spreadsheet = _new_spreadsheet(
        spreadsheet_id=spreadsheet_id, title=title, headers=headers
    )
    state["spreadsheets"][spreadsheet_id] = spreadsheet
    return web.json_response({"ok": True, "spreadsheet_id": spreadsheet_id})


async def list_spreadsheets(request: web.Request) -> web.Response:
    state = request.app["state"]
    return web.json_response(
        {
            "ok": True,
            "spreadsheets": [
                {
                    "spreadsheetId": s["spreadsheetId"],
                    "title": s["properties"]["title"],
                    "rows": list(s["_rows"]),
                    "row_count": len(s["_rows"]),
                }
                for s in state["spreadsheets"].values()
            ],
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
        f"[sheets_mock] {request.method} {request.path}"
        f"{('?' + request.query_string) if request.query_string else ''}",
        flush=True,
    )
    return await handler(request)


def make_app() -> web.Application:
    app = web.Application(middlewares=[_request_logger])
    app["state"] = _new_state()

    # Sheets v4 surface — both with and without /v4/ prefix to be
    # tolerant of either client SDK shape.
    for prefix in ("/v4", ""):
        app.router.add_post(f"{prefix}/spreadsheets", create_spreadsheet)
        app.router.add_post(
            prefix
            + "/spreadsheets/{spreadsheet_id}/values/{range}:append",
            values_append,
        )
        app.router.add_get(
            prefix + "/spreadsheets/{spreadsheet_id}/values/{range}",
            values_get,
        )
        app.router.add_get(
            prefix + "/spreadsheets/{spreadsheet_id}", get_spreadsheet
        )

    # Test hooks
    app.router.add_post("/__mock/seed_spreadsheet", seed_spreadsheet)
    app.router.add_get("/__mock/spreadsheets", list_spreadsheets)
    app.router.add_get("/__mock/captured", list_captured)
    app.router.add_post("/__mock/reset", reset_state)
    return app


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Mock Google Sheets API v4 for IronClaw workflow-canary."
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
        print(f"MOCK_SHEETS_PORT={bound}", flush=True)
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
