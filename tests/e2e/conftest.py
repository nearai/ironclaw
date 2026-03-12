"""pytest fixtures for E2E tests.

Session-scoped: build binary, start mock LLM, start ironclaw, launch browser.
Function-scoped: fresh browser context and page per test.
"""

import asyncio
import os
import signal
import socket
import subprocess
import sys
import tempfile
from pathlib import Path

import pytest

from helpers import AUTH_TOKEN, wait_for_port_line, wait_for_ready

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

# Temp directory for the libSQL database file (cleaned up automatically)
_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-")

# Temp directories for WASM extensions. These start empty and are populated by
# the install pipeline during tests; fixtures do not pre-populate dev build
# artifacts into them.
_WASM_TOOLS_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-wasm-tools-")
_WASM_CHANNELS_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-wasm-channels-")


def _find_free_port() -> int:
    """Bind to port 0 and return the OS-assigned port."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture(scope="session")
def ironclaw_binary():
    """Ensure ironclaw binary is built. Returns the binary path."""
    binary = ROOT / "target" / "debug" / "ironclaw"
    if not binary.exists():
        print("Building ironclaw (this may take a while)...")
        subprocess.run(
            ["cargo", "build", "--no-default-features", "--features", "libsql"],
            cwd=ROOT,
            check=True,
            timeout=600,
        )
    assert binary.exists(), f"Binary not found at {binary}"
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


@pytest.fixture(scope="session")
def wasm_tools_dir(_wasm_build_symlinks):
    """Empty temp dir for WASM tools.

    Starts empty so the server has no pre-loaded extensions at boot.
    The install API (POST /api/extensions/install) downloads and writes
    WASM files here; tests exercise the full install pipeline.

    NOTE on capabilities file naming: Cargo builds with underscored stems
    (web_search_tool.wasm) but capabilities use hyphens (web-search-tool.
    capabilities.json). The loader expects matching stems. If you pre-load
    files, rename caps: web-search-tool → web_search_tool.
    """
    return str(Path(_WASM_TOOLS_TMPDIR.name))


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
    tools_src = ROOT / "tools-src"
    main_tools_src = _MAIN_ROOT / "tools-src"
    if tools_src.is_dir() and main_tools_src.is_dir():
        for tool_dir in tools_src.iterdir():
            if not tool_dir.is_dir():
                continue
            target = tool_dir / "target"
            main_target = main_tools_src / tool_dir.name / "target"
            if not target.exists() and main_target.is_dir():
                target.symlink_to(main_target)
                created.append(target)
    yield
    for link in created:
        if link.is_symlink():
            link.unlink()


@pytest.fixture(scope="session")
async def ironclaw_server(ironclaw_binary, mock_llm_server, wasm_tools_dir):
    """Start the ironclaw gateway. Yields the base URL."""
    gateway_port = _find_free_port()
    env = {
        # Minimal env: PATH for process spawning, HOME for Rust/cargo defaults
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": os.environ.get("HOME", "/tmp"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-tester",
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_DB_TMPDIR.name, "e2e.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        # WASM tool/channel support
        "WASM_ENABLED": "true",
        "WASM_TOOLS_DIR": wasm_tools_dir,
        "WASM_CHANNELS_DIR": _WASM_CHANNELS_TMPDIR.name,
        # Prevent onboarding wizard from triggering
        "ONBOARD_COMPLETED": "true",
        # Force gateway OAuth callback mode (non-loopback URL) and point
        # token exchange at mock_llm.py so OAuth tests work without Google.
        "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
        "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
    }
    # Forward LLVM coverage instrumentation env vars when present
    # (allows cargo-llvm-cov to collect profraw data from E2E runs).
    # Use prefix matching to stay resilient to cargo-llvm-cov changes.
    COV_ENV_PREFIXES = ("CARGO_LLVM_COV", "LLVM_")
    COV_ENV_EXTRAS = ("CARGO_ENCODED_RUSTFLAGS", "CARGO_INCREMENTAL")
    for key, val in os.environ.items():
        if key.startswith(COV_ENV_PREFIXES) or key in COV_ENV_EXTRAS:
            env[key] = val
    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary, "--no-onboard",
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
        # Dump stderr so CI logs show why the server failed to start
        returncode = proc.returncode
        stderr_bytes = b""
        if proc.stderr:
            try:
                stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
            except (asyncio.TimeoutError, Exception):
                pass
        stderr_text = stderr_bytes.decode("utf-8", errors="replace")
        proc.kill()
        pytest.fail(
            f"ironclaw server failed to start on port {gateway_port} "
            f"(returncode={returncode}).\nstderr:\n{stderr_text}"
        )
    finally:
        if proc.returncode is None:
            # Use SIGINT (not SIGTERM) so tokio's ctrl_c handler triggers a
            # graceful shutdown.  This lets the LLVM coverage runtime run its
            # atexit handler and flush .profraw files for cargo-llvm-cov.
            proc.send_signal(signal.SIGINT)
            try:
                await asyncio.wait_for(proc.wait(), timeout=10)
            except asyncio.TimeoutError:
                proc.kill()


@pytest.fixture(scope="session")
async def browser(ironclaw_server):
    """Session-scoped Playwright browser instance.

    Reuses a single browser process across all tests. Individual tests
    get isolated contexts via the ``page`` fixture.
    """
    from playwright.async_api import async_playwright

    headless = os.environ.get("HEADED", "").strip() not in ("1", "true")
    async with async_playwright() as p:
        b = await p.chromium.launch(headless=headless)
        yield b
        await b.close()


@pytest.fixture
async def page(ironclaw_server, browser):
    """Fresh Playwright browser context + page, navigated to the gateway with auth."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    pg = await context.new_page()
    await pg.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}")
    # Wait for the app to initialize (auth screen hidden, SSE connected)
    await pg.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    yield pg
    await context.close()
