"""E2E regression: engine threads stay out of chat sidebar while history works.

Covers the intended split between the chat sidebar and engine APIs:

- A foreground engine thread spawned by `/api/chat/send` must remain
  discoverable via `/api/engine/threads`, but it must *not* surface as an
  `engine` entry inside `/api/chat/threads`.
- `/api/chat/history?thread_id=<engine-thread-id>` must still synthesize the
  transcript for callers that explicitly deep-link to that engine thread id.

The staging regression merged engine foreground threads into the normal chat
sidebar, which made ordinary prompts look like separate `ENGINE`
conversations. This fixture keeps that bug from coming back while preserving
explicit engine-thread history access.
"""

import asyncio
import os
import signal
import socket
import sys
import tempfile
from pathlib import Path

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import AUTH_TOKEN, api_get, api_post, wait_for_ready

ROOT = Path(__file__).resolve().parent.parent.parent.parent
_V2_VIS_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-v2-visibility-e2e-")
_V2_VIS_HOME_TMPDIR = tempfile.TemporaryDirectory(
    prefix="ironclaw-v2-visibility-e2e-home-"
)


def _forward_coverage_env(env: dict):
    for key in os.environ:
        if key.startswith(
            ("CARGO_LLVM_COV", "LLVM_", "CARGO_ENCODED_RUSTFLAGS", "CARGO_INCREMENTAL")
        ):
            env[key] = os.environ[key]


async def _stop_process(proc, sig=signal.SIGINT, timeout=5):
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()


@pytest.fixture(scope="module")
async def v2_visibility_server(ironclaw_binary, mock_llm_server):
    """Start a dedicated ironclaw instance with ENGINE_V2=true."""
    home_dir = _V2_VIS_HOME_TMPDIR.name
    os.makedirs(os.path.join(home_dir, ".ironclaw"), exist_ok=True)

    socks = []
    for _ in range(2):
        sk = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sk.bind(("127.0.0.1", 0))
        socks.append(sk)
    gateway_port = socks[0].getsockname()[1]
    http_port = socks[1].getsockname()[1]
    for sk in socks:
        sk.close()

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "ENGINE_V2": "true",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-v2-visibility-tester",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(
            _V2_VIS_DB_TMPDIR.name, "v2-visibility-e2e.db"
        ),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "false",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "false",
        "ONBOARD_COMPLETED": "true",
    }
    _forward_coverage_env(env)

    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary,
        "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )

    base_url = f"http://127.0.0.1:{gateway_port}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=60)
        yield base_url
    except TimeoutError:
        if proc.returncode is None:
            await _stop_process(proc, timeout=2)
        stderr_bytes = b""
        if proc.stderr:
            try:
                stderr_bytes = await asyncio.wait_for(
                    proc.stderr.read(8192), timeout=2
                )
            except asyncio.TimeoutError:
                pass
        pytest.fail(
            f"v2 visibility server failed to start on {gateway_port}.\n"
            f"stderr: {stderr_bytes.decode('utf-8', errors='replace')}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


async def _wait_for_assistant_response(
    base_url: str, thread_id: str, *, timeout: float = 45.0
) -> list:
    """Poll history until the most recent turn has an assistant response."""
    for _ in range(int(timeout * 2)):
        r = await api_get(
            base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        r.raise_for_status()
        turns = r.json().get("turns", [])
        if turns and (turns[-1].get("response") or "").strip():
            return turns
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for assistant response in thread {thread_id}"
    )


async def _chat_sidebar_threads(base_url: str) -> list[dict]:
    r = await api_get(base_url, "/api/chat/threads", timeout=15)
    r.raise_for_status()
    return r.json().get("threads", [])


async def _engine_threads(base_url: str) -> list[dict]:
    r = await api_get(base_url, "/api/engine/threads", timeout=15)
    r.raise_for_status()
    return r.json().get("threads", [])


class TestV2ThreadVisibility:
    async def test_engine_thread_stays_out_of_chat_sidebar(
        self, v2_visibility_server
    ):
        """Assistant sends still spawn engine threads, but those execution
        threads must stay out of the normal chat sidebar.
        """
        base = v2_visibility_server

        baseline_engine_ids = {t["id"] for t in await _engine_threads(base)}

        send_r = await api_post(
            base,
            "/api/chat/send",
            json={"content": "hello"},
            timeout=30,
        )
        assert send_r.status_code in (200, 202), send_r.text

        engine_thread = None
        for _ in range(60):
            engine_threads = await _engine_threads(base)
            new_threads = [t for t in engine_threads if t["id"] not in baseline_engine_ids]
            if new_threads:
                engine_thread = new_threads[0]
                break
            await asyncio.sleep(0.5)

        assert engine_thread is not None, "engine thread never materialized"

        sidebar_threads = await _chat_sidebar_threads(base)
        assert all(t.get("channel") != "engine" for t in sidebar_threads), (
            "chat sidebar must not show engine execution threads as normal "
            f"conversations, got {sidebar_threads}"
        )
        assert all(t.get("id") != engine_thread["id"] for t in sidebar_threads), (
            "the newly spawned engine thread must stay discoverable via the "
            "/api/engine/threads surface, not /api/chat/threads"
        )

    async def test_history_synthesizes_messages_for_deep_linked_engine_thread(
        self, v2_visibility_server
    ):
        """Deep-linking by engine thread id must return the transcript even
        though the v1 conversation table has no row under that id.
        """
        base = v2_visibility_server

        baseline_engine_ids = {t["id"] for t in await _engine_threads(base)}

        await api_post(
            base,
            "/api/chat/send",
            json={"content": "hello"},
            timeout=30,
        )

        engine_thread_id = None
        for _ in range(60):
            engine_threads = await _engine_threads(base)
            new_threads = [t for t in engine_threads if t["id"] not in baseline_engine_ids]
            if new_threads:
                engine_thread_id = new_threads[0]["id"]
                break
            await asyncio.sleep(0.5)

        assert engine_thread_id is not None, "engine thread never materialized"

        turns = await _wait_for_assistant_response(
            base, engine_thread_id, timeout=45
        )
        assert turns, "engine-thread deep link must return synthesized history"
        last = turns[-1]
        assert (last.get("user_input") or "").lower().strip() == "hello"
        response = (last.get("response") or "").lower()
        assert "hello" in response or "help" in response, (
            f"expected canned greeting, got {last.get('response')!r}"
        )
