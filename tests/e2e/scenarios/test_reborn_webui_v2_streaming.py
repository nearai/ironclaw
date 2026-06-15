"""Reborn WebUI v2 tier-2 coverage: streaming transports and protocol edges.

Exercises the parts of the v2 surface that only manifest over a real TCP
listener — behaviour the in-process `tower::ServiceExt::oneshot` composition
tests in `crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs` cannot
reach: the WebSocket transport and its same-origin upgrade gate, SSE resume via
`Last-Event-ID`, the per-caller SSE/WS concurrency cap, and a couple of
protocol-edge status codes over the wire.

Rate-limit 429 (the 60/120/30-per-60s descriptor budgets) is intentionally NOT
exercised here: it is covered in-process by `webui_v2_serve.rs`, and replaying
60+ mutations against the module-shared caller would exhaust the sliding window
for sibling tests. The SSE concurrency cap below is the per-caller 429 path that
genuinely needs concurrent TCP connections.

Tracks nearai/ironclaw#4634.
"""

import asyncio
import contextlib
import json

import aiohttp
import httpx

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_v2_support import client_action_id, create_thread, send_message

_BEARER = {"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}


def _events_url(base_url: str, thread_id: str) -> str:
    return (
        f"{base_url}/api/webchat/v2/threads/{thread_id}/events"
        f"?token={REBORN_V2_AUTH_TOKEN}"
    )


def _ws_url(base_url: str, thread_id: str) -> str:
    return f"{base_url}/api/webchat/v2/threads/{thread_id}/ws"


async def test_reborn_v2_websocket_streams_projection_frames(reborn_v2_server):
    """The WS transport upgrades with same-origin + bearer and fans out frames.

    WS frames are raw `ProductOutboundEnvelope`s (no SSE `type` wrapper): each
    carries the `webui_v2` adapter id and an opaque `projection_cursor`, the
    same cursor source of truth the SSE `id:` stream uses.
    """
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)

    async with aiohttp.ClientSession() as session:
        async with session.ws_connect(
            _ws_url(reborn_v2_server, thread_id),
            headers={"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}", "Origin": reborn_v2_server},
        ) as ws:
            async with httpx.AsyncClient(headers=_BEARER) as client:
                await send_message(client, reborn_v2_server, thread_id, "echo hello world")

            frames = []
            async with asyncio.timeout(30):
                async for msg in ws:
                    if msg.type != aiohttp.WSMsgType.TEXT:
                        continue
                    envelope = json.loads(msg.data)
                    frames.append(envelope)
                    if len(frames) >= 3:
                        break

    assert frames, "WS must fan out at least one projection frame after a turn"
    for envelope in frames:
        assert envelope.get("adapter_id") == "webui_v2", envelope
        assert envelope.get("projection_cursor"), (
            f"every WS frame must carry a projection cursor: {envelope}"
        )


async def test_reborn_v2_websocket_enforces_origin_and_bearer(reborn_v2_server):
    """WS requires same-origin AND a bearer; `?token=` does not authenticate it."""
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)

    ws_url = _ws_url(reborn_v2_server, thread_id)
    bearer = f"Bearer {REBORN_V2_AUTH_TOKEN}"

    async def expect_handshake_status(*, headers, url, expected):
        async with aiohttp.ClientSession() as session:
            try:
                async with session.ws_connect(url, headers=headers):
                    raise AssertionError(f"WS unexpectedly upgraded (wanted {expected})")
            except aiohttp.WSServerHandshakeError as exc:
                assert exc.status == expected, f"wanted {expected}, got {exc.status}"

    # Cross-origin upgrade is rejected before auth (same-origin policy).
    await expect_handshake_status(
        headers={"Authorization": bearer, "Origin": "http://evil.example"},
        url=ws_url,
        expected=403,
    )
    # The browser does not pre-flight WS, so a missing Origin is also rejected.
    await expect_handshake_status(
        headers={"Authorization": bearer},
        url=ws_url,
        expected=403,
    )
    # The `?token=` shim is scoped to the SSE events route only — never WS.
    await expect_handshake_status(
        headers={"Origin": reborn_v2_server},
        url=f"{ws_url}?token={REBORN_V2_AUTH_TOKEN}",
        expected=401,
    )


async def _read_sse_id_type_pairs(response, *, limit, timeout):
    """Read up to `limit` (event_id, event_type) pairs from an open SSE response.

    SSE frames arrive as `event:`/`id:`/`data:` lines terminated by a blank
    line. Fields are accumulated per frame and flushed on the frame boundary so
    each event's `id:` is paired with its own `event:` regardless of line order.
    """
    pairs = []
    cur_id = None
    cur_type = None
    async with asyncio.timeout(timeout):
        async for raw in response.content:
            line = raw.decode("utf-8", errors="replace").rstrip("\n")
            if line.startswith("id:"):
                cur_id = line[len("id:") :].strip()
            elif line.startswith("event:"):
                cur_type = line[len("event:") :].strip()
            elif line == "":
                # Frame boundary: flush whatever fields the frame carried.
                if cur_type is not None or cur_id is not None:
                    pairs.append((cur_id, cur_type))
                    if len(pairs) >= limit:
                        break
                cur_id = None
                cur_type = None
    return pairs


async def test_reborn_v2_sse_resumes_after_last_event_id(reborn_v2_server):
    """Reconnecting with `Last-Event-ID` resumes after the cursor — no dup, no gap.

    Each SSE frame carries an opaque `id:` (the serialized projection cursor).
    A reconnect that presents the first frame's id must NOT redeliver that frame
    and must continue with the events recorded after it.
    """
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        await send_message(client, reborn_v2_server, thread_id, "echo hello world")

    # Let the turn finish so the projection has a full event history to replay.
    await asyncio.sleep(3)
    url = _events_url(reborn_v2_server, thread_id)

    async with aiohttp.ClientSession(
        timeout=aiohttp.ClientTimeout(total=20, sock_read=20)
    ) as session:
        async with session.get(url, headers={"Accept": "text/event-stream"}) as first:
            assert first.status == 200, first.status
            initial = await _read_sse_id_type_pairs(first, limit=3, timeout=10)

    assert len(initial) >= 2, f"need >=2 SSE frames to test resume, got {initial}"
    cursor_id, _cursor_type = initial[0]
    assert cursor_id, f"SSE frames must expose an id: line, got {initial}"

    async with aiohttp.ClientSession(
        timeout=aiohttp.ClientTimeout(total=20, sock_read=20)
    ) as session:
        async with session.get(
            url, headers={"Accept": "text/event-stream", "Last-Event-ID": cursor_id}
        ) as resumed:
            assert resumed.status == 200, resumed.status
            replay = await _read_sse_id_type_pairs(resumed, limit=3, timeout=10)

    assert replay, "resume must replay the events recorded after the cursor"
    replay_ids = [event_id for event_id, _ in replay]
    # No dup: the cursor's own frame is not re-sent.
    assert cursor_id not in replay_ids, (
        f"resume must not redeliver the cursor frame {cursor_id!r}: {replay_ids}"
    )
    # No gap: the frame that followed the cursor in the first pass reappears.
    next_id_after_cursor = initial[1][0]
    assert next_id_after_cursor in replay_ids, (
        f"resume must continue from the frame after the cursor "
        f"({next_id_after_cursor!r}); got {replay_ids}"
    )


async def test_reborn_v2_sse_concurrency_cap_returns_429(reborn_v2_server):
    """A 4th concurrent SSE stream for one caller is rejected with 429.

    The per-(tenant, user) `SseCapacity` cap is 3; SSE and WS share it. Holding
    three streams open and opening a fourth exercises the cap over real TCP
    connections (the in-process oneshot tests cannot hold concurrent streams).
    """
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)
    url = _events_url(reborn_v2_server, thread_id)

    async with aiohttp.ClientSession(
        timeout=aiohttp.ClientTimeout(total=30, sock_read=30)
    ) as session:
        async with contextlib.AsyncExitStack() as held:
            for _ in range(3):
                stream = await held.enter_async_context(
                    session.get(url, headers={"Accept": "text/event-stream"})
                )
                assert stream.status == 200, f"first 3 streams open, got {stream.status}"
                # Touch the body so the connection is fully established and the
                # slot is acquired before opening the next one.
                await stream.content.readline()

            async with session.get(
                url, headers={"Accept": "text/event-stream"}
            ) as overflow:
                assert overflow.status == 429, (
                    f"4th concurrent stream must be rejected with 429, got {overflow.status}"
                )


async def test_reborn_v2_oversized_create_thread_body_returns_413(reborn_v2_server):
    """A create-thread body past the 16 KiB descriptor cap is rejected with 413.

    The per-route body limit is enforced before auth/validation, so an oversized
    payload never reaches the facade.
    """
    oversized = {"client_action_id": client_action_id(), "padding": "x" * (32 * 1024)}
    async with httpx.AsyncClient(headers=_BEARER) as client:
        response = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/threads", json=oversized, timeout=15
        )
    assert response.status_code == 413, response.text


async def test_reborn_v2_malformed_run_id_returns_400(reborn_v2_server):
    """A non-UUID run id in the cancel path is a 400 validation error, not a 404."""
    async with httpx.AsyncClient(headers=_BEARER) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        response = await client.post(
            f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
            f"/runs/not-a-valid-uuid/cancel",
            json={"client_action_id": client_action_id(), "reason": "user_requested"},
            timeout=15,
        )
    assert response.status_code == 400, response.text
    body = response.json()
    assert body.get("field") == "run_id", body
    assert body.get("validation_code") == "invalid_id", body
