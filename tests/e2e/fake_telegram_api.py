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
        self.reset()

    def reset(self):
        self.sent_messages: list[dict] = []
        self.chat_actions: list[dict] = []
        self.api_calls: list[dict] = []
        self._update_queue: list[dict] = []
        self._next_update_id = 1
        self._update_event = asyncio.Event()

    def queue_update(self, update: dict) -> int:
        update_id = self._next_update_id
        self._next_update_id += 1
        update["update_id"] = update_id
        self._update_queue.append(update)
        self._update_event.set()
        return update_id

    async def get_updates(self, offset: int = 0, timeout: float = 0) -> list:
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
            self._update_queue = [
                u for u in self._update_queue if u["update_id"] >= offset
            ]
            return list(self._update_queue)
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
    state.api_calls.append(
        {"method": "sendMessage", "body": body, "time": time.time()}
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
