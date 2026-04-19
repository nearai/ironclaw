#!/usr/bin/env python3
"""Shared client logic for internal and external IronClaw agent examples."""

import argparse
import asyncio
import json
import os
import uuid
from datetime import datetime, timezone


class MissingWebsocketsError(RuntimeError):
    """Raised when the optional host-side websockets package is unavailable."""


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def envelope(msg_type: str, payload: dict) -> dict:
    return {
        "id": str(uuid.uuid4()),
        "type": msg_type,
        "timestamp": utc_now(),
        "payload": payload,
    }


def load_websockets():
    try:
        import websockets
    except ModuleNotFoundError as exc:
        if exc.name == "websockets":
            raise MissingWebsocketsError(
                "Missing Python package 'websockets'. Install it with: "
                "python3 -m pip install --break-system-packages websockets"
            ) from exc
        raise

    return websockets


async def run_agent(
    *,
    name: str,
    default_ws_url: str,
    default_task_prompt: str = "Write a hello world Python script",
    default_task_path: str = "/workspace",
    ws_url: str | None = None,
    auth_token: str | None = None,
    task_prompt: str | None = None,
    task_path: str | None = None,
    task_id: str | None = None,
    timeout_ms: int | None = None,
    exit_on_result: bool = False,
) -> None:
    ws_url = ws_url if ws_url is not None else os.environ.get("WS_URL", default_ws_url)
    auth_token = (
        auth_token if auth_token is not None else os.environ.get("AGENT_AUTH_TOKEN", "")
    )
    if task_prompt is None:
        task_prompt = os.environ.get("TASK_PROMPT", default_task_prompt)
    if task_path is None:
        task_path = os.environ.get("TASK_PATH", default_task_path)
    task_id = task_id if task_id is not None else os.environ.get("TASK_ID", "demo-task-001")
    if timeout_ms is None:
        timeout_ms = int(os.environ.get("TASK_TIMEOUT_MS", "300000"))
    headers = {}
    websockets = load_websockets()

    if auth_token:
        headers["Authorization"] = f"Bearer {auth_token}"

    async with websockets.connect(
        ws_url,
        additional_headers=headers or None,
        subprotocols=["ironclaw-agent-v1"],
    ) as websocket:
        print(f"[{name}] connected to {ws_url}", flush=True)

        ready_raw = await websocket.recv()
        ready_msg = json.loads(ready_raw)
        print(
            f"[{name}] received {ready_msg.get('type')}: {json.dumps(ready_msg)}",
            flush=True,
        )

        task_request = envelope(
            "task_request",
            {
                "task_id": task_id,
                "prompt": task_prompt,
                "context": {"path": task_path},
                "timeout_ms": timeout_ms,
            },
        )
        await websocket.send(json.dumps(task_request))
        print(
            f"[{name}] sent task_request: {json.dumps(task_request)}",
            flush=True,
        )

        async for raw_message in websocket:
            message = json.loads(raw_message)
            print(
                f"[{name}] received {message.get('type')}: {json.dumps(message)}",
                flush=True,
            )
            if exit_on_result and message.get("type") == "task_result":
                return


def build_arg_parser(
    *,
    name: str,
    default_ws_url: str,
    default_task_prompt: str,
    default_task_path: str,
) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=f"Connect to an IronClaw worker websocket and submit a task as {name}."
    )
    parser.add_argument(
        "--ws-url",
        help=f"Worker websocket URL. Defaults to WS_URL or {default_ws_url}.",
    )
    parser.add_argument(
        "--auth-token",
        help="Bearer token for websocket auth. Defaults to AGENT_AUTH_TOKEN.",
    )
    parser.add_argument(
        "--prompt",
        dest="task_prompt",
        help=f"Task prompt. Defaults to TASK_PROMPT or {default_task_prompt!r}.",
    )
    parser.add_argument(
        "--task-path",
        help=(
            "Task working path in the worker container. "
            f"Defaults to TASK_PATH or {default_task_path}."
        ),
    )
    parser.add_argument(
        "--task-id",
        help="Task identifier. Defaults to TASK_ID or demo-task-001.",
    )
    parser.add_argument(
        "--timeout-ms",
        type=int,
        help="Task timeout in milliseconds. Defaults to TASK_TIMEOUT_MS or 300000.",
    )
    parser.add_argument(
        "--exit-on-result",
        action="store_true",
        help="Exit after the first task_result message instead of keeping the websocket open.",
    )
    return parser


def run_agent_cli(
    *,
    name: str,
    default_ws_url: str,
    default_task_prompt: str = "Write a hello world Python script",
    default_task_path: str = "/workspace",
) -> None:
    parser = build_arg_parser(
        name=name,
        default_ws_url=default_ws_url,
        default_task_prompt=default_task_prompt,
        default_task_path=default_task_path,
    )
    args = parser.parse_args()
    try:
        asyncio.run(
            run_agent(
                name=name,
                default_ws_url=default_ws_url,
                default_task_prompt=default_task_prompt,
                default_task_path=default_task_path,
                ws_url=args.ws_url,
                auth_token=args.auth_token,
                task_prompt=args.task_prompt,
                task_path=args.task_path,
                task_id=args.task_id,
                timeout_ms=args.timeout_ms,
                exit_on_result=args.exit_on_result,
            )
        )
    except MissingWebsocketsError as exc:
        parser.exit(1, f"[{name}] {exc}\n")


def main() -> None:
    name = os.environ.get("AGENT_EXAMPLE_NAME", "ironclaw-agent-example")
    default_ws_url = os.environ.get(
        "AGENT_DEFAULT_WS_URL",
        "ws://127.0.0.1:9090/ws/agent",
    )
    default_task_prompt = os.environ.get(
        "AGENT_DEFAULT_TASK_PROMPT",
        "Write a hello world Python script",
    )
    default_task_path = os.environ.get("AGENT_DEFAULT_TASK_PATH", "/workspace")
    run_agent_cli(
        name=name,
        default_ws_url=default_ws_url,
        default_task_prompt=default_task_prompt,
        default_task_path=default_task_path,
    )


if __name__ == "__main__":
    main()
