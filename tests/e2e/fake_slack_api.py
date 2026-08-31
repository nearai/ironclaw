"""Fake Slack Web API server for E2E tests.

Serves minimal Slack API endpoints so the IronClaw Slack channel can be set
up and exercised without a real Slack connection: conventional messaging
(`chat.postMessage`), the OAuth v2 token exchange, private file downloads,
and the native Agent reply surface the stream reply transport drives —
`agents.sessions.setStatus` / `agents.sessions.rename`, `chat.startStream` /
`chat.appendStream` / `chat.stopStream` (a state machine per streaming
message), and the `conversations.replies` read-back of a stream's
accumulated text.

Control endpoints (/__mock/*) let tests inspect sent messages and streams,
configure failure modes, and reset state between scenarios.
"""

import argparse
import asyncio
import json
import time

from aiohttp import web


class FakeSlackState:
    """Shared mutable state for the fake Slack API."""

    def __init__(self):
        self.reset()

    def reset(self):
        self.sent_messages: list[dict] = []
        self.api_calls: list[dict] = []
        self.rate_limit_count = 0
        self.fail_post_message = False
        self.fail_file_downloads = False
        # Native Agent surface: streaming messages keyed by ts, session
        # status calls in order, and the failure modes a scenario can arm.
        self.streams: dict[str, dict] = {}
        self.sessions: list[dict] = []
        self.stream_seq = 0
        # Answer `feature_disabled` to the session/streaming methods, as a
        # workspace whose app lacks the Agents feature does.
        self.agent_feature_disabled = False
        # Identity returned by the OAuth v2 token endpoint (user-token flow).
        self.oauth_authed_user_id = "U42OWNER"
        self.oauth_team_id = "T0001"
        self.oauth_app_id = "A0001"


def _record(state: FakeSlackState, method: str, body: dict) -> None:
    state.api_calls.append({"method": method, "body": body, "time": time.time()})


def _mint_ts(state: FakeSlackState, prefix: str) -> str:
    state.stream_seq += 1
    return f"{prefix}.{state.stream_seq:06d}"


def _slack_error(error: str, status: int = 200) -> web.Response:
    return web.json_response({"ok": False, "error": error}, status=status)


async def _json_body(request: web.Request) -> dict:
    try:
        body = await request.json()
    except Exception:  # noqa: BLE001 - form-encoded fallback
        body = dict(await request.post())
    return body if isinstance(body, dict) else {}


def _absorb_chunks(stream: dict, body: dict) -> str | None:
    """Fold `markdown_text` / `chunks` into the stream; return an error name
    for a chunk shape Slack would reject."""
    text = body.get("markdown_text")
    if isinstance(text, str):
        stream["text"] += text
    chunks = body.get("chunks")
    if chunks is None:
        return None
    if not isinstance(chunks, list):
        return "invalid_chunks"
    for chunk in chunks:
        kind = chunk.get("type") if isinstance(chunk, dict) else None
        if kind == "markdown_text":
            piece = chunk.get("text")
            if not isinstance(piece, str) or len(piece) > 12_000:
                return "invalid_chunks"
            stream["text"] += piece
        elif kind == "task_update":
            if chunk.get("status") not in ("in_progress", "complete", "error"):
                return "invalid_chunks"
            if not isinstance(chunk.get("id"), str):
                return "invalid_chunks"
            stream["task_updates"].append(chunk)
        elif kind in ("plan_update", "blocks"):
            continue
        else:
            return "invalid_chunks"
    return None


# -- Slack API handlers ----------------------------------------------------


async def chat_post_message(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    body = await request.json()
    _record(state, "chat.postMessage", body)

    # Simulate Slack 429 rate limiting
    if state.rate_limit_count > 0:
        state.rate_limit_count -= 1
        return web.json_response(
            {"ok": False, "error": "rate_limited"},
            status=429,
            headers={"Retry-After": "1"},
        )

    # Simulate forced 500 errors
    if state.fail_post_message:
        return web.json_response(
            {"ok": False, "error": "internal_error"},
            status=500,
        )

    state.sent_messages.append(body)
    ts = f"{time.time():.6f}"
    return web.json_response(
        {
            "ok": True,
            "channel": body.get("channel", "C0001"),
            "ts": ts,
            "message": {
                "text": body.get("text", ""),
                "ts": ts,
                "type": "message",
            },
        }
    )


async def agents_sessions_set_status(request: web.Request) -> web.Response:
    """`agents.sessions.setStatus`: processing | active | suspended | closed."""
    state: FakeSlackState = request.app["state"]
    body = await _json_body(request)
    _record(state, "agents.sessions.setStatus", body)
    if state.agent_feature_disabled:
        return _slack_error("feature_disabled")
    status = body.get("status")
    if status not in ("processing", "active", "suspended", "closed"):
        return _slack_error("invalid_status")
    state.sessions.append(
        {
            "channel_id": body.get("channel_id"),
            "thread_ts": body.get("thread_ts"),
            "status": status,
            "title": body.get("title"),
            "time": time.time(),
        }
    )
    return web.json_response(
        {"ok": True, "status": status, "agent_status": status, "title": body.get("title")}
    )


async def agents_sessions_rename(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    body = await _json_body(request)
    _record(state, "agents.sessions.rename", body)
    if state.agent_feature_disabled:
        return _slack_error("feature_disabled")
    title = body.get("title")
    if not isinstance(title, str) or not 1 <= len(title) <= 200:
        return _slack_error("invalid_arguments")
    state.sessions.append(
        {
            "channel_id": body.get("channel_id"),
            "thread_ts": body.get("thread_ts"),
            "rename": title,
            "time": time.time(),
        }
    )
    return web.json_response({"ok": True, "title": title})


async def chat_start_stream(request: web.Request) -> web.Response:
    """`chat.startStream`: opens one streaming message in a thread."""
    state: FakeSlackState = request.app["state"]
    body = await _json_body(request)
    _record(state, "chat.startStream", body)
    if state.agent_feature_disabled:
        return _slack_error("feature_disabled")
    channel = body.get("channel")
    if not isinstance(channel, str) or not channel:
        return _slack_error("channel_not_found")
    # recipient_user_id / recipient_team_id are "Required when streaming to
    # channels".
    if not channel.startswith("D"):
        if not body.get("recipient_user_id"):
            return _slack_error("missing_recipient_user_id")
        if not body.get("recipient_team_id"):
            return _slack_error("missing_recipient_team_id")
    if body.get("markdown_text") is not None and body.get("chunks") is not None:
        return _slack_error("cannot_provide_both_markdown_text_and_chunks")
    ts = _mint_ts(state, "1710000100")
    stream = {
        "channel": channel,
        "thread_ts": body.get("thread_ts"),
        "recipient_user_id": body.get("recipient_user_id"),
        "recipient_team_id": body.get("recipient_team_id"),
        "task_display_mode": body.get("task_display_mode", "timeline"),
        "text": "",
        "task_updates": [],
        "state": "streaming",
        "session_status": None,
        "append_calls": 0,
    }
    error = _absorb_chunks(stream, body)
    if error:
        return _slack_error(error)
    state.streams[ts] = stream
    return web.json_response({"ok": True, "channel": channel, "ts": ts})


async def chat_append_stream(request: web.Request) -> web.Response:
    """`chat.appendStream`: `markdown_text`/chunks are appended (deltas)."""
    state: FakeSlackState = request.app["state"]
    body = await _json_body(request)
    _record(state, "chat.appendStream", body)
    stream = state.streams.get(body.get("ts"))
    if stream is None:
        return _slack_error("message_not_found")
    if stream["state"] == "stopped_by_user":
        return _slack_error("stopped_by_user")
    if stream["state"] != "streaming":
        return _slack_error("message_not_in_streaming_state")
    if body.get("markdown_text") is None and body.get("chunks") is None:
        return _slack_error("markdown_text_or_chunks_required")
    stream["append_calls"] += 1
    error = _absorb_chunks(stream, body)
    if error:
        return _slack_error(error)
    return web.json_response({"ok": True, "channel": stream["channel"], "ts": body.get("ts")})


async def chat_stop_stream(request: web.Request) -> web.Response:
    """`chat.stopStream`: closes the stream; `session_status` defaults to active."""
    state: FakeSlackState = request.app["state"]
    body = await _json_body(request)
    _record(state, "chat.stopStream", body)
    ts = body.get("ts")
    stream = state.streams.get(ts)
    if stream is None:
        return _slack_error("message_not_found")
    if stream["state"] != "streaming":
        return _slack_error("message_not_in_streaming_state")
    error = _absorb_chunks(stream, body)
    if error:
        return _slack_error(error)
    session_status = body.get("session_status") or "active"
    if session_status not in ("processing", "active", "suspended", "closed"):
        return _slack_error("invalid_arguments")
    stream["state"] = "stopped"
    stream["session_status"] = session_status
    state.sessions.append(
        {
            "channel_id": stream["channel"],
            "thread_ts": stream["thread_ts"],
            "status": session_status,
            "via": "chat.stopStream",
            "time": time.time(),
        }
    )
    # The finished stream is a sent message too, so scenarios that count
    # replies through /__mock/sent_messages see it.
    state.sent_messages.append(
        {
            "channel": stream["channel"],
            "thread_ts": stream["thread_ts"],
            "text": stream["text"],
            "ts": ts,
            "streamed": True,
        }
    )
    return web.json_response(
        {
            "ok": True,
            "channel": stream["channel"],
            "ts": ts,
            "message": {
                "text": stream["text"],
                "ts": ts,
                "type": "message",
                "subtype": "bot_message",
            },
        }
    )


async def conversations_replies(request: web.Request) -> web.Response:
    """`conversations.replies` for one message ts: the stream's text so far."""
    state: FakeSlackState = request.app["state"]
    ts = request.query.get("ts")
    _record(state, "conversations.replies", dict(request.query))
    stream = state.streams.get(ts)
    if stream is not None:
        return web.json_response(
            {
                "ok": True,
                "messages": [{"type": "message", "ts": ts, "text": stream["text"]}],
                "has_more": False,
            }
        )
    for message in state.sent_messages:
        if message.get("ts") == ts:
            return web.json_response(
                {
                    "ok": True,
                    "messages": [
                        {"type": "message", "ts": ts, "text": message.get("text", "")}
                    ],
                    "has_more": False,
                }
            )
    return _slack_error("thread_not_found")


async def download_file(request: web.Request) -> web.Response:
    """Serve fake file content for Slack file downloads."""
    state: FakeSlackState = request.app["state"]
    file_path = request.match_info.get("file_path", "unknown")
    state.api_calls.append(
        {"method": "file_download", "file_path": file_path, "time": time.time()}
    )

    if state.fail_file_downloads:
        return web.Response(status=500, text="Internal Server Error")

    return web.Response(
        body=b"fake slack file content",
        content_type="application/octet-stream",
    )


async def oauth_v2_access(request: web.Request) -> web.Response:
    """Slack OAuth v2 token endpoint (authorization-code exchange).

    Shape mirrors Slack's user-token (`user_scope`) response: the personal
    access token and proven identity ride `authed_user`, workspace/app
    claims ride `team` / `app_id`.
    """
    state: FakeSlackState = request.app["state"]
    body = dict(await request.post())
    state.api_calls.append(
        {
            "method": "oauth.v2.access",
            "body": {k: v for k, v in body.items() if k not in ("client_secret",)},
            "time": time.time(),
        }
    )
    return web.json_response(
        {
            "ok": True,
            "app_id": state.oauth_app_id,
            "authed_user": {
                "id": state.oauth_authed_user_id,
                "scope": (
                    "search:read,channels:history,groups:history,im:history,"
                    "mpim:history,channels:read,groups:read,im:read,mpim:read,"
                    "users:read,chat:write"
                ),
                "access_token": "xoxp-FAKE-SLACK-USER-TOKEN",
                "token_type": "user",
            },
            "team": {"id": state.oauth_team_id, "name": "Fake Workspace"},
            "enterprise": None,
        }
    )


# -- Control endpoints -----------------------------------------------------


async def mock_sent_messages(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    return web.json_response({"messages": state.sent_messages})


async def mock_streams(request: web.Request) -> web.Response:
    """Every streaming message (by ts) and every session status call."""
    state: FakeSlackState = request.app["state"]
    return web.json_response({"streams": state.streams, "sessions": state.sessions})


async def mock_api_calls(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    return web.json_response({"calls": state.api_calls})


async def mock_reset(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    state.reset()
    return web.json_response({"ok": True})


async def mock_set_rate_limit(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    body = await request.json()
    state.rate_limit_count = int(body.get("count", 0))
    return web.json_response({"ok": True, "rate_limit_count": state.rate_limit_count})


async def mock_set_fail_post_message(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    body = await request.json()
    state.fail_post_message = bool(body.get("fail", False))
    return web.json_response(
        {"ok": True, "fail_post_message": state.fail_post_message}
    )


async def mock_set_fail_downloads(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    body = await request.json()
    state.fail_file_downloads = bool(body.get("fail", False))
    return web.json_response(
        {"ok": True, "fail_file_downloads": state.fail_file_downloads}
    )


async def mock_set_agent_feature_disabled(request: web.Request) -> web.Response:
    """Arm `feature_disabled` on the session/streaming methods."""
    state: FakeSlackState = request.app["state"]
    body = await request.json()
    state.agent_feature_disabled = bool(body.get("disabled", False))
    return web.json_response(
        {"ok": True, "agent_feature_disabled": state.agent_feature_disabled}
    )


async def mock_stop_stream_by_user(request: web.Request) -> web.Response:
    """Slack's stop button: the stream stops accepting appends."""
    state: FakeSlackState = request.app["state"]
    body = await request.json()
    stream = state.streams.get(body.get("ts"))
    if stream is None:
        return web.json_response({"ok": False, "error": "message_not_found"}, status=404)
    stream["state"] = "stopped_by_user"
    return web.json_response({"ok": True})


# -- Server entry point ----------------------------------------------------


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()

    app = web.Application()
    app["state"] = FakeSlackState()

    # Slack Web API
    app.router.add_post("/api/chat.postMessage", chat_post_message)
    app.router.add_post("/api/oauth.v2.access", oauth_v2_access)
    # Native Agent reply surface
    app.router.add_post("/api/agents.sessions.setStatus", agents_sessions_set_status)
    app.router.add_post("/api/agents.sessions.rename", agents_sessions_rename)
    app.router.add_post("/api/chat.startStream", chat_start_stream)
    app.router.add_post("/api/chat.appendStream", chat_append_stream)
    app.router.add_post("/api/chat.stopStream", chat_stop_stream)
    app.router.add_get("/api/conversations.replies", conversations_replies)

    # File downloads (Slack serves files from files.slack.com/files-pri/...)
    app.router.add_get("/files-pri/{file_path:.*}", download_file)
    app.router.add_get("/files/{file_path:.*}", download_file)

    # Control endpoints
    app.router.add_get("/__mock/sent_messages", mock_sent_messages)
    app.router.add_get("/__mock/streams", mock_streams)
    app.router.add_get("/__mock/api_calls", mock_api_calls)
    app.router.add_post("/__mock/reset", mock_reset)
    app.router.add_post("/__mock/set_rate_limit", mock_set_rate_limit)
    app.router.add_post("/__mock/set_fail_post_message", mock_set_fail_post_message)
    app.router.add_post("/__mock/set_fail_downloads", mock_set_fail_downloads)
    app.router.add_post(
        "/__mock/set_agent_feature_disabled", mock_set_agent_feature_disabled
    )
    app.router.add_post("/__mock/stop_stream_by_user", mock_stop_stream_by_user)

    async def start():
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        print(f"FAKE_SLACK_PORT={port}", flush=True)
        await asyncio.Event().wait()

    asyncio.run(start())


if __name__ == "__main__":
    main()
