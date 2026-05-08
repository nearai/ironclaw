"""Helpers for live-LLM Playwright tests.

Mirrors the Rust `LiveTestHarnessBuilder` pattern (see
`tests/support/live_harness.rs`). Spins up the `live_llm_proxy.py`
record/replay proxy in front of a real or recorded LLM, points an
ironclaw instance at it, and lets a Playwright test drive the chat
flow against deterministic LLM output.

Modes
-----

- **Record** (`IRONCLAW_LIVE_TEST=1`): the proxy forwards
  `/v1/chat/completions` to the upstream LLM whose URL/key/model are
  configured via `IRONCLAW_LIVE_LLM_BASE_URL`, `IRONCLAW_LIVE_LLM_API_KEY`,
  `IRONCLAW_LIVE_LLM_MODEL`. Each prompt+response pair is appended to
  the test's fixture file.

- **Replay** (default): the proxy reads the committed fixture and
  serves recorded responses by canonical-request hash. Tests are
  skipped (with a clear message) when the fixture is missing so a
  fresh checkout doesn't hard-fail before someone has recorded one.

Fixture path convention
-----------------------

Per-test fixtures live at::

    tests/e2e/fixtures/live/<test_name>.json

where ``<test_name>`` is the bare test function name (no module
prefix). The convention matches the Rust live harness which keys on
the `#[tokio::test]` function name.
"""

from __future__ import annotations

import asyncio
import os
import re
import signal
import sys
from pathlib import Path
from typing import Any, AsyncIterator

import httpx
import pytest


HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "live"
PROXY_SCRIPT = HERE / "live_llm_proxy.py"


def is_live_mode() -> bool:
    """True when the test should record a fresh trace from a real LLM."""
    return os.environ.get("IRONCLAW_LIVE_TEST", "").strip() in ("1", "true")


def fixture_path_for(test_name: str) -> Path:
    """Return the JSON fixture path for a given test."""
    return FIXTURE_DIR / f"{test_name}.json"


async def _wait_for_port_line(
    process: asyncio.subprocess.Process, pattern: str, *, timeout: float = 10.0
) -> int:
    """Read the proxy's stdout until `pattern` matches (and capture group 1)."""
    deadline = asyncio.get_event_loop().time() + timeout
    rx = re.compile(pattern)
    assert process.stdout is not None
    while asyncio.get_event_loop().time() < deadline:
        line = await asyncio.wait_for(process.stdout.readline(), timeout=timeout)
        if not line:
            raise RuntimeError(
                f"live_llm_proxy exited before emitting {pattern!r}; "
                f"stderr (truncated): {(await process.stderr.read(2000)).decode()}"
            )
        decoded = line.decode("utf-8", errors="replace").strip()
        m = rx.search(decoded)
        if m:
            return int(m.group(1))
    raise asyncio.TimeoutError(f"live_llm_proxy did not emit {pattern!r} in {timeout}s")


async def start_live_proxy(
    test_name: str,
    *,
    record_required: bool = False,
) -> AsyncIterator[dict[str, Any]]:
    """Async generator: spin up the proxy and yield ``{"url": ..., "fixture":
    ..., "mode": ...}``. The caller supplies ``test_name`` from
    ``request.node.name``.

    In replay mode with no committed fixture, raises ``pytest.skip`` so
    a fresh checkout does not hard-fail.
    """
    fixture = fixture_path_for(test_name)
    mode = "record" if is_live_mode() else "replay"

    if mode == "replay" and not fixture.exists():
        pytest.skip(
            f"no live-LLM trace fixture at {fixture.relative_to(HERE.parent.parent)}. "
            f"To record one, set IRONCLAW_LIVE_TEST=1 plus IRONCLAW_LIVE_LLM_BASE_URL / "
            f"IRONCLAW_LIVE_LLM_API_KEY / IRONCLAW_LIVE_LLM_MODEL and re-run."
        )

    if record_required and mode != "record":
        pytest.skip(
            "this test must run in record mode (IRONCLAW_LIVE_TEST=1)"
        )

    if mode == "record":
        if not os.environ.get("IRONCLAW_LIVE_LLM_BASE_URL"):
            pytest.skip(
                "record mode requires IRONCLAW_LIVE_LLM_BASE_URL "
                "(and usually IRONCLAW_LIVE_LLM_API_KEY / IRONCLAW_LIVE_LLM_MODEL)"
            )

    proxy_stderr_log = os.environ.get("IRONCLAW_LIVE_PROXY_STDERR_LOG")
    proxy_stderr: Any = asyncio.subprocess.PIPE
    if proxy_stderr_log:
        proxy_stderr = open(proxy_stderr_log, "w")  # noqa: SIM115
    proc = await asyncio.create_subprocess_exec(
        sys.executable,
        str(PROXY_SCRIPT),
        "--port",
        "0",
        "--fixture",
        str(fixture),
        "--mode",
        mode,
        stdout=asyncio.subprocess.PIPE,
        stderr=proxy_stderr,
        env={**os.environ},
    )
    try:
        port = await _wait_for_port_line(proc, r"LIVE_LLM_PROXY_PORT=(\d+)", timeout=15)
        url = f"http://127.0.0.1:{port}"
        # Wait for the /v1/models endpoint to come up.
        deadline = asyncio.get_event_loop().time() + 10
        while asyncio.get_event_loop().time() < deadline:
            try:
                async with httpx.AsyncClient() as client:
                    r = await client.get(f"{url}/v1/models", timeout=2)
                    if r.status_code == 200:
                        break
            except Exception:
                pass
            await asyncio.sleep(0.1)
        yield {
            "url": url,
            "fixture": fixture,
            "mode": mode,
        }
    finally:
        if proc.returncode is None:
            try:
                proc.send_signal(signal.SIGINT)
                await asyncio.wait_for(proc.wait(), timeout=5)
            except (asyncio.TimeoutError, ProcessLookupError):
                proc.kill()


async def proxy_state(url: str) -> dict[str, Any]:
    """Read the proxy's runtime state (entry counts, mode, miss count)."""
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{url}/__live/state", timeout=10)
        response.raise_for_status()
        return response.json()
