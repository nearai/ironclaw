"""Fake Telegram Bot API server for E2E tests.

Serves minimal Telegram Bot API endpoints so the IronClaw Telegram WASM
channel can be set up and exercised without a real Telegram connection.

Control endpoints (/__mock/*) let tests queue updates, inspect sent
messages, and reset state between scenarios.
"""

import argparse
import asyncio
import json
import time

from aiohttp import web


class FakeTelegramState:
    """Shared mutable state for the fake Telegram API."""

    def __init__(self):
        self._update_event = asyncio.Event()
        self._next_update_id = 1
        self.reset()

    def reset(self):
        next_update_id = self._next_update_id
        self.sent_messages: list[dict] = []
        self.chat_actions: list[dict] = []
        self.api_calls: list[dict] = []
        self._update_queue: list[dict] = []
        self._next_update_id = next_update_id
        self._update_event.clear()
        self.reject_markdown = False
        self.rate_limit_count = 0
        self.fail_downloads = False

    def queue_update(self, update: dict) -> int:
        explicit_update_id = update.get("update_id")
        if isinstance(explicit_update_id, int) and explicit_update_id > 0:
            update_id = explicit_update_id
            self._next_update_id = max(self._next_update_id, update_id + 1)
        else:
            update_id = self._next_update_id
            self._next_update_id += 1
        update["update_id"] = update_id
        self._update_queue.append(update)
        self._update_event.set()
        return update_id

    async def get_updates(self, offset: int = 0, timeout: float = 0) -> list:
        # Filter once, then snapshot. The snapshot avoids a TOCTOU race where
        # queue_update() could append between filtering and returning (the
        # await below yields control to the event loop).
        self._update_queue = [u for u in self._update_queue if u["update_id"] >= offset]
        if self._update_queue:
            return list(self._update_queue)
        if timeout > 0:
            self._update_event.clear()
            try:
                await asyncio.wait_for(
                    self._update_event.wait(), timeout=min(timeout, 5)
                )
            except asyncio.TimeoutError:
                pass
            # Re-filter and snapshot after the await.
            filtered = [u for u in self._update_queue if u["update_id"] >= offset]
            self._update_queue = filtered
            return list(filtered)
        return []


# ── Bot API handlers ─────────────────────────────────────────────────────


async def get_me(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    state.api_calls.append({"method": "getMe", "time": time.time()})
    return web.json_response(
        {
            "ok": True,
            "result": {
                "id": 9876543210,
                "is_bot": True,
                "first_name": "E2E Test Bot",
                "username": "e2e_test_bot",
                "can_join_groups": True,
                "can_read_all_group_messages": False,
                "supports_inline_queries": False,
            },
        }
    )


async def delete_webhook(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    state.api_calls.append({"method": "deleteWebhook", "time": time.time()})
    return web.json_response({"ok": True, "result": True})


async def set_webhook(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    try:
        body = await request.json()
    except Exception:
        body = dict(request.query)
    state.api_calls.append(
        {"method": "setWebhook", "body": body, "time": time.time()}
    )
    return web.json_response({"ok": True, "result": True})


async def get_updates(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    offset = int(request.query.get("offset", "0"))
    timeout = int(request.query.get("timeout", "0"))
    state.api_calls.append(
        {
            "method": "getUpdates",
            "offset": offset,
            "timeout": timeout,
            "time": time.time(),
        }
    )
    updates = await state.get_updates(offset=offset, timeout=timeout)
    return web.json_response({"ok": True, "result": updates})


async def send_message(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    body = await request.json()
    # Record every attempt in api_calls (including rejected ones) so tests can
    # inspect the full request sequence. Only successful calls are appended to
    # sent_messages below.
    state.api_calls.append(
        {"method": "sendMessage", "body": body, "time": time.time()}
    )
    # Simulate Telegram 429 rate limiting
    if state.rate_limit_count > 0:
        state.rate_limit_count -= 1
        return web.json_response(
            {
                "ok": False,
                "error_code": 429,
                "description": "Too Many Requests: retry after 1",
                "parameters": {"retry_after": 1},
            },
            status=429,
        )
    # Simulate Telegram rejecting Markdown when the flag is set
    if state.reject_markdown and "parse_mode" in body:
        return web.json_response(
            {
                "ok": False,
                "error_code": 400,
                "description": "Bad Request: can't parse entities",
            },
            status=400,
        )
    state.sent_messages.append(body)
    msg_id = len(state.sent_messages) + 1000
    return web.json_response(
        {
            "ok": True,
            "result": {
                "message_id": msg_id,
                "from": {
                    "id": 9876543210,
                    "is_bot": True,
                    "first_name": "E2E Test Bot",
                    "username": "e2e_test_bot",
                },
                "chat": {"id": body.get("chat_id", 0), "type": "private"},
                "date": int(time.time()),
                "text": body.get("text", ""),
            },
        }
    )


async def send_chat_action(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    body = await request.json()
    state.api_calls.append(
        {"method": "sendChatAction", "body": body, "time": time.time()}
    )
    state.chat_actions.append(body)
    return web.json_response({"ok": True, "result": True})


async def get_file(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    body = dict(request.query)
    state.api_calls.append({"method": "getFile", "body": body, "time": time.time()})
    # Simulate download failures
    if state.fail_downloads:
        return web.json_response(
            {
                "ok": False,
                "error_code": 500,
                "description": "Internal Server Error",
            },
            status=500,
        )
    file_id = body.get("file_id", "test_file_id")
    return web.json_response(
        {
            "ok": True,
            "result": {
                "file_id": file_id,
                "file_unique_id": f"unique_{file_id}",
                "file_size": 1024,
                "file_path": f"documents/{file_id}.pdf",
            },
        }
    )


async def download_file(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    file_path = request.match_info.get("file_path", "unknown")
    state.api_calls.append(
        {"method": "downloadFile", "file_path": file_path, "time": time.time()}
    )
    return web.Response(body=b"fake file content", content_type="application/octet-stream")


# ── Control endpoints ────────────────────────────────────────────────────


async def mock_queue_update(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    body = await request.json()
    update_id = state.queue_update(body)
    return web.json_response({"ok": True, "update_id": update_id})


async def mock_sent_messages(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    return web.json_response({"messages": state.sent_messages})


async def mock_chat_actions(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    return web.json_response({"actions": state.chat_actions})


async def mock_api_calls(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    return web.json_response({"calls": state.api_calls})


async def mock_reset(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    state.reset()
    return web.json_response({"ok": True})


async def mock_set_reject_markdown(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    body = await request.json()
    state.reject_markdown = bool(body.get("reject", False))
    return web.json_response({"ok": True, "reject_markdown": state.reject_markdown})


async def mock_set_rate_limit(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    body = await request.json()
    state.rate_limit_count = int(body.get("count", 0))
    return web.json_response({"ok": True, "rate_limit_count": state.rate_limit_count})


async def mock_set_fail_downloads(request: web.Request) -> web.Response:
    state: FakeTelegramState = request.app["state"]
    body = await request.json()
    state.fail_downloads = bool(body.get("fail", False))
    return web.json_response({"ok": True, "fail_downloads": state.fail_downloads})


# ── Server entry point ───────────────────────────────────────────────────


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()

    app = web.Application()
    app["state"] = FakeTelegramState()

    # Bot API — accept any token in the path
    app.router.add_route("*", "/bot{token}/getMe", get_me)
    app.router.add_post("/bot{token}/deleteWebhook", delete_webhook)
    app.router.add_post("/bot{token}/setWebhook", set_webhook)
    app.router.add_get("/bot{token}/getUpdates", get_updates)
    app.router.add_post("/bot{token}/sendMessage", send_message)
    app.router.add_post("/bot{token}/sendChatAction", send_chat_action)
    app.router.add_get("/bot{token}/getFile", get_file)
    app.router.add_get("/file/bot{token}/{file_path:.*}", download_file)

    # Control endpoints
    app.router.add_post("/__mock/queue_update", mock_queue_update)
    app.router.add_get("/__mock/sent_messages", mock_sent_messages)
    app.router.add_get("/__mock/chat_actions", mock_chat_actions)
    app.router.add_get("/__mock/api_calls", mock_api_calls)
    app.router.add_post("/__mock/reset", mock_reset)
    app.router.add_post("/__mock/set_reject_markdown", mock_set_reject_markdown)
    app.router.add_post("/__mock/set_rate_limit", mock_set_rate_limit)
    app.router.add_post("/__mock/set_fail_downloads", mock_set_fail_downloads)

    async def start():
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", args.port)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        print(f"FAKE_TELEGRAM_PORT={port}", flush=True)
        await asyncio.Event().wait()

    asyncio.run(start())


if __name__ == "__main__":
    main()
