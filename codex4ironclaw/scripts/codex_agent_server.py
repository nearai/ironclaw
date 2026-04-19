#!/usr/bin/env python3
"""Inbound IronClaw WebSocket server used by websocket mode."""

import asyncio
import json
import os

import websockets

from ironclaw_runtime import CodexTaskEndpoint, write_ws_state


def _request_path(websocket) -> str:
    request = getattr(websocket, "request", None)
    path = getattr(request, "path", None) if request is not None else None
    if path:
        return path
    return getattr(websocket, "path", "") or ""


def _request_headers(websocket):
    request = getattr(websocket, "request", None)
    headers = getattr(request, "headers", None) if request is not None else None
    if headers is not None:
        return headers
    return getattr(websocket, "request_headers", {}) or {}


class CodexAgentSession(CodexTaskEndpoint):
    def __init__(self, websocket) -> None:
        super().__init__()
        self.websocket = websocket

    async def send(self, msg_type: str, payload: dict) -> None:
        await self.websocket.send(json.dumps(self._envelope(msg_type, payload)))


class CodexAgentServer:
    def __init__(self) -> None:
        self.bind_host = os.environ.get("WS_BIND_HOST", "0.0.0.0")
        self.port = int(os.environ.get("WS_PORT", "9090"))
        self.path = os.environ.get("WS_PATH", "/ws/agent")
        self.auth_token = os.environ.get("AGENT_AUTH_TOKEN", "")
        self.active_connections: set[int] = set()

    def _write_state(self, listening: bool) -> None:
        write_ws_state(
            ready=listening,
            role="server",
            listening=listening,
            bind_host=self.bind_host,
            port=self.port,
            path=self.path,
            connections=len(self.active_connections),
        )

    async def handle_connection(self, websocket) -> None:
        request_path = _request_path(websocket).split("?", 1)[0] or "/"
        headers = _request_headers(websocket)
        expected_auth = f"Bearer {self.auth_token}"
        provided_auth = headers.get("Authorization", "")

        if request_path != self.path:
            print(
                f"[codex_agent_server] rejecting connection on unexpected path {request_path}",
                flush=True,
            )
            await websocket.close(code=1008, reason=f"Expected path {self.path}")
            return

        if self.auth_token and provided_auth != expected_auth:
            print("[codex_agent_server] rejecting unauthorized connection", flush=True)
            await websocket.close(code=4001, reason="Unauthorized")
            return

        connection_id = id(websocket)
        self.active_connections.add(connection_id)
        self._write_state(listening=True)
        session = CodexAgentSession(websocket)

        print(
            f"[codex_agent_server] agent connected on ws://{self.bind_host}:{self.port}{self.path}",
            flush=True,
        )

        try:
            await session.send_ready()
            async for message in websocket:
                await session.handle_message(message)
        except websockets.ConnectionClosed:
            print("[codex_agent_server] agent disconnected", flush=True)
        finally:
            self.active_connections.discard(connection_id)
            self._write_state(listening=True)

    async def run_forever(self) -> None:
        async with websockets.serve(
            self.handle_connection,
            self.bind_host,
            self.port,
            subprotocols=["ironclaw-agent-v1"],
        ):
            self._write_state(listening=True)
            print(
                f"[codex_agent_server] listening on ws://{self.bind_host}:{self.port}{self.path}",
                flush=True,
            )
            await asyncio.Future()


async def main() -> None:
    server = CodexAgentServer()
    try:
        await server.run_forever()
    finally:
        server._write_state(listening=False)


if __name__ == "__main__":
    asyncio.run(main())
