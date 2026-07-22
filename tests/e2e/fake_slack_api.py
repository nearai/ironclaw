"""Fake Slack Web API server for E2E tests.

Serves minimal Slack API endpoints so the IronClaw Slack WASM channel can be
set up and exercised without a real Slack connection.

Control endpoints (/__mock/*) let tests inspect sent messages, configure
failure modes, and reset state between scenarios.
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
        # Identity returned by the OAuth v2 token endpoint (user-token flow).
        self.oauth_authed_user_id = "U42OWNER"
        self.oauth_team_id = "T0001"
        self.oauth_app_id = "A0001"


# -- Slack API handlers ----------------------------------------------------


async def chat_post_message(request: web.Request) -> web.Response:
    state: FakeSlackState = request.app["state"]
    body = await request.json()
    state.api_calls.append(
        {"method": "chat.postMessage", "body": body, "time": time.time()}
    )

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

    # File downloads (Slack serves files from files.slack.com/files-pri/...)
    app.router.add_get("/files-pri/{file_path:.*}", download_file)
    app.router.add_get("/files/{file_path:.*}", download_file)

    # Control endpoints
    app.router.add_get("/__mock/sent_messages", mock_sent_messages)
    app.router.add_get("/__mock/api_calls", mock_api_calls)
    app.router.add_post("/__mock/reset", mock_reset)
    app.router.add_post("/__mock/set_rate_limit", mock_set_rate_limit)
    app.router.add_post("/__mock/set_fail_post_message", mock_set_fail_post_message)
    app.router.add_post("/__mock/set_fail_downloads", mock_set_fail_downloads)

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
