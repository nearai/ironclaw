"""pytest fixtures for E2E tests.

Session-scoped fixtures build the canonical binary and start hermetic provider
doubles used by the Reborn E2E suites.
"""

import asyncio
from collections.abc import AsyncIterator
import json
import os
import signal
import shutil
import socket
import subprocess
import sys
from pathlib import Path
from typing import Any

import httpx
import pytest

from helpers import (
    EMULATE_GITHUB_BEARER,
    EMULATE_GOOGLE_BEARER,
    EMULATE_SLACK_BEARER,
    wait_for_port_line,
    wait_for_ready,
)

# Project root (two levels up from tests/e2e/)
ROOT = Path(__file__).resolve().parent.parent.parent

# Git main repo root (for worktree support — WASM build artifacts live
# in the main repo's tools-src/*/target/ and aren't shared across worktrees)
_MAIN_ROOT = None
try:
    import subprocess as _sp
    _common = _sp.check_output(
        ["git", "worktree", "list", "--porcelain"],
        cwd=ROOT, text=True, stderr=_sp.DEVNULL,
    )
    for line in _common.splitlines():
        if line.startswith("worktree "):
            _MAIN_ROOT = Path(line.split(" ", 1)[1])
            break  # first entry is always the main worktree
except Exception:
    pass

EMULATE_NPM_PACKAGE = "emulate@0.7.0"
EMULATE_GOOGLE_SEED = ROOT / "tests/e2e/fixtures/emulate/google_gmail.yaml"
EMULATE_SLACK_SEED = ROOT / "tests/e2e/fixtures/emulate/slack.yaml"
EMULATE_GITHUB_SEED = ROOT / "tests/e2e/fixtures/emulate/github.yaml"
EMULATE_GOOGLE_READY_TOKEN = EMULATE_GOOGLE_BEARER
EMULATE_SLACK_READY_TOKEN = EMULATE_SLACK_BEARER
EMULATE_GITHUB_READY_TOKEN = EMULATE_GITHUB_BEARER
EMULATE_STARTUP_ATTEMPTS = 120
EMULATE_STARTUP_POLL_SECONDS = 0.5

# test-tools/*.zip are git-ignored build artifacts (test-tools/README.md);
# rebuild whenever a tool's manifest/schema/prompt/wasm-src changes so an
# uploaded fixture always matches checked-in source.
TEST_TOOLS_DIR = ROOT / "test-tools"
BUILD_TEST_TOOLS_SCRIPT = ROOT / "scripts" / "build-test-tools.sh"
TEST_TOOL_NAMES = ("ascii-renderer", "hacker-news", "market-data")


def _latest_mtime(path: Path) -> float:
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


def _cargo_target_dir() -> Path:
    """Resolve the actual cargo target directory.

    Checks (in order):
    1. CARGO_TARGET_DIR env var
    2. build.target-dir in ~/.cargo/config.toml
    3. Falls back to {ROOT}/target
    """
    env_target = os.environ.get("CARGO_TARGET_DIR")
    if env_target:
        return Path(env_target)

    # Check ~/.cargo/config.toml for build.target-dir
    cargo_config = Path.home() / ".cargo" / "config.toml"
    if cargo_config.exists():
        try:
            for line in cargo_config.read_text().splitlines():
                line = line.strip()
                if line.startswith("target-dir"):
                    # Parse: target-dir = "/path/to/dir"
                    _, _, value = line.partition("=")
                    value = value.strip().strip('"').strip("'")
                    if value:
                        return Path(value)
        except Exception:
            pass

    return ROOT / "target"


def _binary_needs_rebuild(binary: Path) -> bool:
    """Rebuild when the binary is missing or older than embedded sources."""
    if not binary.exists():
        return True

    binary_mtime = binary.stat().st_mtime
    inputs = [
        ROOT / "Cargo.toml",
        ROOT / "Cargo.lock",
        ROOT / "build.rs",
        ROOT / "providers.json",
        ROOT / "src",
        ROOT / "channels-src",
        ROOT / "crates",
    ]
    return any(_latest_mtime(path) > binary_mtime for path in inputs)


def _find_free_port() -> int:
    """Bind to port 0 and return the OS-assigned port."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _reserve_loopback_sockets(count: int) -> list[socket.socket]:
    """Bind loopback sockets and keep them open until the server starts."""
    sockets: list[socket.socket] = []
    try:
        while len(sockets) < count:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.bind(("127.0.0.1", 0))
            sockets.append(sock)
        return sockets
    except Exception:
        for sock in sockets:
            sock.close()
        raise

async def _stop_process(
    proc: asyncio.subprocess.Process, *, sig: int | None = None, timeout: float
) -> None:
    """Signal a subprocess and wait briefly without masking exit races."""
    async def _drain_pipes() -> None:
        try:
            await asyncio.wait_for(proc.communicate(), timeout=1)
        except (asyncio.TimeoutError, ValueError):
            pass

    if proc.returncode is not None:
        await _drain_pipes()
        return

    try:
        if sig is None:
            proc.kill()
        else:
            proc.send_signal(sig)
    except ProcessLookupError:
        try:
            await asyncio.wait_for(proc.wait(), timeout=timeout)
        except asyncio.TimeoutError:
            pass
        return

    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        pass
    await _drain_pipes()


def _emulate_unavailable(reason: str) -> None:
    if os.environ.get("CI") == "true":
        pytest.fail(reason)
    pytest.skip(reason)


def _wasip2_target_missing() -> bool:
    try:
        installed = subprocess.run(
            ["rustup", "target", "list", "--installed"],
            capture_output=True,
            text=True,
            check=True,
            timeout=15,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return True
    return "wasm32-wasip2" not in installed.split()


def _test_tool_zip_stale(tool: str) -> bool:
    zip_path = TEST_TOOLS_DIR / f"{tool}.zip"
    if not zip_path.exists():
        return True
    # wasm-src/target/ is a build cache, not a source input; _latest_mtime
    # already skips directories literally named "target".
    return _latest_mtime(TEST_TOOLS_DIR / tool) > zip_path.stat().st_mtime


@pytest.fixture(scope="session")
def test_tool_zips() -> dict[str, Path]:
    """Build (or reuse) the test-tools/*.zip fixture bundles.

    Rebuild only tools whose manifest/schema/prompt/wasm-src changed since
    their zip was last built, via `scripts/build-test-tools.sh`.
    """
    stale = [tool for tool in TEST_TOOL_NAMES if _test_tool_zip_stale(tool)]
    if stale:
        if _wasip2_target_missing():
            _emulate_unavailable(
                "wasm32-wasip2 target not installed "
                "(run: rustup target add wasm32-wasip2) "
                f"-- required to build test-tools/: {', '.join(stale)}"
            )
        print(f"Building test-tools/ fixtures (stale: {', '.join(stale)})...")
        subprocess.run(
            ["bash", str(BUILD_TEST_TOOLS_SCRIPT), *stale],
            cwd=ROOT,
            check=True,
            timeout=300,
        )
    return {tool: TEST_TOOLS_DIR / f"{tool}.zip" for tool in TEST_TOOL_NAMES}


@pytest.fixture(scope="session")
def ironclaw_reborn_binary():
    """Ensure the canonical `ironclaw` binary includes the WebUI surface."""
    target_dir = _cargo_target_dir()
    binary = target_dir / "debug" / "ironclaw"
    if _binary_needs_rebuild(binary):
        print("Building ironclaw (webui-v2-beta; this may take a while)...")
        subprocess.run(
            [
                "cargo", "build",
                "-p", "ironclaw_reborn_cli",
                "--bin", "ironclaw",
                "--features", "webui-v2-beta",
            ],
            cwd=ROOT,
            check=True,
            timeout=600,
        )
    assert binary.exists(), (
        f"Binary not found at {binary}. "
        f"Cargo target dir resolved to: {target_dir}"
    )
    return str(binary)


@pytest.fixture(scope="session")
def ironclaw_reborn_openai_compat_binary():
    """Ensure `ironclaw` is built with the OpenAI-compatible routes.

    `openai-compat-beta` is a strict superset of `webui-v2-beta`, but it is not
    enabled by the generic Reborn WebUI fixture. Keep this separate so the
    OpenAI-compatible E2E explicitly proves the route-bearing binary.
    """
    target_dir = _cargo_target_dir()
    binary = target_dir / "debug" / "ironclaw"
    stamp = target_dir / "debug" / ".ironclaw-reborn-openai-compat-beta.stamp"
    input_mtime = max(
        _latest_mtime(ROOT / "Cargo.toml"),
        _latest_mtime(ROOT / "Cargo.lock"),
        _latest_mtime(ROOT / "build.rs"),
        _latest_mtime(ROOT / "providers.json"),
        _latest_mtime(ROOT / "src"),
        _latest_mtime(ROOT / "channels-src"),
        _latest_mtime(ROOT / "crates"),
    )
    if (
        _binary_needs_rebuild(binary)
        or not stamp.exists()
        or stamp.stat().st_mtime < input_mtime
    ):
        print("Building ironclaw (openai-compat-beta; this may take a while)...")
        subprocess.run(
            [
                "cargo", "build",
                "-p", "ironclaw_reborn_cli",
                "--bin", "ironclaw",
                "--features", "openai-compat-beta",
            ],
            cwd=ROOT,
            check=True,
            timeout=600,
        )
        stamp.parent.mkdir(parents=True, exist_ok=True)
        stamp.touch()
    assert binary.exists(), (
        f"Binary not found at {binary}. "
        f"Cargo target dir resolved to: {target_dir}"
    )
    return str(binary)


@pytest.fixture(scope="session")
async def mock_llm_server():
    """Start the mock LLM server. Yields the base URL."""
    server_script = Path(__file__).parent / "mock_llm.py"
    proc = await asyncio.create_subprocess_exec(
        sys.executable, str(server_script), "--port", "0",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    try:
        port = await wait_for_port_line(proc, r"MOCK_LLM_PORT=(\d+)", timeout=10)
        url = f"http://127.0.0.1:{port}"
        await wait_for_ready(f"{url}/v1/models", timeout=10)
        yield url
    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            await asyncio.wait_for(proc.wait(), timeout=5)
        except asyncio.TimeoutError:
            proc.kill()


async def _run_emulate_server(
    *,
    service: str,
    seed_path: Path,
    ready_method: str,
    ready_path: str,
    ready_headers: dict[str, str],
    ready_json: dict[str, Any] | None = None,
) -> AsyncIterator[dict[str, str]]:
    """Start a pinned Emulate service and wait for a seeded endpoint."""
    if shutil.which("npx") is None:
        _emulate_unavailable(
            f"npx is required to run the Emulate {service} E2E fixture"
        )

    port = _find_free_port()
    url = f"http://127.0.0.1:{port}"
    env = {
        **os.environ,
        "NO_COLOR": "1",
        "EMULATE_PORT": str(port),
    }
    proc = await asyncio.create_subprocess_exec(
        "npx",
        "--yes",
        EMULATE_NPM_PACKAGE,
        "--service",
        service,
        "--port",
        str(port),
        "--seed",
        str(seed_path),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )

    try:
        async with httpx.AsyncClient() as client:
            last_error = ""
            for _ in range(EMULATE_STARTUP_ATTEMPTS):
                if proc.returncode is not None:
                    break
                try:
                    response = await client.request(
                        ready_method,
                        f"{url}{ready_path}",
                        headers=ready_headers,
                        json=ready_json,
                        timeout=2,
                    )
                    if response.status_code == 200:
                        yield {"url": url}
                        return
                    last_error = f"HTTP {response.status_code}: {response.text[:400]}"
                except httpx.HTTPError as exc:
                    last_error = str(exc)
                await asyncio.sleep(EMULATE_STARTUP_POLL_SECONDS)

        stdout = b""
        stderr = b""
        try:
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=2)
        except asyncio.TimeoutError:
            pass
        _emulate_unavailable(
            f"Emulate {service} failed to start. "
            f"Last probe error: {last_error}\n"
            f"stdout:\n{stdout.decode('utf-8', errors='replace')[:2000]}\n"
            f"stderr:\n{stderr.decode('utf-8', errors='replace')[:2000]}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=5)
            if proc.returncode is None:
                await _stop_process(proc, timeout=2)


@pytest.fixture(scope="session")
async def emulate_google_server():
    """Start Emulate Google with seeded Gmail, Calendar, and Drive data."""
    async for server in _run_emulate_server(
        service="google",
        seed_path=EMULATE_GOOGLE_SEED,
        ready_method="GET",
        ready_path="/gmail/v1/users/me/messages",
        ready_headers={
            "Authorization": f"Bearer {EMULATE_GOOGLE_READY_TOKEN}",
        },
    ):
        yield server


@pytest.fixture(scope="session")
async def emulate_slack_server():
    """Start Emulate Slack with a seeded workspace and bot token."""
    async for server in _run_emulate_server(
        service="slack",
        seed_path=EMULATE_SLACK_SEED,
        ready_method="POST",
        ready_path="/api/auth.test",
        ready_headers={
            "Authorization": f"Bearer {EMULATE_SLACK_READY_TOKEN}",
        },
    ):
        yield server


@pytest.fixture(scope="session")
async def emulate_github_server():
    """Start Emulate GitHub with a seeded user, org, and repository."""
    async for server in _run_emulate_server(
        service="github",
        seed_path=EMULATE_GITHUB_SEED,
        ready_method="GET",
        ready_path="/user",
        ready_headers={
            "Authorization": f"Bearer {EMULATE_GITHUB_READY_TOKEN}",
        },
    ):
        yield server


@pytest.fixture(autouse=True)
async def reset_mock_llm_state(mock_llm_server):
    """Reset mutable mock LLM state between tests.

    The mock server is session-scoped, so scenario tests that override the
    fake GitHub API URL or OAuth counters must not leak that state into later
    tests.
    """
    yield
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/set_github_api_url",
            json={"url": "https://api.github.com"},
            timeout=10,
        )
        response.raise_for_status()
        response = await client.post(
            f"{mock_llm_server}/__mock/oauth/reset",
            timeout=10,
        )
        response.raise_for_status()
        response = await client.post(
            f"{mock_llm_server}/__mock/chat_requests/reset",
            timeout=10,
        )
        response.raise_for_status()
        response = await client.post(
            f"{mock_llm_server}/__mock/capability_policy/reset",
            timeout=10,
        )
        if response.status_code != 404:
            response.raise_for_status()


@pytest.fixture(scope="session", autouse=True)
def _wasm_build_symlinks():
    """Symlink WASM build artifacts from the main repo into the worktree.

    In a git worktree, tools-src/*/target/ directories don't exist because
    Cargo build artifacts aren't shared. The install API's source fallback
    checks these paths. Symlinking makes the fallback work without rebuilding.
    """
    if _MAIN_ROOT is None or _MAIN_ROOT == ROOT:
        yield
        return

    created = []
    for src_dir_name in ("tools-src", "channels-src"):
        src_dir = ROOT / src_dir_name
        main_src_dir = _MAIN_ROOT / src_dir_name
        if src_dir.is_dir() and main_src_dir.is_dir():
            for child in src_dir.iterdir():
                if not child.is_dir():
                    continue
                target = child / "target"
                main_target = main_src_dir / child.name / "target"
                if not target.exists() and main_target.is_dir():
                    target.symlink_to(main_target)
                    created.append(target)
    yield
    for link in created:
        if link.is_symlink():
            link.unlink()
