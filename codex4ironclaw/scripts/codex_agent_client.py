#!/usr/bin/env python3
"""Outbound IronClaw WebSocket client used when the worker connects to a hub."""

import argparse
import asyncio
import json
import os

from ironclaw_agent_example import MissingWebsocketsError, load_websockets, run_agent
from ironclaw_runtime import CodexTaskEndpoint, write_ws_state


class CodexAgentClient(CodexTaskEndpoint):
    def __init__(
        self,
        *,
        ws_url: str | None = None,
        auth_token: str | None = None,
        reconnect_ms: int | None = None,
    ) -> None:
        super().__init__()
        self.ws_url = (
            ws_url if ws_url is not None else os.environ.get("WS_URL", "ws://agent-hub:9000/codex")
        )
        self.auth_token = (
            auth_token if auth_token is not None else os.environ.get("AGENT_AUTH_TOKEN", "")
        )
        if reconnect_ms is None:
            reconnect_ms = int(os.environ.get("RECONNECT_MS", "3000"))
        self.reconnect_ms = reconnect_ms
        self.websocket = None

    async def send(self, msg_type: str, payload: dict) -> None:
        if not self.websocket:
            return
        await self.websocket.send(json.dumps(self._envelope(msg_type, payload)))

    async def connect_once(self) -> None:
        headers = {}
        websockets = load_websockets()
        if self.auth_token:
            headers["Authorization"] = f"Bearer {self.auth_token}"

        async with websockets.connect(
            self.ws_url,
            additional_headers=headers or None,
            subprotocols=["ironclaw-agent-v1"],
        ) as websocket:
            self.websocket = websocket
            write_ws_state(
                ready=True,
                role="client",
                connected=True,
                ws_url=self.ws_url,
            )
            print(f"[codex_agent_client] connected to {self.ws_url}", flush=True)
            await self.send_ready()

            async for message in websocket:
                await self.handle_message(message)

    async def run_forever(self) -> None:
        while True:
            try:
                await self.connect_once()
            except Exception as exc:
                write_ws_state(
                    ready=False,
                    role="client",
                    connected=False,
                    ws_url=self.ws_url,
                    error=str(exc),
                )
                print(f"[codex_agent_client] connection failed: {exc}", flush=True)
            finally:
                self.websocket = None

            await asyncio.sleep(self.reconnect_ms / 1000)


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Run the outbound worker client that connects to an IronClaw hub and "
            "waits for task_request messages."
        )
    )
    parser.add_argument(
        "--ws-url",
        help=(
            "WebSocket URL. Normal mode defaults to WS_URL or ws://agent-hub:9000/codex; "
            "prompt mode defaults to WS_URL or ws://127.0.0.1:9090/ws/agent."
        ),
    )
    parser.add_argument(
        "--auth-token",
        help="Bearer token for websocket auth. Defaults to AGENT_AUTH_TOKEN.",
    )
    parser.add_argument(
        "--reconnect-ms",
        type=int,
        help="Reconnect delay in milliseconds. Defaults to RECONNECT_MS or 3000.",
    )
    parser.add_argument(
        "--prompt",
        help=(
            "Compatibility mode: submit one task_request to an inbound worker "
            "websocket instead of running as the outbound worker client."
        ),
    )
    parser.add_argument(
        "--task-path",
        help=(
            "Task working path for compatibility prompt mode. "
            "Defaults to TASK_PATH or /workspace."
        ),
    )
    parser.add_argument(
        "--task-id",
        help="Task identifier for compatibility prompt mode. Defaults to TASK_ID or demo-task-001.",
    )
    parser.add_argument(
        "--timeout-ms",
        type=int,
        help="Task timeout for compatibility prompt mode. Defaults to TASK_TIMEOUT_MS or 300000.",
    )
    parser.add_argument("--mode", choices=["cli", "websocket"], help=argparse.SUPPRESS)
    return parser


async def main() -> None:
    parser = build_arg_parser()
    args = parser.parse_args()

    if args.prompt is not None:
        if args.mode not in (None, "cli"):
            parser.error("--prompt compatibility mode only accepts --mode cli")

        try:
            await run_agent(
                name="codex_agent_client",
                default_ws_url="ws://127.0.0.1:9090/ws/agent",
                ws_url=args.ws_url,
                auth_token=args.auth_token,
                task_prompt=args.prompt,
                task_path=args.task_path,
                task_id=args.task_id,
                timeout_ms=args.timeout_ms,
                exit_on_result=True,
            )
        except MissingWebsocketsError as exc:
            parser.exit(1, f"[codex_agent_client] {exc}\n")
        return

    if args.mode == "cli":
        parser.error(
            "--mode cli is only valid with --prompt. For one-shot Codex CLI mode, "
            "use entrypoint.sh or docker compose --profile cli."
        )

    client = CodexAgentClient(
        ws_url=args.ws_url,
        auth_token=args.auth_token,
        reconnect_ms=args.reconnect_ms,
    )
    await client.run_forever()


if __name__ == "__main__":
    asyncio.run(main())
