#!/usr/bin/env python3
"""Shared-instance benchmark harness for IronClaw issue #1775.

Runs one real IronClaw gateway instance against the existing mock OpenAI-
compatible LLM server, provisions multiple DB-backed users, opens multiple SSE
connections per user, and sends concurrent chat requests to stress IronClaw
itself rather than external inference services.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
import shlex
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import aiohttp
import httpx


DEFAULT_ADMIN_TOKEN = "bench-admin-token"
DEFAULT_OWNER_ID = "bench-owner"
DEFAULT_MOCK_MODEL = "mock-model"
DEFAULT_RESULTS_DIR = Path(tempfile.gettempdir()) / "ironclaw-benchmarks"
GATEWAY_CONNECTION_LIMIT = 100


@dataclass
class RuntimeSample:
    timestamp: float
    cpu_percent: float
    rss_mb: float


@dataclass
class RequestOutcome:
    user_id: str
    thread_id: str
    sequence: int
    status: str
    send_accepted: bool
    request_post_latency_ms: float | None
    time_to_first_event_ms: float | None
    time_to_final_event_ms: float | None
    connections_with_any_event: int
    connections_with_final_event: int
    all_connections_saw_any_event: bool
    all_connections_saw_final_event: bool
    final_event_type: str | None
    error_message: str | None


@dataclass
class ConnectionObservation:
    saw_any_event: bool = False
    saw_final_event: bool = False


@dataclass
class RequestTracker:
    user_id: str
    thread_id: str
    sequence: int
    connection_count: int
    send_started_at: float
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)
    final_event: asyncio.Event = field(default_factory=asyncio.Event)
    post_completed_at: float | None = None
    first_event_at: float | None = None
    final_event_at: float | None = None
    final_event_type: str | None = None
    error_message: str | None = None
    send_accepted: bool = False
    connection_observations: dict[int, ConnectionObservation] = field(init=False)

    def __post_init__(self) -> None:
        self.connection_observations = {
            idx: ConnectionObservation() for idx in range(self.connection_count)
        }

    async def mark_post_result(self, accepted: bool, completed_at: float, error: str | None) -> None:
        async with self.lock:
            self.send_accepted = accepted
            self.post_completed_at = completed_at
            if error:
                self.error_message = error
                self.final_event_type = "post_error"
                self.final_event_at = completed_at
                self.final_event.set()

    async def record_event(self, connection_index: int, event: dict[str, Any], timestamp: float) -> None:
        event_type = event.get("type")
        if event_type == "heartbeat":
            return

        async with self.lock:
            observation = self.connection_observations[connection_index]
            observation.saw_any_event = True

            if self.first_event_at is None:
                self.first_event_at = timestamp

            if event_type in {"response", "error"}:
                observation.saw_final_event = True
                if self.final_event_at is None:
                    self.final_event_at = timestamp
                    self.final_event_type = str(event_type)
                    if event_type == "error":
                        self.error_message = event.get("message")
                    self.final_event.set()

    async def finalize_timeout(self, message: str) -> None:
        async with self.lock:
            if self.final_event_at is None:
                now = time.monotonic()
                self.final_event_at = now
                self.final_event_type = "timeout"
                self.error_message = message
                self.final_event.set()

    async def snapshot(self) -> RequestOutcome:
        async with self.lock:
            any_event_count = sum(
                1 for obs in self.connection_observations.values() if obs.saw_any_event
            )
            final_event_count = sum(
                1 for obs in self.connection_observations.values() if obs.saw_final_event
            )
            request_post_latency_ms = None
            if self.post_completed_at is not None:
                request_post_latency_ms = (self.post_completed_at - self.send_started_at) * 1000.0

            first_event_ms = None
            if self.first_event_at is not None:
                first_event_ms = (self.first_event_at - self.send_started_at) * 1000.0

            final_event_ms = None
            if self.final_event_at is not None:
                final_event_ms = (self.final_event_at - self.send_started_at) * 1000.0

            return RequestOutcome(
                user_id=self.user_id,
                thread_id=self.thread_id,
                sequence=self.sequence,
                status=self.final_event_type or "unknown",
                send_accepted=self.send_accepted,
                request_post_latency_ms=request_post_latency_ms,
                time_to_first_event_ms=first_event_ms,
                time_to_final_event_ms=final_event_ms,
                connections_with_any_event=any_event_count,
                connections_with_final_event=final_event_count,
                all_connections_saw_any_event=any_event_count == self.connection_count,
                all_connections_saw_final_event=final_event_count == self.connection_count,
                final_event_type=self.final_event_type,
                error_message=self.error_message,
            )


@dataclass
class UserState:
    user_id: str
    token: str
    thread_ids: list[str]
    active_lock: asyncio.Lock = field(default_factory=asyncio.Lock)
    active_requests: dict[str, RequestTracker] = field(default_factory=dict)

    async def set_active_request(self, tracker: RequestTracker) -> None:
        async with self.active_lock:
            self.active_requests[tracker.thread_id] = tracker

    async def clear_active_request(self, tracker: RequestTracker) -> None:
        async with self.active_lock:
            current = self.active_requests.get(tracker.thread_id)
            if current is tracker:
                self.active_requests.pop(tracker.thread_id, None)

    async def handle_event(self, connection_index: int, event: dict[str, Any], timestamp: float) -> None:
        event_thread_id = event.get("thread_id")
        if event_thread_id is None:
            return

        async with self.active_lock:
            tracker = self.active_requests.get(event_thread_id)

        if tracker is None:
            return

        await tracker.record_event(connection_index, event, timestamp)


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def primary_worktree_root(root: Path) -> Path:
    try:
        output = subprocess.check_output(
            ["git", "worktree", "list", "--porcelain"],
            cwd=root,
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except (OSError, subprocess.CalledProcessError):
        return root

    for line in output.splitlines():
        if line.startswith("worktree "):
            return Path(line.split(" ", 1)[1])
    return root


def latest_mtime(path: Path) -> float:
    """Return the newest mtime under a file or directory."""
    if not path.exists():
        return 0.0
    if path.is_file():
        return path.stat().st_mtime

    latest = path.stat().st_mtime
    for root, dirnames, filenames in os.walk(path):
        dirnames[:] = [dirname for dirname in dirnames if dirname != "target"]
        for name in filenames:
            child = Path(root) / name
            try:
                latest = max(latest, child.stat().st_mtime)
            except FileNotFoundError:
                continue
    return latest


def binary_needs_rebuild(binary: Path, root: Path) -> bool:
    """Rebuild when the binary is missing or older than embedded sources."""
    if not binary.exists():
        return True

    binary_mtime = binary.stat().st_mtime
    inputs = [
        root / "Cargo.toml",
        root / "Cargo.lock",
        root / "build.rs",
        root / "providers.json",
        root / "src",
        root / "channels-src",
        root / "crates",
    ]
    return any(latest_mtime(path) > binary_mtime for path in inputs)


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


async def wait_for_ready(url: str, *, timeout: float = 60.0, interval: float = 0.5) -> None:
    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            try:
                response = await client.get(url, timeout=5.0)
                if response.status_code == 200:
                    return
            except (httpx.ConnectError, httpx.ReadError, httpx.TimeoutException):
                pass
            await asyncio.sleep(interval)
    raise TimeoutError(f"Service at {url} not ready after {timeout}s")


async def wait_for_port_line(process: asyncio.subprocess.Process, pattern: str, *, timeout: float) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        try:
            line = await asyncio.wait_for(process.stdout.readline(), timeout=remaining)
        except asyncio.TimeoutError:
            break
        if not line:
            continue
        decoded = line.decode("utf-8", errors="replace").strip()
        match = re.search(pattern, decoded)
        if match:
            return int(match.group(1))
    raise TimeoutError(f"Port pattern '{pattern}' not found after {timeout}s")


async def stop_process(
    process: asyncio.subprocess.Process,
    *,
    sig: int | None = None,
    timeout: float = 10.0,
) -> None:
    if process.returncode is not None:
        return

    try:
        if sig is None:
            process.kill()
        else:
            process.send_signal(sig)
    except ProcessLookupError:
        return

    try:
        await asyncio.wait_for(process.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        if sig is not None and process.returncode is None:
            process.kill()
            try:
                await asyncio.wait_for(process.wait(), timeout=2.0)
            except asyncio.TimeoutError:
                pass


async def sample_process(pid: int, interval_secs: float, stop_event: asyncio.Event) -> list[RuntimeSample]:
    samples: list[RuntimeSample] = []
    while not stop_event.is_set():
        try:
            proc = await asyncio.create_subprocess_exec(
                "ps",
                "-o",
                "rss=",
                "-o",
                "%cpu=",
                "-p",
                str(pid),
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.DEVNULL,
            )
            stdout, _ = await proc.communicate()
            fields = stdout.decode("utf-8", errors="replace").strip().split()
            if len(fields) >= 2:
                rss_mb = float(fields[0]) / 1024.0
                cpu_percent = float(fields[1])
                samples.append(
                    RuntimeSample(
                        timestamp=time.monotonic(),
                        cpu_percent=cpu_percent,
                        rss_mb=rss_mb,
                    )
                )
        except (ValueError, OSError):
            pass

        try:
            await asyncio.wait_for(stop_event.wait(), timeout=interval_secs)
        except asyncio.TimeoutError:
            continue
    return samples


def percentile(values: list[float], pct: float) -> float | None:
    if not values:
        return None
    if len(values) == 1:
        return values[0]

    ordered = sorted(values)
    rank = (len(ordered) - 1) * pct
    low = int(rank)
    high = min(low + 1, len(ordered) - 1)
    weight = rank - low
    return ordered[low] * (1.0 - weight) + ordered[high] * weight


def utc_timestamp() -> str:
    return datetime.now(UTC).strftime("%Y%m%dT%H%M%S.%fZ")


def json_default(value: Any) -> Any:
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, RuntimeSample):
        return asdict(value)
    if isinstance(value, RequestOutcome):
        return asdict(value)
    raise TypeError(f"Unsupported value: {type(value)!r}")


async def start_mock_llm(root: Path, *, startup_timeout: float) -> tuple[asyncio.subprocess.Process, str]:
    script = root / "tests" / "e2e" / "mock_llm.py"
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        str(script),
        "--port",
        "0",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    port = await wait_for_port_line(process, r"MOCK_LLM_PORT=(\d+)", timeout=startup_timeout)
    url = f"http://127.0.0.1:{port}"
    await wait_for_ready(f"{url}/v1/models", timeout=startup_timeout)
    return process, url


def benchmark_env(
    *,
    home_dir: str,
    db_path: str,
    gateway_port: int,
    http_port: int,
    mock_llm_url: str,
    args: argparse.Namespace,
) -> tuple[dict[str, str], dict[str, Any]]:
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "RUST_LOG": args.rust_log,
        "RUST_BACKTRACE": "1",
        "IRONCLAW_OWNER_ID": DEFAULT_OWNER_ID,
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": DEFAULT_ADMIN_TOKEN,
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_url,
        "LLM_MODEL": DEFAULT_MOCK_MODEL,
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": db_path,
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "false",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "false",
        "ONBOARD_COMPLETED": "true",
        "IRONCLAW_BENCH_RUNTIME_MODE": args.runtime_mode,
    }

    public_overrides: dict[str, Any] = {
        "IRONCLAW_BENCH_RUNTIME_MODE": args.runtime_mode,
        "LLM_BACKEND": env["LLM_BACKEND"],
        "DATABASE_BACKEND": env["DATABASE_BACKEND"],
        "HTTP_HOST": env["HTTP_HOST"],
        "HTTP_PORT": http_port,
        "SANDBOX_ENABLED": env["SANDBOX_ENABLED"],
        "SKILLS_ENABLED": env["SKILLS_ENABLED"],
        "ROUTINES_ENABLED": env["ROUTINES_ENABLED"],
        "HEARTBEAT_ENABLED": env["HEARTBEAT_ENABLED"],
        "EMBEDDING_ENABLED": env["EMBEDDING_ENABLED"],
        "WASM_ENABLED": env["WASM_ENABLED"],
    }

    if args.agent_max_parallel_jobs is not None:
        env["AGENT_MAX_PARALLEL_JOBS"] = str(args.agent_max_parallel_jobs)
        public_overrides["AGENT_MAX_PARALLEL_JOBS"] = args.agent_max_parallel_jobs
    if args.tenant_max_llm_concurrent is not None:
        env["TENANT_MAX_LLM_CONCURRENT"] = str(args.tenant_max_llm_concurrent)
        public_overrides["TENANT_MAX_LLM_CONCURRENT"] = args.tenant_max_llm_concurrent
    if args.tenant_max_jobs_concurrent is not None:
        env["TENANT_MAX_JOBS_CONCURRENT"] = str(args.tenant_max_jobs_concurrent)
        public_overrides["TENANT_MAX_JOBS_CONCURRENT"] = args.tenant_max_jobs_concurrent
    if args.auto_approve_tools:
        env["AGENT_AUTO_APPROVE_TOOLS"] = "true"
        public_overrides["AGENT_AUTO_APPROVE_TOOLS"] = True

    return env, public_overrides


async def start_ironclaw(
    root: Path,
    binary: Path,
    *,
    gateway_port: int,
    http_port: int,
    mock_llm_url: str,
    args: argparse.Namespace,
    home_dir: str,
    db_path: str,
) -> tuple[asyncio.subprocess.Process, str, dict[str, Any]]:
    env, public_overrides = benchmark_env(
        home_dir=home_dir,
        db_path=db_path,
        gateway_port=gateway_port,
        http_port=http_port,
        mock_llm_url=mock_llm_url,
        args=args,
    )
    process = await asyncio.create_subprocess_exec(
        str(binary),
        "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )
    base_url = f"http://127.0.0.1:{gateway_port}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=args.startup_timeout)
    except TimeoutError as exc:
        stderr_bytes = b""
        if process.stderr is not None:
            try:
                stderr_bytes = await asyncio.wait_for(process.stderr.read(8192), timeout=2.0)
            except asyncio.TimeoutError:
                pass
        await stop_process(process, timeout=2.0)
        stderr_text = stderr_bytes.decode("utf-8", errors="replace")
        raise RuntimeError(
            f"IronClaw failed to start within {args.startup_timeout}s.\nstderr:\n{stderr_text}"
        ) from exc
    return process, base_url, public_overrides


async def api_post_json(
    client: httpx.AsyncClient,
    base_url: str,
    path: str,
    token: str,
    payload: dict[str, Any] | None = None,
    *,
    timeout: float = 30.0,
) -> httpx.Response:
    request_kwargs: dict[str, Any] = {
        "headers": {"Authorization": f"Bearer {token}"},
        "timeout": timeout,
    }
    if payload is not None:
        request_kwargs["json"] = payload
    return await client.post(
        f"{base_url}{path}",
        **request_kwargs,
    )


async def create_users(
    client: httpx.AsyncClient, base_url: str, user_count: int
) -> list[dict[str, str]]:
    users: list[dict[str, str]] = []
    for index in range(user_count):
        response = await api_post_json(
            client,
            base_url,
            "/api/admin/users",
            DEFAULT_ADMIN_TOKEN,
            {
                "display_name": f"Bench User {index + 1:03d}",
                "role": "member",
            },
        )
        response.raise_for_status()
        payload = response.json()
        users.append({"id": payload["id"], "token": payload["token"]})
    return users


async def create_threads(
    client: httpx.AsyncClient,
    base_url: str,
    users: list[dict[str, str]],
    threads_per_user: int,
) -> list[UserState]:
    user_states: list[UserState] = []
    for user in users:
        thread_ids: list[str] = []
        for _ in range(threads_per_user):
            response = await api_post_json(
                client,
                base_url,
                "/api/chat/thread/new",
                user["token"],
                None,
            )
            response.raise_for_status()
            payload = response.json()
            thread_ids.append(payload["id"])
        user_states.append(
            UserState(
                user_id=user["id"],
                token=user["token"],
                thread_ids=thread_ids,
            )
        )
    return user_states


async def iter_sse_events(response: aiohttp.ClientResponse):
    data_lines: list[str] = []
    while True:
        raw_line = await response.content.readline()
        if not raw_line:
            break

        line = raw_line.decode("utf-8", errors="replace").rstrip("\r\n")
        if not line:
            if not data_lines:
                continue

            payload = "\n".join(data_lines)
            data_lines.clear()
            try:
                yield json.loads(payload)
            except json.JSONDecodeError:
                continue
            continue

        if line.startswith(":"):
            continue

        if line.startswith("data:"):
            data_lines.append(line[5:].lstrip())

    if data_lines:
        payload = "\n".join(data_lines)
        try:
            yield json.loads(payload)
        except json.JSONDecodeError:
            return


async def sse_listener(
    *,
    base_url: str,
    user_state: UserState,
    connection_index: int,
    startup_future: asyncio.Future[None],
    stop_event: asyncio.Event,
) -> None:
    timeout = aiohttp.ClientTimeout(total=None, sock_read=None, connect=30.0)
    try:
        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.get(
                f"{base_url}/api/chat/events?token={user_state.token}",
                headers={"Accept": "text/event-stream"},
            ) as response:
                if response.status != 200:
                    body = await response.text()
                    raise RuntimeError(
                        f"SSE connection {connection_index} for user {user_state.user_id} "
                        f"failed with {response.status}: {body}"
                    )
                if not startup_future.done():
                    startup_future.set_result(None)
                async for event in iter_sse_events(response):
                    if stop_event.is_set():
                        break
                    await user_state.handle_event(connection_index, event, time.monotonic())
    except asyncio.CancelledError:
        if not startup_future.done():
            startup_future.cancel()
        raise
    except Exception as exc:
        if not startup_future.done():
            startup_future.set_exception(exc)
            return
        raise


async def run_user_workload(
    *,
    client: httpx.AsyncClient,
    base_url: str,
    user_state: UserState,
    max_in_flight: asyncio.Semaphore,
    args: argparse.Namespace,
) -> list[RequestOutcome]:
    pending_requests: asyncio.Queue[tuple[int, str]] = asyncio.Queue()
    for sequence, thread_id in enumerate(user_state.thread_ids, start=1):
        pending_requests.put_nowait((sequence, thread_id))

    async def worker() -> list[RequestOutcome]:
        worker_outcomes: list[RequestOutcome] = []
        while True:
            try:
                sequence, thread_id = pending_requests.get_nowait()
            except asyncio.QueueEmpty:
                break

            tracker = RequestTracker(
                user_id=user_state.user_id,
                thread_id=thread_id,
                sequence=sequence,
                connection_count=args.sse_connections_per_user,
                send_started_at=time.monotonic(),
            )

            async with max_in_flight:
                await user_state.set_active_request(tracker)
                try:
                    content = (
                        f"[bench user={user_state.user_id} seq={sequence}] "
                        "What is 2 + 2?"
                    )
                    response = await api_post_json(
                        client,
                        base_url,
                        "/api/chat/send",
                        user_state.token,
                        {"content": content, "thread_id": thread_id},
                        timeout=args.request_timeout,
                    )
                    error_message = None
                    accepted = response.status_code == 202
                    if not accepted:
                        error_message = (
                            f"POST /api/chat/send returned {response.status_code}: {response.text}"
                        )
                    await tracker.mark_post_result(
                        accepted=accepted,
                        completed_at=time.monotonic(),
                        error=error_message,
                    )

                    if accepted:
                        try:
                            await asyncio.wait_for(
                                tracker.final_event.wait(), timeout=args.request_timeout
                            )
                        except asyncio.TimeoutError:
                            await tracker.finalize_timeout(
                                "Timed out waiting for final SSE event after "
                                f"{args.request_timeout}s"
                            )

                    await asyncio.sleep(args.delivery_grace_secs)
                    worker_outcomes.append(await tracker.snapshot())
                finally:
                    await user_state.clear_active_request(tracker)
                    pending_requests.task_done()

        return worker_outcomes

    worker_count = min(args.senders_per_user, len(user_state.thread_ids))
    per_worker_outcomes = await asyncio.gather(*(worker() for _ in range(worker_count)))
    return [outcome for outcomes in per_worker_outcomes for outcome in outcomes]


def summarize_results(
    *,
    request_outcomes: list[RequestOutcome],
    runtime_samples: list[RuntimeSample],
    workload_started_at: float,
    workload_finished_at: float,
    args: argparse.Namespace,
    env_overrides: dict[str, Any],
    base_url: str,
) -> dict[str, Any]:
    completed = sum(1 for outcome in request_outcomes if outcome.status == "response")
    errors = sum(
        1
        for outcome in request_outcomes
        if outcome.status not in {"response", "timeout"}
    )
    timeouts = sum(1 for outcome in request_outcomes if outcome.status == "timeout")
    first_event_latencies = [
        outcome.time_to_first_event_ms
        for outcome in request_outcomes
        if outcome.time_to_first_event_ms is not None
    ]
    final_event_latencies = [
        outcome.time_to_final_event_ms
        for outcome in request_outcomes
        if outcome.time_to_final_event_ms is not None
    ]

    expected_connection_deliveries = len(request_outcomes) * args.sse_connections_per_user
    actual_any_event_deliveries = sum(
        outcome.connections_with_any_event for outcome in request_outcomes
    )
    actual_final_event_deliveries = sum(
        outcome.connections_with_final_event for outcome in request_outcomes
    )
    any_event_delivery_rate = (
        actual_any_event_deliveries / expected_connection_deliveries
        if expected_connection_deliveries
        else 0.0
    )
    final_event_delivery_rate = (
        actual_final_event_deliveries / expected_connection_deliveries
        if expected_connection_deliveries
        else 0.0
    )

    cpu_values = [sample.cpu_percent for sample in runtime_samples]
    rss_values = [sample.rss_mb for sample in runtime_samples]

    return {
        "label": args.label,
        "generated_at": datetime.now(UTC).isoformat(),
        "benchmark": {
            "base_url": base_url,
            "runtime_mode": args.runtime_mode,
            "user_count": args.user_count,
            "sse_connections_per_user": args.sse_connections_per_user,
            "senders_per_user": args.senders_per_user,
            "messages_per_user": args.messages_per_user,
            "max_in_flight_requests": args.max_in_flight_requests,
            "request_timeout_secs": args.request_timeout,
            "delivery_grace_secs": args.delivery_grace_secs,
            "sampling_interval_secs": args.sample_interval_secs,
        },
        "env_overrides": env_overrides,
        "workload_timing": {
            "started_at_monotonic": workload_started_at,
            "finished_at_monotonic": workload_finished_at,
            "duration_secs": workload_finished_at - workload_started_at,
        },
        "summary": {
            "requests_total": len(request_outcomes),
            "requests_completed": completed,
            "errors": errors,
            "timeouts": timeouts,
            "time_to_first_event_ms": {
                "p50": percentile(first_event_latencies, 0.50),
                "p95": percentile(first_event_latencies, 0.95),
                "p99": percentile(first_event_latencies, 0.99),
            },
            "time_to_final_event_ms": {
                "p50": percentile(final_event_latencies, 0.50),
                "p95": percentile(final_event_latencies, 0.95),
                "p99": percentile(final_event_latencies, 0.99),
            },
            "sse_delivery": {
                "expected_connection_deliveries": expected_connection_deliveries,
                "any_event_delivery_rate": any_event_delivery_rate,
                "final_event_delivery_rate": final_event_delivery_rate,
                "requests_all_connections_saw_any_event": sum(
                    1 for outcome in request_outcomes if outcome.all_connections_saw_any_event
                ),
                "requests_all_connections_saw_final_event": sum(
                    1 for outcome in request_outcomes if outcome.all_connections_saw_final_event
                ),
            },
            "cpu_percent": {
                "avg": statistics.fmean(cpu_values) if cpu_values else None,
                "max": max(cpu_values) if cpu_values else None,
            },
            "rss_mb": {
                "avg": statistics.fmean(rss_values) if rss_values else None,
                "max": max(rss_values) if rss_values else None,
            },
        },
        "runtime_samples": runtime_samples,
        "requests": request_outcomes,
    }


def print_summary(summary: dict[str, Any], output_path: Path) -> None:
    time_to_first = summary["summary"]["time_to_first_event_ms"]
    time_to_final = summary["summary"]["time_to_final_event_ms"]
    sse_delivery = summary["summary"]["sse_delivery"]
    cpu = summary["summary"]["cpu_percent"]
    rss = summary["summary"]["rss_mb"]

    print()
    print(f"label: {summary['label']}")
    print(
        f"requests: {summary['summary']['requests_completed']}/"
        f"{summary['summary']['requests_total']} completed, "
        f"errors={summary['summary']['errors']}, "
        f"timeouts={summary['summary']['timeouts']}"
    )
    print(
        "time_to_first_event_ms: "
        f"p50={format_metric(time_to_first['p50'])} "
        f"p95={format_metric(time_to_first['p95'])} "
        f"p99={format_metric(time_to_first['p99'])}"
    )
    print(
        "time_to_final_event_ms: "
        f"p50={format_metric(time_to_final['p50'])} "
        f"p95={format_metric(time_to_final['p95'])} "
        f"p99={format_metric(time_to_final['p99'])}"
    )
    print(
        "sse_delivery: "
        f"any={sse_delivery['any_event_delivery_rate']:.3f} "
        f"final={sse_delivery['final_event_delivery_rate']:.3f}"
    )
    print(
        "resource_usage: "
        f"cpu_avg={format_metric(cpu['avg'])}% "
        f"cpu_max={format_metric(cpu['max'])}% "
        f"rss_avg={format_metric(rss['avg'])}MB "
        f"rss_max={format_metric(rss['max'])}MB"
    )
    print(f"results_json: {output_path}")


def format_metric(value: float | None) -> str:
    if value is None:
        return "n/a"
    return f"{value:.2f}"


def build_command_for_display(command: list[str]) -> str:
    return " ".join(shlex.quote(part) for part in command)


async def async_main(args: argparse.Namespace) -> int:
    if args.user_count <= 0:
        raise ValueError("--user-count must be positive")
    if args.sse_connections_per_user <= 0:
        raise ValueError("--sse-connections-per-user must be positive")
    if args.messages_per_user <= 0:
        raise ValueError("--messages-per-user must be positive")
    if args.senders_per_user <= 0:
        raise ValueError("--senders-per-user must be positive")
    if args.max_in_flight_requests <= 0:
        raise ValueError("--max-in-flight-requests must be positive")
    if args.user_count * args.sse_connections_per_user > GATEWAY_CONNECTION_LIMIT:
        raise ValueError(
            "Requested SSE fanout exceeds the gateway connection limit of "
            f"{GATEWAY_CONNECTION_LIMIT}: "
            f"{args.user_count} users * {args.sse_connections_per_user} connections"
        )

    root = repo_root()
    main_root = primary_worktree_root(root)
    default_binary = main_root / "target" / "debug" / "ironclaw"
    binary = Path(args.ironclaw_binary) if args.ironclaw_binary else default_binary
    build_root = main_root if args.ironclaw_binary is None else root
    if not args.skip_build and binary_needs_rebuild(binary, build_root):
        build_cmd = [
            "cargo",
            "build",
            "--no-default-features",
            "--features",
            "libsql,bench-runtime",
        ]
        print(f"building ironclaw: {build_command_for_display(build_cmd)}")
        subprocess.run(build_cmd, cwd=build_root, check=True, timeout=900)

    if not binary.exists():
        raise FileNotFoundError(
            f"IronClaw binary not found at {binary}. Run cargo build or omit --skip-build."
        )

    artifacts_tmpdir: tempfile.TemporaryDirectory[str] | None = None
    if args.keep_artifacts:
        artifacts_root = Path(tempfile.mkdtemp(prefix="ironclaw-bench-"))
    else:
        artifacts_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-bench-")
        artifacts_root = Path(artifacts_tmpdir.name)

    home_dir = artifacts_root / "home"
    db_dir = artifacts_root / "db"
    home_dir.mkdir(parents=True, exist_ok=True)
    db_dir.mkdir(parents=True, exist_ok=True)
    db_path = db_dir / "benchmark.db"
    gateway_port = find_free_port()
    http_port = find_free_port()

    mock_llm_process: asyncio.subprocess.Process | None = None
    ironclaw_process: asyncio.subprocess.Process | None = None
    sse_stop_event = asyncio.Event()
    sampler_stop_event = asyncio.Event()
    sse_tasks: list[asyncio.Task[Any]] = []
    sampler_task: asyncio.Task[list[RuntimeSample]] | None = None

    try:
        mock_llm_process, mock_llm_url = await start_mock_llm(
            root, startup_timeout=args.startup_timeout
        )
        ironclaw_process, base_url, env_overrides = await start_ironclaw(
            root,
            binary,
            gateway_port=gateway_port,
            http_port=http_port,
            mock_llm_url=mock_llm_url,
            args=args,
            home_dir=str(home_dir),
            db_path=str(db_path),
        )

        async with httpx.AsyncClient() as client:
            users = await create_users(client, base_url, args.user_count)
            user_states = await create_threads(
                client, base_url, users, threads_per_user=args.messages_per_user
            )

            startup_futures: list[asyncio.Future[None]] = []
            for user_state in user_states:
                for connection_index in range(args.sse_connections_per_user):
                    startup_future = asyncio.get_running_loop().create_future()
                    startup_futures.append(startup_future)
                    sse_tasks.append(
                        asyncio.create_task(
                            sse_listener(
                                base_url=base_url,
                                user_state=user_state,
                                connection_index=connection_index,
                                startup_future=startup_future,
                                stop_event=sse_stop_event,
                            )
                        )
                    )

            await asyncio.wait_for(
                asyncio.gather(*startup_futures),
                timeout=args.startup_timeout,
            )

            sampler_task = asyncio.create_task(
                sample_process(
                    ironclaw_process.pid,
                    args.sample_interval_secs,
                    sampler_stop_event,
                )
            )

            workload_started_at = time.monotonic()
            max_in_flight = asyncio.Semaphore(args.max_in_flight_requests)
            per_user_results = await asyncio.gather(
                *(
                    run_user_workload(
                        client=client,
                        base_url=base_url,
                        user_state=user_state,
                        max_in_flight=max_in_flight,
                        args=args,
                    )
                    for user_state in user_states
                )
            )
            workload_finished_at = time.monotonic()

        request_outcomes = [
            outcome for user_results in per_user_results for outcome in user_results
        ]
        sampler_stop_event.set()
        runtime_samples = await sampler_task if sampler_task is not None else []

        summary = summarize_results(
            request_outcomes=request_outcomes,
            runtime_samples=runtime_samples,
            workload_started_at=workload_started_at,
            workload_finished_at=workload_finished_at,
            args=args,
            env_overrides=env_overrides,
            base_url=base_url,
        )

        output_dir = Path(args.results_dir).expanduser()
        output_dir.mkdir(parents=True, exist_ok=True)
        output_path = output_dir / f"{args.label}-{utc_timestamp()}.json"
        output_path.write_text(json.dumps(summary, indent=2, default=json_default) + "\n")
        print_summary(summary, output_path)

        return 0
    finally:
        sse_stop_event.set()
        sampler_stop_event.set()
        if sse_tasks:
            for task in sse_tasks:
                task.cancel()
            await asyncio.gather(*sse_tasks, return_exceptions=True)
        if sampler_task is not None and not sampler_task.done():
            await asyncio.gather(sampler_task, return_exceptions=True)
        if ironclaw_process is not None:
            await stop_process(ironclaw_process, sig=signal.SIGINT, timeout=10.0)
        if mock_llm_process is not None:
            await stop_process(mock_llm_process, sig=signal.SIGTERM, timeout=5.0)
        if args.keep_artifacts:
            print(f"artifacts_dir: {artifacts_root}")
        if artifacts_tmpdir is not None:
            artifacts_tmpdir.cleanup()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", default="baseline", help="Result label written into the JSON output.")
    parser.add_argument(
        "--ironclaw-binary",
        help="Path to a prebuilt ironclaw binary. Defaults to target/debug/ironclaw.",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Do not run cargo build even if the binary looks stale or missing.",
    )
    parser.add_argument("--user-count", type=int, default=20)
    parser.add_argument("--sse-connections-per-user", type=int, default=2)
    parser.add_argument("--senders-per-user", type=int, default=1)
    parser.add_argument("--messages-per-user", type=int, default=5)
    parser.add_argument("--max-in-flight-requests", type=int, default=20)
    parser.add_argument("--request-timeout", type=float, default=30.0)
    parser.add_argument("--startup-timeout", type=float, default=60.0)
    parser.add_argument("--delivery-grace-secs", type=float, default=0.75)
    parser.add_argument("--sample-interval-secs", type=float, default=1.0)
    parser.add_argument(
        "--runtime-mode",
        choices=["multi_thread", "current_thread"],
        default="multi_thread",
        help="Sets IRONCLAW_BENCH_RUNTIME_MODE for the spawned server.",
    )
    parser.add_argument("--agent-max-parallel-jobs", type=int)
    parser.add_argument("--tenant-max-llm-concurrent", type=int)
    parser.add_argument("--tenant-max-jobs-concurrent", type=int)
    parser.add_argument(
        "--auto-approve-tools",
        action="store_true",
        help="Sets AGENT_AUTO_APPROVE_TOOLS=true for tool-inclusive variants.",
    )
    parser.add_argument(
        "--results-dir",
        default=str(DEFAULT_RESULTS_DIR),
        help="Directory for JSON benchmark result files.",
    )
    parser.add_argument(
        "--keep-artifacts",
        action="store_true",
        help="Leave the temporary HOME and libSQL database on disk for inspection.",
    )
    parser.add_argument(
        "--rust-log",
        default="ironclaw=warn",
        help="RUST_LOG value for the spawned IronClaw process.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    return asyncio.run(async_main(args))


if __name__ == "__main__":
    raise SystemExit(main())
