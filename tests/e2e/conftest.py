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

from helpers import (
    AUTH_TOKEN,
    HTTP_WEBHOOK_SECRET,
    OWNER_SCOPE_ID,
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

# Temp directory for the libSQL database file (cleaned up automatically)
_DB_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-")

# Temp HOME so pairing/allowFrom state never touches the developer's real ~/.ironclaw
_HOME_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-home-")

# Temp directories for WASM extensions. These start empty and are populated by
# the install pipeline during tests; fixtures do not pre-populate dev build
# artifacts into them.
_WASM_TOOLS_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-wasm-tools-")
_WASM_CHANNELS_TMPDIR = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-wasm-channels-")


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
    if proc.returncode is not None:
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


def _forward_coverage_env(env: dict[str, str]) -> None:
    """Forward cargo-llvm-cov env vars into child processes when present."""
    cov_env_prefixes = ("CARGO_LLVM_COV", "LLVM_")
    cov_env_extras = ("CARGO_ENCODED_RUSTFLAGS", "CARGO_INCREMENTAL")
    for key, val in os.environ.items():
        if key.startswith(cov_env_prefixes) or key in cov_env_extras:
            env[key] = val


def _build_gateway_env(
    *,
    mock_llm_server: str,
    wasm_tools_dir: str,
    home_dir: str,
    gateway_port: int,
    http_port: int,
    db_path: str,
    extra_env: dict[str, str] | None = None,
) -> dict[str, str]:
    """Build a deterministic env block for an isolated gateway instance."""
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": db_path,
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "true",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "true",
        "WASM_TOOLS_DIR": wasm_tools_dir,
        "WASM_CHANNELS_DIR": _WASM_CHANNELS_TMPDIR.name,
        "ONBOARD_COMPLETED": "true",
        "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
        "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
    }
    if extra_env:
        env.update(extra_env)
    _forward_coverage_env(env)
    return env


class ManagedIronclawServer:
    """Restartable ironclaw process wrapper for E2E scenarios."""

    def __init__(
        self,
        *,
        binary: str,
        env: dict[str, str],
        gateway_port: int,
        label: str,
    ):
        self.binary = binary
        self.env = env
        self.gateway_port = gateway_port
        self.label = label
        self.base_url = f"http://127.0.0.1:{gateway_port}"
        self.proc: asyncio.subprocess.Process | None = None

    async def start(self) -> None:
        """Start the gateway and wait for `/api/health`."""
        if self.proc and self.proc.returncode is None:
            return

        self.proc = await asyncio.create_subprocess_exec(
            self.binary,
            "--no-onboard",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=self.env,
        )
        try:
            await wait_for_ready(f"{self.base_url}/api/health", timeout=60)
        except TimeoutError:
            proc = self.proc
            if proc and proc.returncode is None:
                await _stop_process(proc, timeout=2)
            returncode = proc.returncode if proc else None
            stderr_bytes = b""
            if proc and proc.stderr:
                try:
                    stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
                except asyncio.TimeoutError:
                    pass
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            pytest.fail(
                f"{self.label} failed to start on port {self.gateway_port} "
                f"(returncode={returncode}).\nstderr:\n{stderr_text}"
            )

    async def stop(self) -> None:
        """Gracefully stop the gateway if it is still running."""
        proc = self.proc
        if proc is None or proc.returncode is not None:
            return
        await _stop_process(proc, sig=signal.SIGINT, timeout=10)
        if proc.returncode is None:
            await _stop_process(proc, timeout=2)

    async def restart(self) -> None:
        """Restart the gateway on the same port, DB, and HOME."""
        await self.stop()
        await self.start()

    async def close(self) -> None:
        """Stop the gateway and release process resources."""
        await self.stop()


@pytest.fixture(scope="session")
def ironclaw_binary():
    """Ensure ironclaw binary is built. Returns the binary path."""
    target_dir = _cargo_target_dir()
    binary = target_dir / "debug" / "ironclaw"
    if _binary_needs_rebuild(binary):
        print("Building ironclaw (this may take a while)...")
        subprocess.run(
            ["cargo", "build", "--no-default-features", "--features", "libsql"],
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
def server_ports():
    """Reserve dynamic ports for the gateway and HTTP webhook channel."""
    reserved = _reserve_loopback_sockets(2)
    try:
        yield {
            "gateway": reserved[0].getsockname()[1],
            "http": reserved[1].getsockname()[1],
            "sockets": reserved,
        }
    finally:
        for sock in reserved:
            sock.close()


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
async def ironclaw_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    server_ports,
):
    """Start the ironclaw gateway. Yields the base URL."""
    home_dir = _HOME_TMPDIR.name
    gateway_port = server_ports["gateway"]
    http_port = server_ports["http"]
    for sock in server_ports["sockets"]:
        if sock.fileno() != -1:
            sock.close()
    env = {
        # Minimal env: PATH for process spawning, HOME for Rust/cargo defaults
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_DB_TMPDIR.name, "e2e.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "true",
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
    _forward_coverage_env(env)
    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary, "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )
    startup_kill_attempted = False
    base_url = f"http://127.0.0.1:{gateway_port}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=60)
        yield base_url
    except TimeoutError:
        # Dump stderr so CI logs show why the server failed to start
        if proc.returncode is None:
            startup_kill_attempted = True
            await _stop_process(proc, timeout=2)
        returncode = proc.returncode
        stderr_bytes = b""
        if proc.stderr:
            try:
                stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
            except asyncio.TimeoutError:
                pass
        stderr_text = stderr_bytes.decode("utf-8", errors="replace")
        pytest.fail(
            f"ironclaw server failed to start on port {gateway_port} "
            f"(returncode={returncode}).\nstderr:\n{stderr_text}"
        )
    finally:
        if proc.returncode is None:
            if startup_kill_attempted:
                await _stop_process(proc, timeout=2)
            else:
                # Use SIGINT (not SIGTERM) so tokio's ctrl_c handler triggers a
                # graceful shutdown.  This lets the LLVM coverage runtime run its
                # atexit handler and flush .profraw files for cargo-llvm-cov.
                await _stop_process(proc, sig=signal.SIGINT, timeout=10)
                if proc.returncode is None:
                    await _stop_process(proc, timeout=2)


@pytest.fixture(scope="session")
async def hosted_oauth_refresh_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
):
    """Start a hosted-mode ironclaw instance for OAuth refresh regression tests."""
    reserved = _reserve_loopback_sockets(2)
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-hosted-oauth-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-hosted-oauth-home-")

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()

        db_path = os.path.join(db_tmpdir.name, "hosted-oauth-refresh.db")
        home_dir = home_tmpdir.name
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": home_dir,
            "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
            "RUST_LOG": "ironclaw=info",
            "RUST_BACKTRACE": "1",
            "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
            "GATEWAY_ENABLED": "true",
            "GATEWAY_HOST": "127.0.0.1",
            "GATEWAY_PORT": str(gateway_port),
            "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
            "HTTP_HOST": "127.0.0.1",
            "HTTP_PORT": str(http_port),
            "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
            "CLI_ENABLED": "false",
            "LLM_BACKEND": "openai_compatible",
            "LLM_BASE_URL": mock_llm_server,
            "LLM_MODEL": "mock-model",
            "DATABASE_BACKEND": "libsql",
            "LIBSQL_PATH": db_path,
            "SECRETS_MASTER_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "SANDBOX_ENABLED": "false",
            "SKILLS_ENABLED": "true",
            "ROUTINES_ENABLED": "true",
            "HEARTBEAT_ENABLED": "false",
            "EMBEDDING_ENABLED": "false",
            "WASM_ENABLED": "true",
            "WASM_TOOLS_DIR": wasm_tools_dir,
            "WASM_CHANNELS_DIR": _WASM_CHANNELS_TMPDIR.name,
            "ONBOARD_COMPLETED": "true",
            "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
            "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
            "GOOGLE_OAUTH_CLIENT_ID": "hosted-google-client-id",
        }
        _forward_coverage_env(env)

        proc = await asyncio.create_subprocess_exec(
            ironclaw_binary, "--no-onboard",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env,
        )
        startup_kill_attempted = False
        base_url = f"http://127.0.0.1:{gateway_port}"
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            yield {
                "base_url": base_url,
                "db_path": db_path,
                "mock_llm_url": mock_llm_server,
            }
        except TimeoutError:
            if proc.returncode is None:
                startup_kill_attempted = True
                await _stop_process(proc, timeout=2)
            returncode = proc.returncode
            stderr_bytes = b""
            if proc.stderr:
                try:
                    stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
                except asyncio.TimeoutError:
                    pass
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            pytest.fail(
                f"hosted oauth refresh server failed to start on port {gateway_port} "
                f"(returncode={returncode}).\nstderr:\n{stderr_text}"
            )
        finally:
            if proc.returncode is None:
                if startup_kill_attempted:
                    await _stop_process(proc, timeout=2)
                else:
                    await _stop_process(proc, sig=signal.SIGINT, timeout=10)
                    if proc.returncode is None:
                        await _stop_process(proc, timeout=2)
    finally:
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()


@pytest.fixture(scope="session")
async def loop_limited_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
):
    """Start an isolated ironclaw instance with a low tool-iteration limit."""
    reserved = _reserve_loopback_sockets(2)
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-loop-limit-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-loop-limit-home-")

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": home_tmpdir.name,
            "IRONCLAW_BASE_DIR": os.path.join(home_tmpdir.name, ".ironclaw"),
            "RUST_LOG": "ironclaw=info",
            "RUST_BACKTRACE": "1",
            "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
            "GATEWAY_ENABLED": "true",
            "GATEWAY_HOST": "127.0.0.1",
            "GATEWAY_PORT": str(gateway_port),
            "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
            "HTTP_HOST": "127.0.0.1",
            "HTTP_PORT": str(http_port),
            "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
            "CLI_ENABLED": "false",
            "LLM_BACKEND": "openai_compatible",
            "LLM_BASE_URL": mock_llm_server,
            "LLM_MODEL": "mock-model",
            "DATABASE_BACKEND": "libsql",
            "LIBSQL_PATH": os.path.join(db_tmpdir.name, "loop-limited.db"),
            "SANDBOX_ENABLED": "false",
            "SKILLS_ENABLED": "true",
            "ROUTINES_ENABLED": "true",
            "HEARTBEAT_ENABLED": "false",
            "EMBEDDING_ENABLED": "false",
            "WASM_ENABLED": "true",
            "WASM_TOOLS_DIR": wasm_tools_dir,
            "WASM_CHANNELS_DIR": _WASM_CHANNELS_TMPDIR.name,
            "ONBOARD_COMPLETED": "true",
            "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
            "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
            "AGENT_MAX_TOOL_ITERATIONS": "2",
        }
        _forward_coverage_env(env)

        proc = await asyncio.create_subprocess_exec(
            ironclaw_binary, "--no-onboard",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env,
        )
        startup_kill_attempted = False
        base_url = f"http://127.0.0.1:{gateway_port}"
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            yield base_url
        except TimeoutError:
            if proc.returncode is None:
                startup_kill_attempted = True
                await _stop_process(proc, timeout=2)
            returncode = proc.returncode
            stderr_bytes = b""
            if proc.stderr:
                try:
                    stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
                except asyncio.TimeoutError:
                    pass
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            pytest.fail(
                f"loop-limited ironclaw server failed to start on port {gateway_port} "
                f"(returncode={returncode}).\nstderr:\n{stderr_text}"
            )
        finally:
            if proc.returncode is None:
                if startup_kill_attempted:
                    await _stop_process(proc, timeout=2)
                else:
                    await _stop_process(proc, sig=signal.SIGINT, timeout=10)
                    if proc.returncode is None:
                        await _stop_process(proc, timeout=2)
    finally:
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()


@pytest.fixture(scope="session")
async def length_preserving_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
):
    """Start an isolated ironclaw instance using the NearAI provider path."""
    reserved = _reserve_loopback_sockets(2)
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-length-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-length-home-")

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": home_tmpdir.name,
            "IRONCLAW_BASE_DIR": os.path.join(home_tmpdir.name, ".ironclaw"),
            "RUST_LOG": "ironclaw=info",
            "RUST_BACKTRACE": "1",
            "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
            "GATEWAY_ENABLED": "true",
            "GATEWAY_HOST": "127.0.0.1",
            "GATEWAY_PORT": str(gateway_port),
            "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
            "HTTP_HOST": "127.0.0.1",
            "HTTP_PORT": str(http_port),
            "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
            "CLI_ENABLED": "false",
            "LLM_BACKEND": "nearai",
            "NEARAI_BASE_URL": mock_llm_server,
            "NEARAI_MODEL": "mock-model",
            "NEARAI_API_KEY": "mock-nearai-key",
            "DATABASE_BACKEND": "libsql",
            "LIBSQL_PATH": os.path.join(db_tmpdir.name, "length-preserving.db"),
            "SANDBOX_ENABLED": "false",
            "SKILLS_ENABLED": "true",
            "ROUTINES_ENABLED": "true",
            "HEARTBEAT_ENABLED": "false",
            "EMBEDDING_ENABLED": "false",
            "WASM_ENABLED": "true",
            "WASM_TOOLS_DIR": wasm_tools_dir,
            "WASM_CHANNELS_DIR": _WASM_CHANNELS_TMPDIR.name,
            "ONBOARD_COMPLETED": "true",
            "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
            "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
        }
        _forward_coverage_env(env)

        proc = await asyncio.create_subprocess_exec(
            ironclaw_binary, "--no-onboard",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env,
        )
        startup_kill_attempted = False
        base_url = f"http://127.0.0.1:{gateway_port}"
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            yield base_url
        except TimeoutError:
            if proc.returncode is None:
                startup_kill_attempted = True
                await _stop_process(proc, timeout=2)
            returncode = proc.returncode
            stderr_bytes = b""
            if proc.stderr:
                try:
                    stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
                except asyncio.TimeoutError:
                    pass
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            pytest.fail(
                f"length-preserving ironclaw server failed to start on port {gateway_port} "
                f"(returncode={returncode}).\nstderr:\n{stderr_text}"
            )
        finally:
            if proc.returncode is None:
                if startup_kill_attempted:
                    await _stop_process(proc, timeout=2)
                else:
                    await _stop_process(proc, sig=signal.SIGINT, timeout=10)
                    if proc.returncode is None:
                        await _stop_process(proc, timeout=2)
    finally:
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()


@pytest.fixture(scope="session")
async def extension_cleanup_server(
    ironclaw_binary,
    mock_llm_server,
):
    """Start an isolated ironclaw instance for uninstall secret cleanup E2E tests."""
    reserved = _reserve_loopback_sockets(2)
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-cleanup-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-cleanup-home-")
    tools_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-cleanup-tools-")
    channels_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-cleanup-channels-")

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()

        db_path = os.path.join(db_tmpdir.name, "extension-cleanup.db")
        home_dir = home_tmpdir.name
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": home_dir,
            "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
            "RUST_LOG": "ironclaw=info",
            "RUST_BACKTRACE": "1",
            "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
            "GATEWAY_ENABLED": "true",
            "GATEWAY_HOST": "127.0.0.1",
            "GATEWAY_PORT": str(gateway_port),
            "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
            "GATEWAY_USER_ID": OWNER_SCOPE_ID,
            "HTTP_HOST": "127.0.0.1",
            "HTTP_PORT": str(http_port),
            "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
            "CLI_ENABLED": "false",
            "LLM_BACKEND": "openai_compatible",
            "LLM_BASE_URL": mock_llm_server,
            "LLM_MODEL": "mock-model",
            "DATABASE_BACKEND": "libsql",
            "LIBSQL_PATH": db_path,
            "SECRETS_MASTER_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "SANDBOX_ENABLED": "false",
            "SKILLS_ENABLED": "true",
            "ROUTINES_ENABLED": "true",
            "HEARTBEAT_ENABLED": "false",
            "EMBEDDING_ENABLED": "false",
            "WASM_ENABLED": "true",
            "WASM_TOOLS_DIR": tools_tmpdir.name,
            "WASM_CHANNELS_DIR": channels_tmpdir.name,
            "ONBOARD_COMPLETED": "true",
            "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
            "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
            "GOOGLE_OAUTH_CLIENT_ID": "hosted-google-client-id",
        }
        _forward_coverage_env(env)

        proc = await asyncio.create_subprocess_exec(
            ironclaw_binary, "--no-onboard",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env,
        )
        startup_kill_attempted = False
        base_url = f"http://127.0.0.1:{gateway_port}"
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            yield {
                "base_url": base_url,
                "db_path": db_path,
                "gateway_user_id": OWNER_SCOPE_ID,
                "mock_llm_url": mock_llm_server,
            }
        except TimeoutError:
            if proc.returncode is None:
                startup_kill_attempted = True
                await _stop_process(proc, timeout=2)
            returncode = proc.returncode
            stderr_bytes = b""
            if proc.stderr:
                try:
                    stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
                except asyncio.TimeoutError:
                    pass
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            pytest.fail(
                f"extension cleanup server failed to start on port {gateway_port} "
                f"(returncode={returncode}).\nstderr:\n{stderr_text}"
            )
        finally:
            if proc.returncode is None:
                if startup_kill_attempted:
                    await _stop_process(proc, timeout=2)
                else:
                    await _stop_process(proc, sig=signal.SIGINT, timeout=10)
                    if proc.returncode is None:
                        await _stop_process(proc, timeout=2)
    finally:
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()
        tools_tmpdir.cleanup()
        channels_tmpdir.cleanup()


@pytest.fixture
async def managed_gateway_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
):
    """Start an isolated, restartable gateway instance for SSE/connectivity tests."""
    reserved = _reserve_loopback_sockets(2)
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-managed-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-managed-home-")
    server = None

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()

        env = _build_gateway_env(
            mock_llm_server=mock_llm_server,
            wasm_tools_dir=wasm_tools_dir,
            home_dir=home_tmpdir.name,
            gateway_port=gateway_port,
            http_port=http_port,
            db_path=os.path.join(db_tmpdir.name, "managed-gateway.db"),
        )
        server = ManagedIronclawServer(
            binary=ironclaw_binary,
            env=env,
            gateway_port=gateway_port,
            label="managed gateway server",
        )
        await server.start()
        yield server
    finally:
        if server is not None:
            await server.close()
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()


@pytest.fixture
async def limited_gateway_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
):
    """Start an isolated gateway with a low SSE/WebSocket connection cap."""
    reserved = _reserve_loopback_sockets(2)
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-limited-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-limited-home-")
    server = None

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()

        env = _build_gateway_env(
            mock_llm_server=mock_llm_server,
            wasm_tools_dir=wasm_tools_dir,
            home_dir=home_tmpdir.name,
            gateway_port=gateway_port,
            http_port=http_port,
            db_path=os.path.join(db_tmpdir.name, "limited-gateway.db"),
            extra_env={"GATEWAY_MAX_CONNECTIONS": "2"},
        )
        server = ManagedIronclawServer(
            binary=ironclaw_binary,
            env=env,
            gateway_port=gateway_port,
            label="limited gateway server",
        )
        await server.start()
        yield server
    finally:
        if server is not None:
            await server.close()
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()


@pytest.fixture(scope="session")
async def http_channel_server(ironclaw_server, server_ports):
    """HTTP webhook channel base URL."""
    base_url = f"http://127.0.0.1:{server_ports['http']}"
    await wait_for_ready(f"{base_url}/health", timeout=30)
    return base_url


@pytest.fixture(scope="session")
async def http_channel_server_without_secret(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
):
    """Start the HTTP webhook channel without a configured secret."""
    gateway_port = _find_free_port()
    http_port = _find_free_port()
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
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(_DB_TMPDIR.name, "e2e-webhook-no-secret.db"),
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
    _forward_coverage_env(env)
    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary, "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )
    startup_kill_attempted = False
    gateway_url = f"http://127.0.0.1:{gateway_port}"
    http_base_url = f"http://127.0.0.1:{http_port}"
    try:
        await wait_for_ready(f"{gateway_url}/api/health", timeout=60)
        await wait_for_ready(f"{http_base_url}/health", timeout=30)
        yield http_base_url
    except TimeoutError:
        # Dump stderr so CI logs show why the server failed to start
        if proc.returncode is None:
            startup_kill_attempted = True
            await _stop_process(proc, timeout=2)
        returncode = proc.returncode
        stderr_bytes = b""
        if proc.stderr:
            try:
                stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
            except asyncio.TimeoutError:
                pass
        stderr_text = stderr_bytes.decode("utf-8", errors="replace")
        pytest.fail(
            f"ironclaw server without webhook secret failed to start on ports "
            f"gateway={gateway_port}, http={http_port} "
            f"(returncode={returncode}).\nstderr:\n{stderr_text}"
        )
    finally:
        if proc.returncode is None:
            if startup_kill_attempted:
                await _stop_process(proc, timeout=2)
            else:
                # Use SIGINT (not SIGTERM) so tokio's ctrl_c handler triggers a
                # graceful shutdown.  This lets the LLVM coverage runtime run its
                # atexit handler and flush .profraw files for cargo-llvm-cov.
                await _stop_process(proc, sig=signal.SIGINT, timeout=10)
                if proc.returncode is None:
                    await _stop_process(proc, timeout=2)


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


@pytest.fixture
async def loop_limited_page(loop_limited_server, browser):
    """Fresh Playwright page bound to the low-iteration gateway fixture."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    pg = await context.new_page()
    await pg.goto(f"{loop_limited_server}/?token={AUTH_TOKEN}")
    await pg.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    yield pg
    await context.close()


@pytest.fixture
async def length_preserving_page(length_preserving_server, browser):
    """Fresh Playwright page bound to the length-preserving gateway fixture."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    pg = await context.new_page()
    await pg.goto(f"{length_preserving_server}/?token={AUTH_TOKEN}")
    await pg.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    yield pg
    await context.close()
