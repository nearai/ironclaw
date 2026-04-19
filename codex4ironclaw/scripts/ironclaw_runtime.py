#!/usr/bin/env python3
"""Shared helpers for IronClaw websocket worker roles."""

import asyncio
import json
import os
import shlex
import signal
import time
import uuid
from datetime import datetime, timezone


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def ws_state_path() -> str:
    return os.environ.get("WS_STATE_FILE", "/tmp/ironclaw_ws_state.json")


def write_ws_state(**state: object) -> None:
    """Persist websocket state for the health endpoint."""
    path = ws_state_path()
    directory = os.path.dirname(path)
    if directory:
        os.makedirs(directory, exist_ok=True)

    payload = {"timestamp": utc_now(), **state}
    temp_path = f"{path}.tmp"
    with open(temp_path, "w", encoding="utf-8") as handle:
        json.dump(payload, handle)
    os.replace(temp_path, path)


def truthy_env(name: str, default: str = "false") -> bool:
    return os.environ.get(name, default).strip().lower() in {
        "1",
        "true",
        "yes",
        "on",
    }


class CodexTaskEndpoint:
    """Shared task execution logic used by websocket client and server roles."""

    def __init__(self) -> None:
        self.worker_id = os.environ.get("IRONCLAW_WORKER_ID", "worker-codex-01")
        self.version = os.environ.get("CODEX_VERSION", "unknown")
        self.mode = os.environ.get("CODEX_MODE", "websocket")
        self.codex_bin = os.environ.get("CODEX_BIN", "/app/node_modules/.bin/codex")
        self.workspace_root = os.environ.get("WORKSPACE_ROOT", "/workspace")
        self.bypass_codex_sandbox = truthy_env("CODEX_BYPASS_SANDBOX", "true")
        self.running_tasks: dict[str, asyncio.subprocess.Process] = {}

    def _envelope(self, msg_type: str, payload: dict) -> dict:
        return {
            "id": str(uuid.uuid4()),
            "type": msg_type,
            "timestamp": utc_now(),
            "payload": payload,
        }

    async def send(self, msg_type: str, payload: dict) -> None:
        raise NotImplementedError

    async def send_ready(self) -> None:
        await self.send(
            "ready",
            {
                "worker_id": self.worker_id,
                "version": self.version,
                "mode": self.mode,
            },
        )

    async def send_error(self, task_id: str, error: str) -> None:
        await self.send(
            "task_result",
            {
                "task_id": task_id,
                "status": "error",
                "output": "",
                "error": error,
                "duration_ms": 0,
            },
        )

    async def send_progress(self, task_id: str, delta: str, done: bool = False) -> None:
        await self.send(
            "task_progress",
            {
                "task_id": task_id,
                "delta": delta,
                "done": done,
            },
        )

    def _resolve_cwd(self, requested_path: str | None) -> str:
        if not requested_path:
            return self.workspace_root

        candidate = requested_path
        if not os.path.isabs(candidate):
            candidate = os.path.join(self.workspace_root, candidate)

        if os.path.isdir(candidate):
            return os.path.abspath(candidate)

        return self.workspace_root

    async def run_task_request(self, payload: dict) -> None:
        task_id = payload.get("task_id", "")
        prompt = payload.get("prompt", "").strip()
        timeout_ms = int(payload.get("timeout_ms", 300000) or 300000)
        context = payload.get("context") or {}
        requested_path = context.get("path") if isinstance(context, dict) else None
        cwd = self._resolve_cwd(requested_path)

        if not prompt:
            await self.send_error(task_id, "Missing prompt in task_request payload.")
            return

        cmd = [
            self.codex_bin,
            "exec",
        ]
        if self.bypass_codex_sandbox:
            cmd.append("--dangerously-bypass-approvals-and-sandbox")
        else:
            cmd.append("--full-auto")
        cmd.extend(
            [
                "--skip-git-repo-check",
                "--cd",
                cwd,
                prompt,
            ]
        )

        await self.send_progress(task_id, f"Running: {' '.join(shlex.quote(part) for part in cmd)}")

        started = time.monotonic()
        process = await asyncio.create_subprocess_exec(
            *cmd,
            cwd=cwd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        self.running_tasks[task_id] = process

        try:
            stdout, stderr = await asyncio.wait_for(process.communicate(), timeout_ms / 1000)
        except asyncio.TimeoutError:
            process.send_signal(signal.SIGTERM)
            await process.wait()
            self.running_tasks.pop(task_id, None)
            await self.send_error(task_id, f"Task timed out after {timeout_ms} ms.")
            return

        self.running_tasks.pop(task_id, None)
        duration_ms = int((time.monotonic() - started) * 1000)
        stdout_text = stdout.decode("utf-8", errors="replace").strip()
        stderr_text = stderr.decode("utf-8", errors="replace").strip()
        output = stdout_text or stderr_text

        if process.returncode == 0:
            await self.send_progress(task_id, "Codex task completed.", done=True)
            await self.send(
                "task_result",
                {
                    "task_id": task_id,
                    "status": "success",
                    "output": output,
                    "error": None,
                    "duration_ms": duration_ms,
                },
            )
            return

        error = stderr_text or stdout_text or f"codex exited with status {process.returncode}"
        await self.send(
            "task_result",
            {
                "task_id": task_id,
                "status": "error",
                "output": stdout_text,
                "error": error,
                "duration_ms": duration_ms,
            },
        )

    async def handle_message(self, raw_message: str) -> None:
        try:
            envelope = json.loads(raw_message)
        except json.JSONDecodeError:
            print("[ironclaw_runtime] ignoring malformed JSON message", flush=True)
            return

        msg_type = envelope.get("type")
        payload = envelope.get("payload", {})

        if msg_type == "ping":
            await self.send("pong", {})
            return

        if msg_type == "task_request":
            asyncio.create_task(self.run_task_request(payload))
            return

        if msg_type == "cancel":
            task_id = payload.get("task_id", "")
            process = self.running_tasks.get(task_id)
            if process and process.returncode is None:
                process.send_signal(signal.SIGTERM)
                await process.wait()
                self.running_tasks.pop(task_id, None)
                await self.send(
                    "task_result",
                    {
                        "task_id": task_id,
                        "status": "cancelled",
                        "output": "",
                        "error": None,
                        "duration_ms": 0,
                    },
                )
            else:
                await self.send_error(task_id, "Cancellation acknowledged, but no active task was found.")
            return

        print(f"[ironclaw_runtime] ignoring unsupported message type: {msg_type}", flush=True)
