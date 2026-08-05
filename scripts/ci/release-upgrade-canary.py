#!/usr/bin/env python3
"""Verify a previous IronClaw release artifact can upgrade to an exact candidate."""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import http.server
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import tarfile
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any

AUTH_TOKEN = "ironclaw-release-upgrade-canary-token"
USER_ID = "release-upgrade-canary-user"
TENANT_ID = "release-upgrade-canary"
AGENT_ID = "release-upgrade-canary-agent"
MODEL_NAME = "release-upgrade-canary-model"
WORKSPACE_FILE = "upgrade-canary.txt"
WORKSPACE_BYTES = b"ironclaw release upgrade canary workspace\n"
PROMPTS = (
    "release upgrade canary create scheduled routine",
    "release upgrade canary create paused routine",
)
ROUTINE_DEFINITIONS = {
    PROMPTS[0]: {
        "name": "release-upgrade-canary-scheduled",
        "prompt": "release upgrade canary scheduled routine action",
        "schedule": {
            "kind": "cron",
            "expression": "0 0 1 1 *",
            "timezone": "UTC",
        },
    },
    PROMPTS[1]: {
        "name": "release-upgrade-canary-paused",
        "prompt": "release upgrade canary paused routine action",
        "schedule": {
            "kind": "cron",
            "expression": "0 0 2 1 *",
            "timezone": "UTC",
        },
    },
}
PAUSED_ROUTINE_NAME = ROUTINE_DEFINITIONS[PROMPTS[1]]["name"]
MESSAGE_FIELDS = (
    "message_id",
    "kind",
    "content",
    "sequence",
    "status",
    "turn_run_id",
)
MAX_ARCHIVE_BINARY_BYTES = 1024 * 1024 * 1024
_PASSTHROUGH_ENV = ("PATH", "LANG", "LC_ALL", "TZ")


class CanaryFailure(RuntimeError):
    """The release artifacts did not satisfy the upgrade contract."""


@dataclass(frozen=True)
class ProductSnapshot:
    thread_ids: tuple[str, ...]
    timelines: dict[str, tuple[dict[str, Any], ...]]

    @property
    def message_count(self) -> int:
        return sum(len(messages) for messages in self.timelines.values())


@dataclass(frozen=True)
class RoutineSnapshot:
    automations: tuple[dict[str, Any], ...]


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def verify_checksum(archive: Path, checksum_file: Path) -> None:
    try:
        fields = checksum_file.read_text(encoding="utf-8").strip().split()
    except OSError as error:
        raise CanaryFailure(f"could not read checksum file {checksum_file}: {error}") from error
    if len(fields) != 2 or not re.fullmatch(r"[0-9a-fA-F]{64}", fields[0]):
        raise CanaryFailure(f"invalid SHA-256 checksum file: {checksum_file}")
    if Path(fields[1].lstrip("*")).name != archive.name:
        raise CanaryFailure(
            f"checksum {checksum_file.name} names {fields[1]!r}, expected {archive.name!r}"
        )
    actual = _sha256(archive)
    if actual.lower() != fields[0].lower():
        raise CanaryFailure(
            f"SHA-256 mismatch for {archive.name}: expected {fields[0].lower()}, got {actual}"
        )


def extract_binary(archive: Path, destination: Path, binary_name: str) -> Path:
    if Path(binary_name).name != binary_name or binary_name in {"", ".", ".."}:
        raise CanaryFailure(f"invalid release binary name: {binary_name!r}")
    try:
        with tarfile.open(archive, mode="r:gz") as package:
            matches = [
                member
                for member in package.getmembers()
                if member.isfile() and Path(member.name).name == binary_name
            ]
            if len(matches) != 1:
                raise CanaryFailure(
                    f"{archive.name} must contain exactly one {binary_name}; found {len(matches)}"
                )
            member = matches[0]
            if member.size <= 0 or member.size > MAX_ARCHIVE_BINARY_BYTES:
                raise CanaryFailure(
                    f"{archive.name} contains an invalid {binary_name} size: {member.size}"
                )
            source = package.extractfile(member)
            if source is None:
                raise CanaryFailure(f"could not read {binary_name} from {archive.name}")
            destination.mkdir(parents=True, exist_ok=True)
            extracted = destination / binary_name
            with extracted.open("wb") as output:
                shutil.copyfileobj(source, output)
            extracted.chmod(0o755)
            return extracted
    except (OSError, tarfile.TarError) as error:
        raise CanaryFailure(f"could not extract {archive}: {error}") from error


def _read_version(binary: Path) -> str:
    try:
        result = subprocess.run(
            [str(binary), "--version"],
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise CanaryFailure(f"could not execute {binary.name} --version: {error}") from error
    if result.returncode != 0:
        raise CanaryFailure(
            f"{binary.name} --version exited {result.returncode}: {result.stderr[-2000:]}"
        )
    match = re.search(r"\bironclaw\s+([^\s]+)", result.stdout, re.IGNORECASE)
    if match is None:
        raise CanaryFailure(f"could not parse IronClaw version from {result.stdout!r}")
    return match.group(1)


def _assert_version(binary: Path, expected: str, label: str) -> None:
    actual = _read_version(binary)
    if actual != expected:
        raise CanaryFailure(f"{label} artifact version is {actual!r}, expected {expected!r}")


def _json_response(handler: http.server.BaseHTTPRequestHandler, payload: object) -> None:
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    handler.send_response(200)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)


class _MockLlmHandler(http.server.BaseHTTPRequestHandler):
    server_version = "IronClawReleaseUpgradeMock/1"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler contract
        if self.path.rstrip("/") in {"/v1/models", "/models"}:
            _json_response(
                self,
                {"object": "list", "data": [{"id": MODEL_NAME, "object": "model"}]},
            )
            return
        self.send_error(404)

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler contract
        if self.path.rstrip("/") not in {"/v1/chat/completions", "/chat/completions"}:
            self.send_error(404)
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length <= 0 or length > 1024 * 1024:
                raise ValueError("request body is empty or too large")
            request = json.loads(self.rfile.read(length))
        except (ValueError, json.JSONDecodeError):
            self.send_error(400)
            return
        routine = _requested_routine(request)
        completion_id = f"chatcmpl-{uuid.uuid4().hex[:12]}"
        if routine is not None:
            tool_call = {
                "index": 0,
                "id": f"call-{uuid.uuid4().hex[:12]}",
                "type": "function",
                "function": {
                    "name": "builtin__trigger_create",
                    "arguments": json.dumps(routine, separators=(",", ":")),
                },
            }
            if not request.get("stream"):
                _json_response(
                    self,
                    {
                        "id": completion_id,
                        "object": "chat.completion",
                        "created": 0,
                        "model": MODEL_NAME,
                        "choices": [
                            {
                                "index": 0,
                                "message": {
                                    "role": "assistant",
                                    "content": None,
                                    "tool_calls": [tool_call],
                                },
                                "finish_reason": "tool_calls",
                            }
                        ],
                        "usage": {
                            "prompt_tokens": 1,
                            "completion_tokens": 1,
                            "total_tokens": 2,
                        },
                    },
                )
                return
            chunks = (
                {
                    "id": completion_id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": MODEL_NAME,
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"role": "assistant", "tool_calls": [tool_call]},
                            "finish_reason": None,
                        }
                    ],
                },
                {
                    "id": completion_id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": MODEL_NAME,
                    "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}],
                },
            )
            _write_sse(self, chunks)
            return

        text = "release upgrade canary deterministic reply"
        if not request.get("stream"):
            _json_response(
                self,
                {
                    "id": completion_id,
                    "object": "chat.completion",
                    "created": 0,
                    "model": MODEL_NAME,
                    "choices": [
                        {
                            "index": 0,
                            "message": {"role": "assistant", "content": text},
                            "finish_reason": "stop",
                        }
                    ],
                    "usage": {
                        "prompt_tokens": 1,
                        "completion_tokens": 1,
                        "total_tokens": 2,
                    },
                },
            )
            return
        chunks = (
            {
                "id": completion_id,
                "object": "chat.completion.chunk",
                "created": 0,
                "model": MODEL_NAME,
                "choices": [
                    {
                        "index": 0,
                        "delta": {"role": "assistant", "content": text},
                        "finish_reason": None,
                    }
                ],
            },
            {
                "id": completion_id,
                "object": "chat.completion.chunk",
                "created": 0,
                "model": MODEL_NAME,
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            },
        )
        _write_sse(self, chunks)


def _requested_routine(request: dict[str, Any]) -> dict[str, Any] | None:
    messages = request.get("messages")
    if not isinstance(messages, list):
        return None
    if any(isinstance(message, dict) and message.get("role") == "tool" for message in messages):
        return None
    for message in reversed(messages):
        if not isinstance(message, dict) or message.get("role") != "user":
            continue
        content = message.get("content")
        if isinstance(content, str) and content in ROUTINE_DEFINITIONS:
            return ROUTINE_DEFINITIONS[content]
    return None


def _write_sse(handler: http.server.BaseHTTPRequestHandler, chunks: tuple[dict[str, Any], ...]) -> None:
    body = b"".join(
        f"data: {json.dumps(chunk, separators=(',', ':'))}\n\n".encode("utf-8")
        for chunk in chunks
    ) + b"data: [DONE]\n\n"
    handler.send_response(200)
    handler.send_header("Content-Type", "text/event-stream")
    handler.send_header("Cache-Control", "no-cache")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)


@contextlib.contextmanager
def mock_llm_server():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _MockLlmHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = server.server_address
        yield f"http://{host}:{port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def _request(
    method: str,
    url: str,
    *,
    payload: object | None = None,
    authenticated: bool = True,
    timeout: float = 15,
) -> tuple[bytes, dict[str, str]]:
    data = None
    headers: dict[str, str] = {}
    if authenticated:
        headers["Authorization"] = f"Bearer {AUTH_TOKEN}"
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.read(), {key.lower(): value for key, value in response.headers.items()}
    except urllib.error.HTTPError as error:
        body = error.read(4000).decode("utf-8", errors="replace")
        raise CanaryFailure(f"{method} {url} returned HTTP {error.code}: {body}") from error
    except (OSError, urllib.error.URLError) as error:
        raise CanaryFailure(f"{method} {url} failed: {error}") from error


def _request_json(method: str, url: str, *, payload: object | None = None) -> dict[str, Any]:
    body, _ = _request(method, url, payload=payload)
    try:
        value = json.loads(body)
    except json.JSONDecodeError as error:
        raise CanaryFailure(f"{method} {url} returned invalid JSON: {error}") from error
    if not isinstance(value, dict):
        raise CanaryFailure(f"{method} {url} returned a non-object JSON value")
    return value


def _wait_until_ready(base_url: str, process: subprocess.Popen[bytes], timeout: float = 90) -> None:
    deadline = time.monotonic() + timeout
    health_url = f"{base_url}/api/health"
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise CanaryFailure(f"IronClaw exited during startup with code {process.returncode}")
        try:
            _request("GET", health_url, authenticated=False, timeout=3)
            return
        except CanaryFailure:
            time.sleep(0.25)
    raise CanaryFailure(f"IronClaw did not become ready at {health_url} within {timeout}s")


class IronclawServer:
    def __init__(
        self,
        *,
        binary: Path,
        root: Path,
        cwd: Path,
        mock_llm_url: str,
        label: str,
        artifact_dir: Path,
        workspace_root: Path | None = None,
        legacy_workspace_snapshot: Path | None = None,
    ) -> None:
        self.binary = binary
        self.root = root
        self.cwd = cwd
        self.mock_llm_url = mock_llm_url
        self.label = label
        self.artifact_dir = artifact_dir
        self.workspace_root = workspace_root
        self.legacy_workspace_snapshot = legacy_workspace_snapshot
        self.process: subprocess.Popen[bytes] | None = None
        self.base_url = ""
        self._stdout = None
        self._stderr = None

    def start(self) -> str:
        if self.process is not None:
            raise CanaryFailure(f"{self.label} server is already started")
        port = _free_port()
        self.base_url = f"http://127.0.0.1:{port}"
        environment = {
            key: value for key in _PASSTHROUGH_ENV if (value := os.environ.get(key))
        }
        environment.update(
            {
                "HOME": str(self.root / "home"),
                "IRONCLAW_REBORN_HOME": str(self.root / "reborn-home"),
                "IRONCLAW_REBORN_PROFILE": "local-dev",
                "IRONCLAW_REBORN_WEBUI_TOKEN": AUTH_TOKEN,
                "IRONCLAW_REBORN_WEBUI_USER_ID": USER_ID,
                "IRONCLAW_DISABLE_OS_KEYCHAIN": "1",
                "MOCK_LLM_API_KEY": "release-upgrade-canary-key",
                "NO_PROXY": "127.0.0.1,localhost,::1",
                "no_proxy": "127.0.0.1,localhost,::1",
                "RUST_BACKTRACE": "1",
                "RUST_LOG": "ironclaw=info",
                "TZ": "UTC",
            }
        )
        if self.workspace_root is not None:
            environment["IRONCLAW_REBORN_WORKSPACE_ROOT"] = str(self.workspace_root)
        if self.legacy_workspace_snapshot is not None:
            environment["IRONCLAW_REBORN_LEGACY_WORKSPACE_SNAPSHOT"] = str(
                self.legacy_workspace_snapshot
            )
        self.artifact_dir.mkdir(parents=True, exist_ok=True)
        self._stdout = (self.artifact_dir / f"{self.label}.stdout.log").open("wb")
        self._stderr = (self.artifact_dir / f"{self.label}.stderr.log").open("wb")
        try:
            self.process = subprocess.Popen(
                [
                    str(self.binary),
                    "serve",
                    "--host",
                    "127.0.0.1",
                    "--port",
                    str(port),
                ],
                cwd=self.cwd,
                env=environment,
                stdin=subprocess.DEVNULL,
                stdout=self._stdout,
                stderr=self._stderr,
            )
            _wait_until_ready(self.base_url, self.process)
            return self.base_url
        except BaseException:
            self.stop()
            raise

    def stop(self) -> None:
        process = self.process
        if process is not None and process.poll() is None:
            try:
                process.send_signal(signal.SIGINT)
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        self.process = None
        for stream in (self._stdout, self._stderr):
            if stream is not None:
                stream.close()
        self._stdout = None
        self._stderr = None

    def __enter__(self) -> "IronclawServer":
        self.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self.stop()


def _write_config(root: Path, mock_llm_url: str) -> None:
    reborn_home = root / "reborn-home"
    reborn_home.mkdir(parents=True, exist_ok=True)
    config = f'''api_version = "ironclaw.runtime/v1"

[boot]
profile = "local-dev"

[identity]
default_owner = "{USER_ID}"
tenant = "{TENANT_ID}"
default_agent = "{AGENT_ID}"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "{MODEL_NAME}"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "{mock_llm_url}/v1"
'''
    (reborn_home / "config.toml").write_text(config, encoding="utf-8")


def _create_thread(base_url: str) -> str:
    response = _request_json(
        "POST",
        f"{base_url}/api/webchat/v2/threads",
        payload={"client_action_id": str(uuid.uuid4())},
    )
    try:
        thread_id = response["thread"]["thread_id"]
    except (KeyError, TypeError) as error:
        raise CanaryFailure(f"create-thread response omitted thread_id: {response}") from error
    if not isinstance(thread_id, str) or not thread_id:
        raise CanaryFailure(f"create-thread response had invalid thread_id: {response}")
    return thread_id


def _send_message(base_url: str, thread_id: str, content: str) -> None:
    response = _request_json(
        "POST",
        f"{base_url}/api/webchat/v2/threads/{urllib.parse.quote(thread_id, safe='')}/messages",
        payload={"client_action_id": str(uuid.uuid4()), "content": content},
    )
    if response.get("outcome") not in {"submitted", "already_submitted"}:
        raise CanaryFailure(f"message was not accepted for {thread_id}: {response}")


def _timeline(base_url: str, thread_id: str) -> list[dict[str, Any]]:
    response = _request_json(
        "GET",
        f"{base_url}/api/webchat/v2/threads/{urllib.parse.quote(thread_id, safe='')}/timeline",
    )
    messages = response.get("messages")
    if not isinstance(messages, list) or not all(isinstance(item, dict) for item in messages):
        raise CanaryFailure(f"timeline for {thread_id} had invalid messages: {response}")
    return messages


def _wait_for_complete_timeline(base_url: str, thread_id: str, timeout: float = 60) -> None:
    deadline = time.monotonic() + timeout
    last_messages: list[dict[str, Any]] = []
    while time.monotonic() < deadline:
        last_messages = _timeline(base_url, thread_id)
        if any(
            message.get("kind") == "assistant"
            and message.get("status") == "finalized"
            and str(message.get("content") or "").strip()
            for message in last_messages
        ):
            return
        time.sleep(0.25)
    raise CanaryFailure(
        f"thread {thread_id} did not produce a finalized assistant message: {last_messages}"
    )


def capture_snapshot(base_url: str) -> ProductSnapshot:
    response = _request_json("GET", f"{base_url}/api/webchat/v2/threads")
    threads = response.get("threads")
    if not isinstance(threads, list):
        raise CanaryFailure(f"thread-list response had invalid threads: {response}")
    thread_ids: list[str] = []
    for thread in threads:
        if not isinstance(thread, dict) or not isinstance(thread.get("thread_id"), str):
            raise CanaryFailure(f"thread-list response contained an invalid row: {thread!r}")
        thread_ids.append(thread["thread_id"])
    ordered_ids = tuple(sorted(thread_ids))
    timelines: dict[str, tuple[dict[str, Any], ...]] = {}
    for thread_id in ordered_ids:
        projected = []
        for message in _timeline(base_url, thread_id):
            projected.append({field: message.get(field) for field in MESSAGE_FIELDS})
        timelines[thread_id] = tuple(projected)
    return ProductSnapshot(thread_ids=ordered_ids, timelines=timelines)


def _assert_snapshot(actual: ProductSnapshot, expected: ProductSnapshot, label: str) -> None:
    if actual != expected:
        raise CanaryFailure(
            f"{label} changed persisted thread state\n"
            f"expected={json.dumps(expected.__dict__, sort_keys=True, default=list)}\n"
            f"actual={json.dumps(actual.__dict__, sort_keys=True, default=list)}"
        )


def capture_routine_snapshot(base_url: str) -> RoutineSnapshot:
    response = _request_json("GET", f"{base_url}/api/webchat/v2/automations")
    automations = response.get("automations")
    if not isinstance(automations, list):
        raise CanaryFailure(f"automation-list response had invalid automations: {response}")
    expected_names = {definition["name"] for definition in ROUTINE_DEFINITIONS.values()}
    projected: list[dict[str, Any]] = []
    for automation in automations:
        if not isinstance(automation, dict) or automation.get("name") not in expected_names:
            continue
        projected.append(
            {
                field: automation.get(field)
                for field in (
                    "automation_id",
                    "name",
                    "source",
                    "state",
                    "next_run_at",
                    "created_at",
                )
            }
        )
    projected.sort(key=lambda automation: str(automation["name"]))
    if len(projected) != len(ROUTINE_DEFINITIONS):
        raise CanaryFailure(
            "automation-list response omitted the seeded release routines: "
            f"expected={sorted(expected_names)} actual={projected}"
        )
    return RoutineSnapshot(automations=tuple(projected))


def _wait_for_routines(base_url: str, timeout: float = 30) -> RoutineSnapshot:
    deadline = time.monotonic() + timeout
    last_error: CanaryFailure | None = None
    while time.monotonic() < deadline:
        try:
            return capture_routine_snapshot(base_url)
        except CanaryFailure as error:
            last_error = error
            time.sleep(0.25)
    raise CanaryFailure(f"seeded routines did not become visible: {last_error}")


def _pause_routine(base_url: str, snapshot: RoutineSnapshot) -> RoutineSnapshot:
    paused = next(
        (
            automation
            for automation in snapshot.automations
            if automation["name"] == PAUSED_ROUTINE_NAME
        ),
        None,
    )
    if paused is None or not isinstance(paused.get("automation_id"), str):
        raise CanaryFailure(f"could not identify routine to pause: {snapshot}")
    automation_id = urllib.parse.quote(paused["automation_id"], safe="")
    _request_json(
        "POST",
        f"{base_url}/api/webchat/v2/automations/{automation_id}/pause",
    )
    updated = _wait_for_routines(base_url)
    states = {automation["name"]: automation["state"] for automation in updated.automations}
    if states != {
        "release-upgrade-canary-paused": "paused",
        "release-upgrade-canary-scheduled": "scheduled",
    }:
        raise CanaryFailure(f"seeded routines did not preserve scheduled/paused states: {states}")
    return updated


def _assert_routines(actual: RoutineSnapshot, expected: RoutineSnapshot, label: str) -> None:
    if actual != expected:
        raise CanaryFailure(
            f"{label} changed persisted routine state\n"
            f"expected={json.dumps(expected.automations, sort_keys=True)}\n"
            f"actual={json.dumps(actual.automations, sort_keys=True)}"
        )


def _assert_workspace(base_url: str) -> None:
    query = urllib.parse.urlencode({"mount": "workspace", "path": WORKSPACE_FILE})
    body, _ = _request("GET", f"{base_url}/api/webchat/v2/fs/content?{query}")
    if body != WORKSPACE_BYTES:
        raise CanaryFailure(
            f"migrated workspace content differed: expected {len(WORKSPACE_BYTES)} bytes, "
            f"got {len(body)}"
        )


def _server(
    *,
    binary: Path,
    root: Path,
    cwd: Path,
    mock_llm_url: str,
    label: str,
    artifact_dir: Path,
    candidate: bool,
) -> IronclawServer:
    return IronclawServer(
        binary=binary,
        root=root,
        cwd=cwd,
        mock_llm_url=mock_llm_url,
        label=label,
        artifact_dir=artifact_dir,
        workspace_root=(root / "candidate-workspace" if candidate else None),
        legacy_workspace_snapshot=(root / "legacy-workspace" if candidate else None),
    )


def run_upgrade_canary(
    *,
    previous_archive: Path,
    previous_checksum: Path,
    previous_version: str,
    candidate_archive: Path,
    candidate_checksum: Path,
    candidate_version: str,
    binary_name: str,
    artifact_dir: Path,
) -> set[str]:
    previous_archive = previous_archive.resolve()
    previous_checksum = previous_checksum.resolve()
    candidate_archive = candidate_archive.resolve()
    candidate_checksum = candidate_checksum.resolve()
    artifact_dir = artifact_dir.resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    evidence: set[str] = set()
    try:
        verify_checksum(previous_archive, previous_checksum)
        verify_checksum(candidate_archive, candidate_checksum)
        evidence.add("checksums")
        with tempfile.TemporaryDirectory(prefix="ironclaw-release-upgrade-") as temporary:
            root = Path(temporary)
            for directory in (
                root / "home",
                root / "legacy-workspace",
                root / "candidate-workspace",
            ):
                directory.mkdir(parents=True)
            (root / "legacy-workspace" / WORKSPACE_FILE).write_bytes(WORKSPACE_BYTES)
            previous_binary = extract_binary(
                previous_archive, root / "previous", binary_name
            )
            candidate_binary = extract_binary(
                candidate_archive, root / "candidate", binary_name
            )
            _assert_version(previous_binary, previous_version, "previous")
            _assert_version(candidate_binary, candidate_version, "candidate")
            evidence.add("artifact_versions")

            with mock_llm_server() as mock_llm_url:
                _write_config(root, mock_llm_url)
                with _server(
                    binary=previous_binary,
                    root=root,
                    cwd=root / "legacy-workspace",
                    mock_llm_url=mock_llm_url,
                    label="previous-seed",
                    artifact_dir=artifact_dir,
                    candidate=False,
                ) as previous:
                    for prompt in PROMPTS:
                        thread_id = _create_thread(previous.base_url)
                        _send_message(previous.base_url, thread_id, prompt)
                        _wait_for_complete_timeline(previous.base_url, thread_id)
                    baseline = capture_snapshot(previous.base_url)
                    routine_baseline = _pause_routine(
                        previous.base_url, _wait_for_routines(previous.base_url)
                    )
                if len(baseline.thread_ids) != len(PROMPTS) or baseline.message_count < 4:
                    raise CanaryFailure(
                        "previous artifact did not create the required two-thread timeline baseline: "
                        f"threads={len(baseline.thread_ids)} messages={baseline.message_count}"
                    )
                evidence.add("previous_release_state")
                evidence.add("routine_state")

                with _server(
                    binary=candidate_binary,
                    root=root,
                    cwd=root / "candidate-workspace",
                    mock_llm_url=mock_llm_url,
                    label="candidate-upgrade",
                    artifact_dir=artifact_dir,
                    candidate=True,
                ) as candidate:
                    _assert_snapshot(
                        capture_snapshot(candidate.base_url), baseline, "first candidate boot"
                    )
                    _assert_routines(
                        capture_routine_snapshot(candidate.base_url),
                        routine_baseline,
                        "first candidate boot",
                    )
                    _assert_workspace(candidate.base_url)
                evidence.add("upgrade")

                with _server(
                    binary=candidate_binary,
                    root=root,
                    cwd=root / "candidate-workspace",
                    mock_llm_url=mock_llm_url,
                    label="candidate-restart",
                    artifact_dir=artifact_dir,
                    candidate=True,
                ) as candidate:
                    _assert_snapshot(
                        capture_snapshot(candidate.base_url), baseline, "candidate restart"
                    )
                    _assert_routines(
                        capture_routine_snapshot(candidate.base_url),
                        routine_baseline,
                        "candidate restart",
                    )
                    _assert_workspace(candidate.base_url)
                evidence.add("restart_idempotence")

                source_file = root / "legacy-workspace" / WORKSPACE_FILE
                if source_file.read_bytes() != WORKSPACE_BYTES:
                    raise CanaryFailure("candidate upgrade modified the retained rc1 workspace source")
                with _server(
                    binary=previous_binary,
                    root=root,
                    cwd=root / "legacy-workspace",
                    mock_llm_url=mock_llm_url,
                    label="previous-rollback",
                    artifact_dir=artifact_dir,
                    candidate=False,
                ) as previous:
                    _assert_snapshot(
                        capture_snapshot(previous.base_url), baseline, "previous-release rollback"
                    )
                    _assert_routines(
                        capture_routine_snapshot(previous.base_url),
                        routine_baseline,
                        "previous-release rollback",
                    )
                evidence.add("rollback")

                with _server(
                    binary=candidate_binary,
                    root=root,
                    cwd=root / "candidate-workspace",
                    mock_llm_url=mock_llm_url,
                    label="candidate-reupgrade",
                    artifact_dir=artifact_dir,
                    candidate=True,
                ) as candidate:
                    _assert_snapshot(
                        capture_snapshot(candidate.base_url), baseline, "candidate re-upgrade"
                    )
                    _assert_routines(
                        capture_routine_snapshot(candidate.base_url),
                        routine_baseline,
                        "candidate re-upgrade",
                    )
                    _assert_workspace(candidate.base_url)
                evidence.add("reupgrade")
    except BaseException as error:
        (artifact_dir / "result.json").write_text(
            json.dumps({"status": "failed", "error_type": type(error).__name__}) + "\n",
            encoding="utf-8",
        )
        raise

    (artifact_dir / "result.json").write_text(
        json.dumps(
            {
                "status": "passed",
                "evidence": sorted(evidence),
                "threads": len(PROMPTS),
                "messages": baseline.message_count,
                "routines": len(ROUTINE_DEFINITIONS),
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--previous-archive", type=Path, required=True)
    parser.add_argument("--previous-checksum", type=Path, required=True)
    parser.add_argument("--previous-version", required=True)
    parser.add_argument("--candidate-archive", type=Path, required=True)
    parser.add_argument("--candidate-checksum", type=Path, required=True)
    parser.add_argument("--candidate-version", required=True)
    parser.add_argument("--binary-name", default="ironclaw")
    parser.add_argument("--artifact-dir", type=Path, required=True)
    args = parser.parse_args()
    evidence = run_upgrade_canary(
        previous_archive=args.previous_archive,
        previous_checksum=args.previous_checksum,
        previous_version=args.previous_version,
        candidate_archive=args.candidate_archive,
        candidate_checksum=args.candidate_checksum,
        candidate_version=args.candidate_version,
        binary_name=args.binary_name,
        artifact_dir=args.artifact_dir,
    )
    print("release upgrade canary passed: " + ", ".join(sorted(evidence)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
