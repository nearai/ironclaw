"""E2E regression: engine v2 threads are visible in sidebar and history.

Covers the behavior PR #2532 introduced in `chat_threads_handler` and
`chat_history_handler`:

- An engine v2 thread created from a `/api/chat/send` call shows up in the
  `/api/chat/threads` sidebar with `channel == "engine"`.
- `/api/chat/history?thread_id=<engine-thread-id>` returns the messages
  synthesized from engine thread transcript even when the v1 conversation
  table has no row for that id (deep-link-by-id path).

Prior behavior silently dropped these threads from the sidebar and
returned an empty history on deep-link; the fixture drives the HTTP
surface directly so the regression survives independent of frontend
polish.
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

    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    gateway_port = s.getsockname()[1]
    s.close()

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


async def _engine_only_threads(base_url: str) -> list[dict]:
    """Return sidebar entries whose channel is engine (the v2-merge path)."""
    r = await api_get(base_url, "/api/chat/threads", timeout=15)
    r.raise_for_status()
    return [t for t in r.json().get("threads", []) if t.get("channel") == "engine"]


class TestV2ThreadVisibility:
    async def test_engine_only_thread_appears_in_sidebar_with_engine_channel(
        self, v2_visibility_server
    ):
        """Send without a client-supplied thread_id: the v1 flow dual-writes
        into the shared assistant conversation, but the engine spins up a
        fresh thread id that has no matching v1 row. The PR's merge should
        surface that engine thread in the sidebar with `channel=engine`.
        """
        base = v2_visibility_server

        baseline = await _engine_only_threads(base)
        baseline_ids = {t["id"] for t in baseline}

        send_r = await api_post(
            base,
            "/api/chat/send",
            json={"content": "hello"},
            timeout=30,
        )
        assert send_r.status_code in (200, 202), send_r.text

        new_engine_entry = None
        for _ in range(60):
            merged = await _engine_only_threads(base)
            new_entries = [t for t in merged if t["id"] not in baseline_ids]
            if new_entries:
                new_engine_entry = new_entries[0]
                break
            await asyncio.sleep(0.5)

        assert new_engine_entry is not None, (
            "a new engine-only thread must appear in the sidebar after an "
            "assistant send with no thread_id; PR #2532 added this merge path"
        )
        assert new_engine_entry.get("title"), (
            f"engine sidebar entry must carry a goal as title, got "
            f"{new_engine_entry}"
        )

    async def test_history_synthesizes_messages_for_deep_linked_engine_thread(
        self, v2_visibility_server
    ):
        """Deep-linking by engine thread id must return the transcript even
        though the v1 conversation table has no row under that id.
        """
        base = v2_visibility_server

        baseline_ids = {t["id"] for t in await _engine_only_threads(base)}

        await api_post(
            base,
            "/api/chat/send",
            json={"content": "hello"},
            timeout=30,
        )

        engine_thread_id = None
        for _ in range(60):
            merged = await _engine_only_threads(base)
            new = [t for t in merged if t["id"] not in baseline_ids]
            if new:
                engine_thread_id = new[0]["id"]
                break
            await asyncio.sleep(0.5)

        assert engine_thread_id is not None, "engine-only thread never materialized"

        # Deep-link by engine thread id. Before PR #2532 this returned an
        # empty turn list because the v1 conversation lookup missed.
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
