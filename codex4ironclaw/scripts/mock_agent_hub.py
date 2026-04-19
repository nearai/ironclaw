#!/usr/bin/env python3
"""Minimal mock IronClaw agent hub for local websocket testing."""

import asyncio
import json
import uuid
from datetime import datetime, timezone

import websockets


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def _envelope(msg_type: str, payload: dict) -> dict:
    return {
        "id": str(uuid.uuid4()),
        "type": msg_type,
        "timestamp": _utc_now(),
        "payload": payload,
    }


async def handle_connection(websocket) -> None:
    print("[mock_agent_hub] client connected", flush=True)

    try:
        ready_raw = await websocket.recv()
        ready_msg = json.loads(ready_raw)
        print(
            f"[mock_agent_hub] received {ready_msg.get('type')}: {json.dumps(ready_msg)}",
            flush=True,
        )

        task_request = _envelope(
            "task_request",
            {
                "task_id": "demo-task-001",
                "prompt": "Write a hello world Python script",
                "context": {"path": "/workspace"},
                "timeout_ms": 30000,
            },
        )
        await websocket.send(json.dumps(task_request))
        print(
            f"[mock_agent_hub] sent task_request: {json.dumps(task_request)}",
            flush=True,
        )

        async for raw_message in websocket:
            message = json.loads(raw_message)
            print(
                f"[mock_agent_hub] received {message.get('type')}: {json.dumps(message)}",
                flush=True,
            )
    except websockets.ConnectionClosed:
        print("[mock_agent_hub] client disconnected", flush=True)


async def main() -> None:
    async with websockets.serve(handle_connection, "0.0.0.0", 9000):
        print("[mock_agent_hub] listening on ws://0.0.0.0:9000/codex", flush=True)
        await asyncio.Future()


if __name__ == "__main__":
    asyncio.run(main())
