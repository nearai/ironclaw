#!/usr/bin/env python3
"""Minimal Agent Client Protocol stdio bridge for the IronClaw Codex worker."""

import argparse
import asyncio
import json
import os
import sys
import threading
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

from ironclaw_agent_example import MissingWebsocketsError, load_websockets


JSON = dict[str, Any]
SUPPORTED_PROTOCOL_VERSION = 1


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def log(message: str) -> None:
    print(f"[acp_bridge] {message}", file=sys.stderr, flush=True)


def worker_envelope(msg_type: str, payload: JSON) -> JSON:
    return {
        "id": str(uuid.uuid4()),
        "type": msg_type,
        "timestamp": utc_now(),
        "payload": payload,
    }


def jsonrpc_result(msg_id: Any, result: Any) -> JSON:
    return {"jsonrpc": "2.0", "id": msg_id, "result": result}


def jsonrpc_error(msg_id: Any, code: int, message: str, data: Any = None) -> JSON:
    error: JSON = {"code": code, "message": message}
    if data is not None:
        error["data"] = data
    return {"jsonrpc": "2.0", "id": msg_id, "error": error}


def session_update(session_id: str, text: str) -> JSON:
    return {
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text},
            },
        },
    }


def extract_prompt_text(prompt: Any) -> str:
    if isinstance(prompt, str):
        return prompt

    if not isinstance(prompt, list):
        return ""

    parts: list[str] = []
    for block in prompt:
        if not isinstance(block, dict):
            continue

        block_type = block.get("type")
        if block_type == "text":
            parts.append(str(block.get("text", "")))
            continue

        if block_type == "resource_link":
            uri = block.get("uri", "")
            name = block.get("name", "")
            parts.append(f"[resource_link name={name!r} uri={uri!r}]")
            continue

        if block_type == "resource":
            resource = block.get("resource") or {}
            if isinstance(resource, dict):
                uri = resource.get("uri", "")
                if "text" in resource:
                    parts.append(f"[resource uri={uri!r}]\n{resource.get('text', '')}")
                elif "blob" in resource:
                    parts.append(f"[binary resource uri={uri!r}]")

    return "\n\n".join(part for part in parts if part).strip()


@dataclass
class AcpSession:
    session_id: str
    cwd: str


@dataclass
class ActivePrompt:
    task_id: str
    websocket: Any | None = None


class AcpBridge:
    def __init__(
        self,
        *,
        ws_url: str,
        auth_token: str,
        project_dir: str,
        timeout_ms: int,
        use_session_cwd: bool,
    ) -> None:
        self.ws_url = ws_url
        self.auth_token = auth_token
        self.project_dir = project_dir
        self.timeout_ms = timeout_ms
        self.use_session_cwd = use_session_cwd
        self.sessions: dict[str, AcpSession] = {}
        self.active_prompts: dict[str, ActivePrompt] = {}
        self.write_lock = asyncio.Lock()

    async def send(self, message: JSON) -> None:
        data = json.dumps(message, separators=(",", ":"))
        async with self.write_lock:
            print(data, flush=True)

    async def handle_initialize(self, request: JSON) -> None:
        params = request.get("params") or {}
        try:
            requested_version = int(params.get("protocolVersion", 1) or 1)
        except (TypeError, ValueError):
            requested_version = SUPPORTED_PROTOCOL_VERSION
        protocol_version = min(requested_version, SUPPORTED_PROTOCOL_VERSION)
        if protocol_version < 1:
            protocol_version = SUPPORTED_PROTOCOL_VERSION
        await self.send(
            jsonrpc_result(
                request.get("id"),
                {
                    "protocolVersion": protocol_version,
                    "agentCapabilities": {
                        "loadSession": False,
                        "promptCapabilities": {
                            "image": False,
                            "audio": False,
                            "embeddedContext": True,
                        },
                        "mcpCapabilities": {
                            "http": False,
                            "sse": False,
                        },
                        "sessionCapabilities": {},
                    },
                    "agentInfo": {
                        "name": "ironclaw-codex-worker",
                        "title": "IronClaw Codex Worker ACP Bridge",
                        "version": "1.0.0",
                    },
                    "authMethods": [],
                },
            )
        )

    async def handle_session_new(self, request: JSON) -> None:
        params = request.get("params") or {}
        cwd = params.get("cwd") or self.project_dir
        if not isinstance(cwd, str) or not cwd.startswith("/"):
            await self.send(jsonrpc_error(request.get("id"), -32602, "cwd must be absolute"))
            return

        session_id = f"ironclaw-acp-{uuid.uuid4()}"
        self.sessions[session_id] = AcpSession(session_id=session_id, cwd=cwd)
        await self.send(jsonrpc_result(request.get("id"), {"sessionId": session_id}))

    async def handle_session_prompt(self, request: JSON) -> None:
        params = request.get("params") or {}
        session_id = params.get("sessionId")
        session = self.sessions.get(session_id)
        if session is None:
            await self.send(jsonrpc_error(request.get("id"), -32602, "unknown sessionId"))
            return

        prompt_text = extract_prompt_text(params.get("prompt"))
        if not prompt_text:
            await self.send(jsonrpc_error(request.get("id"), -32602, "prompt is empty"))
            return

        task_id = f"acp-{uuid.uuid4()}"
        active = ActivePrompt(task_id=task_id)
        self.active_prompts[session_id] = active

        try:
            stop_reason = await self.run_worker_task(session, active, prompt_text)
            await self.send(jsonrpc_result(request.get("id"), {"stopReason": stop_reason}))
        except asyncio.CancelledError:
            await self.send(jsonrpc_result(request.get("id"), {"stopReason": "cancelled"}))
        except Exception as exc:
            await self.send(session_update(session_id, f"Bridge error: {exc}"))
            await self.send(jsonrpc_result(request.get("id"), {"stopReason": "end_turn"}))
        finally:
            self.active_prompts.pop(session_id, None)

    async def run_worker_task(
        self,
        session: AcpSession,
        active: ActivePrompt,
        prompt_text: str,
    ) -> str:
        websockets = load_websockets()
        headers = {}
        if self.auth_token:
            headers["Authorization"] = f"Bearer {self.auth_token}"

        async with websockets.connect(
            self.ws_url,
            additional_headers=headers or None,
            subprotocols=["ironclaw-agent-v1"],
        ) as websocket:
            active.websocket = websocket
            ready_raw = await websocket.recv()
            ready = json.loads(ready_raw)
            if ready.get("type") != "ready":
                raise RuntimeError(f"unexpected worker greeting: {ready}")

            await websocket.send(
                json.dumps(
                    worker_envelope(
                        "task_request",
                        {
                            "task_id": active.task_id,
                            "prompt": prompt_text,
                            "context": {
                                "path": (
                                    session.cwd
                                    if self.use_session_cwd
                                    else self.project_dir
                                ),
                                "acp_cwd": session.cwd,
                            },
                            "timeout_ms": self.timeout_ms,
                        },
                    )
                )
            )

            async for raw_message in websocket:
                message = json.loads(raw_message)
                msg_type = message.get("type")
                payload = message.get("payload") or {}

                if msg_type == "task_progress":
                    delta = str(payload.get("delta", ""))
                    if delta:
                        await self.send(session_update(session.session_id, delta))
                    continue

                if msg_type == "task_result":
                    status = payload.get("status")
                    output = payload.get("output") or ""
                    error = payload.get("error") or ""
                    if output:
                        await self.send(session_update(session.session_id, str(output)))
                    if error:
                        await self.send(session_update(session.session_id, f"Error: {error}"))
                    if status == "cancelled":
                        return "cancelled"
                    return "end_turn"

        return "end_turn"

    async def handle_session_cancel(self, request: JSON) -> None:
        params = request.get("params") or {}
        session_id = params.get("sessionId")
        active = self.active_prompts.get(session_id)
        if active and active.websocket:
            await active.websocket.send(
                json.dumps(
                    worker_envelope(
                        "cancel",
                        {"task_id": active.task_id},
                    )
                )
            )

        if "id" in request:
            await self.send(jsonrpc_result(request.get("id"), None))

    async def handle_request(self, request: JSON) -> None:
        msg_id = request.get("id")
        method = request.get("method")
        if request.get("jsonrpc") != "2.0" or not isinstance(method, str):
            await self.send(jsonrpc_error(msg_id, -32600, "invalid JSON-RPC request"))
            return

        if method == "initialize":
            await self.handle_initialize(request)
            return

        if method == "session/new":
            await self.handle_session_new(request)
            return

        if method == "session/prompt":
            asyncio.create_task(self.handle_session_prompt(request))
            return

        if method == "session/cancel":
            await self.handle_session_cancel(request)
            return

        if method == "authenticate":
            await self.send(jsonrpc_result(msg_id, {}))
            return

        if method == "session/load":
            await self.send(jsonrpc_error(msg_id, -32601, "session/load is not supported"))
            return

        await self.send(jsonrpc_error(msg_id, -32601, f"method not found: {method}"))

    async def run_stdio(self) -> None:
        queue: asyncio.Queue[str | None] = asyncio.Queue()
        loop = asyncio.get_running_loop()
        stdin_fileno = sys.stdin.fileno()
        stdin_buffer = bytearray()
        stdin_was_blocking = os.get_blocking(stdin_fileno)
        using_reader = False

        def read_stdin() -> None:
            for line in sys.stdin:
                loop.call_soon_threadsafe(queue.put_nowait, line)
            loop.call_soon_threadsafe(queue.put_nowait, None)

        def finish_stdin() -> None:
            if using_reader:
                loop.remove_reader(stdin_fileno)
            if stdin_buffer:
                queue.put_nowait(bytes(stdin_buffer).decode("utf-8", "replace"))
                stdin_buffer.clear()
            queue.put_nowait(None)

        def read_ready() -> None:
            while True:
                try:
                    chunk = os.read(stdin_fileno, 4096)
                except BlockingIOError:
                    return

                if not chunk:
                    finish_stdin()
                    return

                stdin_buffer.extend(chunk)
                while True:
                    newline_index = stdin_buffer.find(b"\n")
                    if newline_index < 0:
                        break

                    line = bytes(stdin_buffer[:newline_index])
                    del stdin_buffer[: newline_index + 1]
                    queue.put_nowait(line.rstrip(b"\r").decode("utf-8", "replace"))

        try:
            os.set_blocking(stdin_fileno, False)
            loop.add_reader(stdin_fileno, read_ready)
            using_reader = True
        except (AttributeError, NotImplementedError):
            os.set_blocking(stdin_fileno, stdin_was_blocking)
            threading.Thread(target=read_stdin, daemon=True).start()

        try:
            while True:
                line = await queue.get()
                if line is None:
                    break

                try:
                    request = json.loads(line)
                except json.JSONDecodeError as exc:
                    await self.send(jsonrpc_error(None, -32700, f"parse error: {exc}"))
                    continue

                if not isinstance(request, dict):
                    await self.send(jsonrpc_error(None, -32600, "request must be an object"))
                    continue

                await self.handle_request(request)
        finally:
            if using_reader:
                loop.remove_reader(stdin_fileno)
                os.set_blocking(stdin_fileno, stdin_was_blocking)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a minimal ACP stdio bridge for the IronClaw Codex worker."
    )
    parser.add_argument(
        "--ws-url",
        default=os.environ.get("ACP_WORKER_WS_URL")
        or os.environ.get("WS_URL")
        or "ws://127.0.0.1:9090/ws/agent",
        help="IronClaw worker websocket URL.",
    )
    parser.add_argument(
        "--auth-token",
        default=os.environ.get("AGENT_AUTH_TOKEN", ""),
        help="Bearer token for the IronClaw worker websocket.",
    )
    parser.add_argument(
        "--project-dir",
        default=os.environ.get("ACP_PROJECT_DIR")
        or os.environ.get("WORKSPACE_ROOT")
        or "/workspace",
        help="Worker-side project directory to send as context.path.",
    )
    parser.add_argument(
        "--timeout-ms",
        type=int,
        default=int(os.environ.get("TASK_TIMEOUT_MS", "300000")),
        help="Worker task timeout in milliseconds.",
    )
    parser.add_argument(
        "--use-session-cwd",
        action="store_true",
        default=os.environ.get("ACP_USE_SESSION_CWD", "").lower()
        in {"1", "true", "yes", "on"},
        help=(
            "Send ACP session/new cwd as context.path. By default the bridge "
            "uses --project-dir because Docker workers usually see /workspace."
        ),
    )
    return parser.parse_args()


async def async_main() -> int:
    args = parse_args()
    bridge = AcpBridge(
        ws_url=args.ws_url,
        auth_token=args.auth_token,
        project_dir=args.project_dir,
        timeout_ms=args.timeout_ms,
        use_session_cwd=args.use_session_cwd,
    )
    cwd_mode = "session-cwd" if args.use_session_cwd else "project-dir"
    log(
        f"bridging ACP stdio to {args.ws_url} "
        f"project_dir={args.project_dir} cwd_mode={cwd_mode}"
    )
    try:
        await bridge.run_stdio()
    except MissingWebsocketsError as exc:
        log(str(exc))
        return 1
    return 0


def main() -> None:
    raise SystemExit(asyncio.run(async_main()))


if __name__ == "__main__":
    main()
