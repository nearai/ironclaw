"""pytest fixtures for E2E tests.

Session-scoped: build binary, start mock LLM, start ironclaw, launch browser.
Function-scoped: fresh browser context and page per test.
"""

import asyncio
from collections.abc import AsyncIterator
from contextlib import AsyncExitStack, aclosing, asynccontextmanager
import json
import os
import signal
import shutil
import socket
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

import httpx
import pytest

from helpers import (
    AUTH_TOKEN,
    EMULATE_GITHUB_BEARER,
    EMULATE_GOOGLE_BEARER,
    EMULATE_SLACK_BEARER,
    HTTP_WEBHOOK_SECRET,
    OWNER_SCOPE_ID,
    wait_for_port_line,
    wait_for_ready,
)
from provider_fault_proxy import ProviderFaultProxyWorld

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

EMULATE_NPM_PACKAGE = "emulate@0.7.0"
EMULATE_CLI_PATH = os.environ.get("IRONCLAW_EMULATE_CLI")
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
    proc: asyncio.subprocess.Process,
    *,
    sig: int | None = None,
    timeout: float,
    process_group: bool = False,
) -> None:
    """Signal a subprocess and wait briefly without masking exit races."""
    async def _drain_pipes() -> None:
        try:
            await asyncio.wait_for(proc.communicate(), timeout=1)
        except (asyncio.TimeoutError, ValueError):
            pass

    signal_to_send = signal.SIGKILL if sig is None else sig
    try:
        if process_group:
            os.killpg(proc.pid, signal_to_send)
        elif sig is None:
            proc.kill()
        else:
            proc.send_signal(sig)
    except ProcessLookupError:
        try:
            await asyncio.wait_for(proc.wait(), timeout=timeout)
        except asyncio.TimeoutError:
            pass
        return

    if proc.returncode is None:
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

    Mirrors the `ironclaw_binary` staleness pattern: rebuild only the tools
    whose manifest/schema/prompt/wasm-src changed since their zip was last
    built, via `scripts/build-test-tools.sh`.
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
        "GATEWAY_USER_ID": OWNER_SCOPE_ID,
        "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_API_KEY": "mock-api-key",
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
        "SKILLS_DIR": str(ROOT / "skills"),
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
    binary = target_dir / "debug" / "ironclaw-legacy"
    if _binary_needs_rebuild(binary):
        print("Building ironclaw (this may take a while)...")
        subprocess.run(
            [
                "cargo", "build",
                "-p", "ironclaw_legacy",
                "--bin", "ironclaw-legacy",
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
def ironclaw_reborn_binary():
    """Ensure the Reborn `ironclaw` binary is built with the WebChat v2 surface.

    Distinct from `ironclaw_binary` (the legacy `ironclaw` web channel): this is
    the Reborn CLI, whose WebUI v2 SPA and `serve` subcommand are compiled
    unconditionally. Returns the binary path. Used by the Reborn WebUI v2 smoke
    scenario.
    """
    target_dir = _cargo_target_dir()
    binary = target_dir / "debug" / "ironclaw"
    if _binary_needs_rebuild(binary):
        print("Building Reborn ironclaw (this may take a while)...")
        subprocess.run(
            [
                "cargo", "build",
                "-p", "ironclaw",
                "--bin", "ironclaw",
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
    """Ensure Reborn `ironclaw` is built for the OpenAI-compatible scenarios.

    The OpenAI-compatible routes are compiled unconditionally, so this builds
    the same binary as the generic Reborn WebUI fixture. It is kept separate so
    the OpenAI-compatible E2E owns its own build/staleness stamp.
    """
    target_dir = _cargo_target_dir()
    binary = target_dir / "debug" / "ironclaw"
    stamp = target_dir / "debug" / ".ironclaw-reborn-openai-compat.stamp"
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
        print("Building Reborn ironclaw (OpenAI-compatible E2E; this may take a while)...")
        subprocess.run(
            [
                "cargo", "build",
                "-p", "ironclaw",
                "--bin", "ironclaw",
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


async def _run_emulate_server(
    *,
    service: str,
    seed_path: Path,
    ready_method: str,
    ready_path: str,
    ready_headers: dict[str, str],
    ready_json: dict[str, Any] | None = None,
    port: int | None = None,
) -> AsyncIterator[dict[str, str]]:
    """Start a pinned Emulate service and wait for a seeded endpoint."""
    if EMULATE_CLI_PATH:
        emulate_cli = Path(EMULATE_CLI_PATH)
        if not emulate_cli.is_file():
            _emulate_unavailable(
                f"IRONCLAW_EMULATE_CLI does not exist: {emulate_cli}"
            )
        if shutil.which("node") is None:
            _emulate_unavailable(
                f"node is required to run the Emulate {service} E2E fixture"
            )
        command = ["node", str(emulate_cli)]
    elif shutil.which("npx") is None:
        _emulate_unavailable(
            f"npx is required to run the Emulate {service} E2E fixture"
        )
    else:
        command = ["npx", "--yes", EMULATE_NPM_PACKAGE]

    port = port or _find_free_port()
    url = f"http://127.0.0.1:{port}"
    env = {
        **os.environ,
        "NO_COLOR": "1",
        "EMULATE_PORT": str(port),
    }
    proc = await asyncio.create_subprocess_exec(
        *command,
        "--service",
        service,
        "--port",
        str(port),
        "--seed",
        str(seed_path),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
        start_new_session=True,
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
            await _stop_process(
                proc, sig=signal.SIGINT, timeout=5, process_group=True
            )
        await _stop_process(proc, timeout=2, process_group=True)


async def _bootstrap_emulate_github_access(
    client: httpx.AsyncClient, url: str
) -> None:
    """Grant the primary fixture actor write access to the seeded org repo."""
    headers = {"Authorization": f"Bearer {EMULATE_GITHUB_READY_TOKEN}"}
    team = await client.post(
        f"{url}/orgs/nearai/teams",
        headers=headers,
        json={
            "name": "Provider Contract",
            "permission": "push",
            "privacy": "closed",
        },
        timeout=5,
    )
    if team.status_code != 201:
        raise RuntimeError(
            "Failed to create Emulate GitHub provider team: "
            f"{team.status_code} {team.text[:400]}"
        )
    membership = await client.put(
        f"{url}/orgs/nearai/teams/provider-contract/memberships/reborn-dev",
        headers=headers,
        json={"role": "maintainer"},
        timeout=5,
    )
    if membership.status_code != 200:
        raise RuntimeError(
            "Failed to grant Emulate GitHub provider membership: "
            f"{membership.status_code} {membership.text[:400]}"
        )


@asynccontextmanager
async def _emulate_service(
    *,
    service: str,
    seed_path: Path,
    ready_method: str,
    ready_path: str,
    ready_headers: dict[str, str],
    port: int | None = None,
) -> AsyncIterator[dict[str, str]]:
    generator = _run_emulate_server(
        service=service,
        seed_path=seed_path,
        ready_method=ready_method,
        ready_path=ready_path,
        ready_headers=ready_headers,
        port=port,
    )
    async with aclosing(generator):
        async for server in generator:
            yield server


@asynccontextmanager
async def _emulate_github_service(
    *, port: int | None = None
) -> AsyncIterator[dict[str, str]]:
    """Start GitHub Emulate and grant the seeded actor repository access."""
    async with _emulate_service(
        service="github",
        seed_path=EMULATE_GITHUB_SEED,
        ready_method="GET",
        ready_path="/user",
        ready_headers={
            "Authorization": f"Bearer {EMULATE_GITHUB_READY_TOKEN}",
        },
        port=port,
    ) as server:
        async with httpx.AsyncClient() as client:
            await _bootstrap_emulate_github_access(client, server["url"])
        yield server


class ResettableEmulateProviderWorld:
    """Restart seeded provider processes on stable ports between journeys."""

    def __init__(self) -> None:
        reserved = _reserve_loopback_sockets(3)
        services = ("google", "slack", "github")
        self._ports = {
            service: sock.getsockname()[1]
            for service, sock in zip(services, reserved, strict=True)
        }
        self._reservations = dict(zip(services, reserved, strict=True))
        self._stacks: dict[str, AsyncExitStack] = {}

    @property
    def servers(self) -> dict[str, dict[str, str]]:
        return {
            service: {"url": f"http://127.0.0.1:{port}"}
            for service, port in self._ports.items()
        }

    async def start(self, services: set[str] | None = None) -> None:
        selected = set(self._ports) if services is None else services
        for service in sorted(selected):
            if service in self._stacks:
                raise RuntimeError(f"Emulate {service} provider is already running")
            await self._start_service(service)

    async def _start_service(self, service: str) -> None:
        stack = AsyncExitStack()
        try:
            if service == "google":
                context = _emulate_service(
                    service=service,
                    seed_path=EMULATE_GOOGLE_SEED,
                    ready_method="GET",
                    ready_path="/gmail/v1/users/me/messages",
                    ready_headers={
                        "Authorization": f"Bearer {EMULATE_GOOGLE_READY_TOKEN}",
                    },
                    port=self._ports[service],
                )
            elif service == "slack":
                context = _emulate_service(
                    service=service,
                    seed_path=EMULATE_SLACK_SEED,
                    ready_method="POST",
                    ready_path="/api/auth.test",
                    ready_headers={
                        "Authorization": f"Bearer {EMULATE_SLACK_READY_TOKEN}",
                    },
                    port=self._ports[service],
                )
            elif service == "github":
                context = _emulate_github_service(port=self._ports[service])
            else:
                raise ValueError(f"Unknown Emulate provider service: {service}")
            reservation = self._reservations.pop(service, None)
            if reservation is not None:
                reservation.close()
            await stack.enter_async_context(context)
        except BaseException:
            await stack.aclose()
            raise
        self._stacks[service] = stack

    async def reset(self, services: set[str]) -> None:
        await self.close(services, reserve=True)
        await self.start(services)

    async def close(
        self,
        services: set[str] | None = None,
        *,
        reserve: bool = False,
    ) -> None:
        selected = set(self._stacks) if services is None else services
        for service in sorted(selected, reverse=True):
            stack = self._stacks.pop(service, None)
            if stack is not None:
                await stack.aclose()
            if reserve:
                self._reservations[service] = await self._reserve_service_port(
                    service
                )
        if services is None:
            reservations, self._reservations = self._reservations, {}
            for sock in reservations.values():
                sock.close()

    async def _reserve_service_port(self, service: str) -> socket.socket:
        last_error: OSError | None = None
        for _ in range(20):
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            try:
                sock.bind(("127.0.0.1", self._ports[service]))
                return sock
            except OSError as exc:
                sock.close()
                last_error = exc
                await asyncio.sleep(0.1)
        raise last_error or OSError(
            f"could not reserve Emulate {service} port {self._ports[service]}"
        )


@pytest.fixture(scope="module")
async def resettable_emulate_provider_world():
    """Keep stable provider URLs while restoring seed state per journey."""
    world = ResettableEmulateProviderWorld()
    try:
        await world.start()
        yield world
    finally:
        await world.close()


@pytest.fixture(scope="module")
async def provider_fault_proxy_world(resettable_emulate_provider_world):
    """Proxy stable Emulate worlds through independently resettable faults."""
    world = ProviderFaultProxyWorld(
        {
            service: server["url"]
            for service, server in resettable_emulate_provider_world.servers.items()
        }
    )
    try:
        await world.start()
        yield world
    finally:
        await world.close()


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
    async with _emulate_github_service() as server:
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
            f"{mock_llm_server}/__mock/llm_trace/reset",
            timeout=10,
        )
        response.raise_for_status()
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_faults/reset",
            timeout=10,
        )
        response.raise_for_status()
        response = await client.post(
            f"{mock_llm_server}/__mock/capability_policy/reset",
            timeout=10,
        )
        if response.status_code != 404:
            response.raise_for_status()


@pytest.fixture(autouse=True)
def reset_fake_telegram_state(request):
    """Reset fake Telegram API state after Telegram-dependent tests."""
    yield
    if (
        "telegram_e2e_server" not in request.fixturenames
        and "isolated_telegram_e2e_server" not in request.fixturenames
        and "telegram_e2e_server_with_routines" not in request.fixturenames
        and "fake_telegram_server" not in request.fixturenames
    ):
        return
    fake_tg_url = request.getfixturevalue("fake_telegram_server")
    with httpx.Client() as client:
        client.post(f"{fake_tg_url}/__mock/reset", timeout=10)


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
        "GATEWAY_USER_ID": OWNER_SCOPE_ID,
        "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "HTTP_WEBHOOK_SECRET": HTTP_WEBHOOK_SECRET,
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_API_KEY": "mock-api-key",
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
    env["SECRETS_MASTER_KEY"] = (
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    )
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


async def _run_hosted_oauth_refresh_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    *,
    extra_env: dict[str, str] | None = None,
    extra_result: dict[str, str] | None = None,
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
            "LLM_API_KEY": "mock-api-key",
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
            "IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK": "1",
            "GOOGLE_OAUTH_CLIENT_ID": "hosted-google-client-id",
        }
        if extra_env:
            env.update(extra_env)
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
                "wasm_tools_dir": wasm_tools_dir,
                **(extra_result or {}),
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
async def hosted_oauth_refresh_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
):
    """Start hosted mode for OAuth refresh tests that do not need provider APIs."""
    async for server in _run_hosted_oauth_refresh_server(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
    ):
        yield server


@pytest.fixture(scope="session")
async def hosted_google_emulate_server(
    ironclaw_binary,
    mock_llm_server,
    emulate_google_server,
    wasm_tools_dir,
):
    """Start hosted mode with Google provider API traffic rewritten to Emulate."""
    rewrite_map = {
        "gmail.googleapis.com": emulate_google_server["url"],
        "www.googleapis.com": emulate_google_server["url"],
        "slides.googleapis.com": emulate_google_server["url"],
    }
    async for server in _run_hosted_oauth_refresh_server(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
        extra_env={"IRONCLAW_TEST_HTTP_REWRITE_MAP": json.dumps(rewrite_map)},
        extra_result={"emulate_google_url": emulate_google_server["url"]},
    ):
        yield server


@pytest.fixture(scope="session")
async def hosted_github_emulate_server(
    ironclaw_binary,
    mock_llm_server,
    emulate_github_server,
    wasm_tools_dir,
):
    """Start hosted mode with GitHub provider API traffic rewritten to Emulate."""
    rewrite_map = {"api.github.com": emulate_github_server["url"]}
    async for server in _run_hosted_oauth_refresh_server(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
        extra_env={
            "IRONCLAW_TEST_HTTP_REWRITE_MAP": json.dumps(rewrite_map),
            "GITHUB_OAUTH_CLIENT_ID": "hosted-github-client-id",
            "GITHUB_OAUTH_CLIENT_SECRET": "hosted-github-client-secret",
        },
        extra_result={"emulate_github_url": emulate_github_server["url"]},
    ):
        yield server


@pytest.fixture(scope="session")
async def hosted_provider_emulate_server(
    ironclaw_binary,
    mock_llm_server,
    emulate_google_server,
    emulate_github_server,
    emulate_slack_server,
    wasm_tools_dir,
):
    """Start hosted mode with Google, GitHub, and Slack routed to Emulate."""
    rewrite_map = {
        "gmail.googleapis.com": emulate_google_server["url"],
        "www.googleapis.com": emulate_google_server["url"],
        "slides.googleapis.com": emulate_google_server["url"],
        "api.github.com": emulate_github_server["url"],
        "slack.com": emulate_slack_server["url"],
    }
    async for server in _run_hosted_oauth_refresh_server(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
        extra_env={
            "IRONCLAW_TEST_HTTP_REWRITE_MAP": json.dumps(rewrite_map),
            "GITHUB_OAUTH_CLIENT_ID": "hosted-github-client-id",
            "GITHUB_OAUTH_CLIENT_SECRET": "hosted-github-client-secret",
            "SLACK_OAUTH_CLIENT_ID": "hosted-slack-client-id",
            "SLACK_OAUTH_CLIENT_SECRET": "hosted-slack-client-secret",
        },
        extra_result={
            "emulate_google_url": emulate_google_server["url"],
            "emulate_github_url": emulate_github_server["url"],
            "emulate_slack_url": emulate_slack_server["url"],
        },
    ):
        yield server


@pytest.fixture(scope="session")
async def hosted_google_oauth_refresh_server(hosted_google_emulate_server):
    """Compatibility fixture for hosted Gmail OAuth refresh regression tests."""
    yield hosted_google_emulate_server


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
            "LLM_API_KEY": "mock-api-key",
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
            "LLM_API_KEY": "mock-api-key",
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
            "IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK": "1",
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
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-webhook-no-secret-home-")
    home_dir = home_tmpdir.name
    base_dir = os.path.join(home_dir, ".ironclaw")
    os.makedirs(base_dir, exist_ok=True)
    gateway_port = _find_free_port()
    http_port = _find_free_port()
    env = {
        # Minimal env: PATH for process spawning, HOME for Rust/cargo defaults
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": base_dir,
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": OWNER_SCOPE_ID,
        "IRONCLAW_OWNER_ID": OWNER_SCOPE_ID,
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_API_KEY": "mock-api-key",
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
        home_tmpdir.cleanup()


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
    await _open_authed_gateway_page(pg, ironclaw_server, wait_for_sse=True)
    yield pg
    await context.close()


async def _open_authed_gateway_page(pg, base_url: str, *, wait_for_sse: bool = False) -> None:
    """Navigate to an authed gateway page, retrying one flaky first-load auth race."""
    from playwright.async_api import TimeoutError as PlaywrightTimeoutError

    url = f"{base_url}/?token={AUTH_TOKEN}"
    await pg.goto(url)
    try:
        await pg.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    except PlaywrightTimeoutError:
        await pg.goto(url)
        await pg.wait_for_selector("#auth-screen", state="hidden", timeout=15000)

    if not wait_for_sse:
        return

    # Wait for SSE connection (onopen sets sseHasConnectedBefore = true)
    await pg.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=10000,
    )


@pytest.fixture
async def loop_limited_page(loop_limited_server, browser):
    """Fresh Playwright page bound to the low-iteration gateway fixture."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    pg = await context.new_page()
    await _open_authed_gateway_page(pg, loop_limited_server)
    yield pg
    await context.close()


@pytest.fixture
async def length_preserving_page(length_preserving_server, browser):
    """Fresh Playwright page bound to the length-preserving gateway fixture."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    pg = await context.new_page()
    await _open_authed_gateway_page(pg, length_preserving_server)
    yield pg
    await context.close()

# ---------------------------------------------------------------------------
# Slack E2E fixtures
# ---------------------------------------------------------------------------

@pytest.fixture(scope="session")
async def fake_slack_server():
    """Start the fake Slack API server for E2E tests."""
    fake_api_path = Path(__file__).parent / "fake_slack_api.py"
    proc = await asyncio.create_subprocess_exec(
        sys.executable,
        str(fake_api_path),
        "--port",
        "0",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    port = await wait_for_port_line(proc, r"FAKE_SLACK_PORT=(\d+)")
    base_url = f"http://127.0.0.1:{port}"
    await wait_for_ready(f"{base_url}/__mock/sent_messages", timeout=10)
    yield base_url
    proc.send_signal(signal.SIGINT)
    try:
        await asyncio.wait_for(proc.wait(), timeout=5)
    except asyncio.TimeoutError:
        proc.kill()


async def _run_slack_provider_e2e_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    slack_api_url,
    *,
    result_url_key: str,
):
    """IronClaw instance wired to a Slack-compatible provider API."""
    reserved = _reserve_loopback_sockets(2)
    try:
        db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-slack-db-")
        home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-slack-home-")
        channels_tmpdir = tempfile.TemporaryDirectory(
            prefix="ironclaw-e2e-slack-channels-"
        )
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
            db_path=os.path.join(db_tmpdir.name, "slack-e2e.db"),
            extra_env={
                "GATEWAY_USER_ID": "e2e-tester",
                "ROUTINES_ENABLED": "false",
                "SKILLS_ENABLED": "false",
                "SECRETS_MASTER_KEY": (
                    "0123456789abcdef0123456789abcdef"
                    "0123456789abcdef0123456789abcdef"
                ),
                "WASM_CHANNELS_DIR": channels_tmpdir.name,
                "IRONCLAW_TEST_HTTP_REWRITE_MAP": json.dumps(
                    {
                        "slack.com": slack_api_url,
                        "files.slack.com": slack_api_url,
                    }
                ),
            },
        )

        proc = await asyncio.create_subprocess_exec(
            ironclaw_binary,
            "--no-onboard",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env,
        )
        startup_kill_attempted = False
        base_url = f"http://127.0.0.1:{gateway_port}"
        http_url = f"http://127.0.0.1:{http_port}"
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            yield {
                "base_url": base_url,
                "http_url": http_url,
                result_url_key: slack_api_url,
                "channels_dir": channels_tmpdir.name,
            }
        except TimeoutError:
            if proc.returncode is None:
                startup_kill_attempted = True
                await _stop_process(proc, timeout=2)
            returncode = proc.returncode
            stderr_bytes = b""
            if proc.stderr:
                try:
                    stderr_bytes = await asyncio.wait_for(
                        proc.stderr.read(8192), timeout=2
                    )
                except asyncio.TimeoutError:
                    pass
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            pytest.fail(
                f"slack_e2e_server failed to start on gateway port {gateway_port} "
                f"and webhook port {http_port} (returncode={returncode}).\n"
                f"stderr:\n{stderr_text}"
            )
        finally:
            if proc.returncode is None:
                if startup_kill_attempted:
                    await _stop_process(proc, timeout=2)
                else:
                    await _stop_process(proc, sig=signal.SIGINT, timeout=10)
                    if proc.returncode is None:
                        await _stop_process(proc, timeout=2)
            db_tmpdir.cleanup()
            home_tmpdir.cleanup()
            channels_tmpdir.cleanup()
    finally:
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()


@pytest.fixture(scope="session")
async def slack_e2e_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    fake_slack_server,
):
    """IronClaw instance wired to the fake Slack API for E2E Slack tests."""
    async for server in _run_slack_provider_e2e_server(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
        fake_slack_server,
        result_url_key="fake_slack_url",
    ):
        yield server


@pytest.fixture(scope="session")
async def slack_emulate_e2e_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    emulate_slack_server,
):
    """IronClaw instance wired to Emulate Slack for channel E2E tests."""
    async for server in _run_slack_provider_e2e_server(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
        emulate_slack_server["url"],
        result_url_key="emulate_slack_url",
    ):
        yield server

# ── Telegram E2E fixtures ────────────────────────────────────────────────


@pytest.fixture(scope="session")
async def fake_telegram_server():
    """Start the fake Telegram Bot API server. Yields the base URL."""
    server_script = Path(__file__).parent / "fake_telegram_api.py"
    proc = await asyncio.create_subprocess_exec(
        sys.executable, str(server_script), "--port", "0",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    try:
        port = await wait_for_port_line(
            proc, r"FAKE_TELEGRAM_PORT=(\d+)", timeout=10
        )
        url = f"http://127.0.0.1:{port}"
        yield url
    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            await asyncio.wait_for(proc.wait(), timeout=5)
        except asyncio.TimeoutError:
            proc.kill()


async def _telegram_e2e_server_impl(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    fake_telegram_server,
    *,
    routines_enabled: bool,
):
    """Start an isolated ironclaw instance wired to the fake Telegram API.

    Yields a dict with:
    - ``base_url``: gateway URL
    - ``http_url``: webhook server URL (for POSTing Telegram webhooks)
    - ``fake_tg_url``: fake Telegram API URL (for control endpoints)
    """
    reserved = _reserve_loopback_sockets(2)
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-tg-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-tg-home-")
    channels_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-tg-channels-")

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()

        home_dir = home_tmpdir.name
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": home_dir,
            "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
            "RUST_LOG": "ironclaw=debug",
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
            "LLM_API_KEY": "mock-api-key",
            "LLM_MODEL": "mock-model",
            "DATABASE_BACKEND": "libsql",
            "LIBSQL_PATH": os.path.join(db_tmpdir.name, "tg-e2e.db"),
            "SECRETS_MASTER_KEY": (
                "0123456789abcdef0123456789abcdef"
                "0123456789abcdef0123456789abcdef"
            ),
            "SANDBOX_ENABLED": "false",
            "SKILLS_ENABLED": "true",
            "ROUTINES_ENABLED": "true" if routines_enabled else "false",
            "HEARTBEAT_ENABLED": "false",
            "EMBEDDING_ENABLED": "false",
            "WASM_ENABLED": "true",
            "WASM_TOOLS_DIR": wasm_tools_dir,
            "WASM_CHANNELS_DIR": channels_tmpdir.name,
            "ONBOARD_COMPLETED": "true",
            "IRONCLAW_OAUTH_CALLBACK_URL": (
                "https://oauth.test.example/oauth/callback"
            ),
            "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_server,
            # Route Telegram API calls to the fake server
            "IRONCLAW_TEST_TELEGRAM_API_BASE_URL": fake_telegram_server,
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
        http_url = f"http://127.0.0.1:{http_port}"
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            yield {
                "base_url": base_url,
                "http_url": http_url,
                "fake_tg_url": fake_telegram_server,
                "channels_dir": channels_tmpdir.name,
            }
        except TimeoutError:
            if proc.returncode is None:
                startup_kill_attempted = True
                await _stop_process(proc, timeout=2)
            returncode = proc.returncode
            stderr_bytes = b""
            if proc.stderr:
                try:
                    stderr_bytes = await asyncio.wait_for(
                        proc.stderr.read(8192), timeout=2
                    )
                except asyncio.TimeoutError:
                    pass
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            pytest.fail(
                f"telegram e2e server failed to start on port {gateway_port} "
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
        channels_tmpdir.cleanup()


@pytest.fixture
async def telegram_e2e_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    fake_telegram_server,
):
    async for server in _telegram_e2e_server_impl(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
        fake_telegram_server,
        routines_enabled=False,
    ):
        yield server


@pytest.fixture
async def isolated_telegram_e2e_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    fake_telegram_server,
):
    async for server in _telegram_e2e_server_impl(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
        fake_telegram_server,
        routines_enabled=False,
    ):
        yield server


@pytest.fixture(scope="session")
async def telegram_e2e_server_with_routines(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    fake_telegram_server,
):
    async for server in _telegram_e2e_server_impl(
        ironclaw_binary,
        mock_llm_server,
        wasm_tools_dir,
        fake_telegram_server,
        routines_enabled=True,
    ):
        yield server


# ── Mission auto-resume (#3133 / #3166) ──────────────────────────────────


@pytest.fixture
async def mission_gmail_live_server(
    ironclaw_binary,
    mock_llm_server,
    wasm_tools_dir,
    request,
):
    """Isolated ironclaw instance for the mission auto-resume live test.

    Wires together three runtime pieces:

    1. The `live_llm_proxy.py` record/replay proxy as ironclaw's
       LLM backend. The proxy is launched with a per-test fixture
       file in `tests/e2e/fixtures/live/<test_name>.json`. In record
       mode (`IRONCLAW_LIVE_TEST=1`) it forwards to the upstream
       LLM and writes the trace; in replay mode it reads the
       committed trace.
    2. The existing mock_llm.py for the Gmail HTTP API mocks. The
       `IRONCLAW_TEST_HTTP_REWRITE_MAP` env var routes
       `gmail.googleapis.com` at mock_llm.py so the gmail WASM
       tool's HTTP calls land on deterministic responses (the
       `/__mock/gmail/state` endpoint exposes counters for
       assertions). OAuth exchange (`/oauth/exchange`) also lives
       on mock_llm.py.
    3. A function-scoped ironclaw process with engine v2 enabled
       and admin tools auto-approved so the chat-driven
       `routine_create` + `mission_fire` flow runs without a
       human in the loop. The *authentication* gate raised by
       the auth preflight on a direct `gmail` call is still active
       even with auto-approve — that's the gate the #3133
       auto-resume path exercises.
    """
    from live_harness import start_live_proxy

    proxy_iter = start_live_proxy(request.node.name)
    proxy = await proxy_iter.__anext__()

    reserved = _reserve_loopback_sockets(2)
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-mission-gmail-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-mission-gmail-home-")
    channels_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-mission-gmail-channels-")
    tools_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-e2e-mission-gmail-tools-")

    try:
        gateway_port = reserved[0].getsockname()[1]
        http_port = reserved[1].getsockname()[1]
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()

        # Override LLM base URL to the live proxy. _build_gateway_env
        # bakes in the LLM_BASE_URL from `mock_llm_server`; we override
        # via extra_env after building the rest of the env block.
        env = _build_gateway_env(
            mock_llm_server=mock_llm_server,
            wasm_tools_dir=tools_tmpdir.name,
            home_dir=home_tmpdir.name,
            gateway_port=gateway_port,
            http_port=http_port,
            db_path=os.path.join(db_tmpdir.name, "mission-gmail-live.db"),
            extra_env={
                "SECRETS_MASTER_KEY": (
                    "0123456789abcdef0123456789abcdef"
                    "0123456789abcdef0123456789abcdef"
                ),
                "WASM_CHANNELS_DIR": channels_tmpdir.name,
                # Route LLM through the live proxy.
                "LLM_BASE_URL": proxy["url"],
                # Route gmail.googleapis.com at mock_llm so the
                # gmail WASM tool's HTTP calls land deterministically.
                "IRONCLAW_TEST_HTTP_REWRITE_MAP": json.dumps(
                    {"gmail.googleapis.com": mock_llm_server}
                ),
                "IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK": "1",
                "GOOGLE_OAUTH_CLIENT_ID": "hosted-google-client-id",
                # Auto-approve administrative tools so the chat-driven
                # mission_create + fire sequence runs without a human
                # approval prompt. Authentication gates remain active.
                "AGENT_AUTO_APPROVE_TOOLS": "true",
                # Engine v2 is required: only v2 missions carry the
                # `paused_gate` field half-2 keys off of.
                "ENGINE_V2": "true",
            },
        )

        # Tee ironclaw stderr to a debug log so live-test failures are
        # diagnosable without re-running. Only used during /tests/e2e
        # live recordings — production paths don't see this.
        stderr_log_path = os.environ.get("IRONCLAW_E2E_STDERR_LOG")
        stderr_dest: Any = asyncio.subprocess.PIPE
        if stderr_log_path:
            stderr_dest = open(stderr_log_path, "w")  # noqa: SIM115
        proc = await asyncio.create_subprocess_exec(
            ironclaw_binary, "--no-onboard",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=stderr_dest,
            env=env,
        )
        startup_kill_attempted = False
        base_url = f"http://127.0.0.1:{gateway_port}"
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            yield {
                "base_url": base_url,
                "mock_llm_url": mock_llm_server,
                "live_proxy_url": proxy["url"],
                "fixture": str(proxy["fixture"]),
                "mode": proxy["mode"],
            }
        except TimeoutError:
            if proc.returncode is None:
                startup_kill_attempted = True
                await _stop_process(proc, timeout=2)
            returncode = proc.returncode
            stderr_bytes = b""
            if proc.stderr:
                try:
                    stderr_bytes = await asyncio.wait_for(
                        proc.stderr.read(8192), timeout=2
                    )
                except asyncio.TimeoutError:
                    pass
            stderr_text = stderr_bytes.decode("utf-8", errors="replace")
            pytest.fail(
                f"mission_gmail_live_server failed to start on port "
                f"{gateway_port} (returncode={returncode}).\n"
                f"stderr:\n{stderr_text}"
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
        # Tear the proxy down too.
        try:
            await proxy_iter.__anext__()
        except StopAsyncIteration:
            pass
        for sock in reserved:
            if sock.fileno() != -1:
                sock.close()
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()
        channels_tmpdir.cleanup()
        tools_tmpdir.cleanup()


@pytest.fixture
async def mission_gmail_live_page(mission_gmail_live_server, browser):
    """Browser page bound to the mission_gmail_live_server fixture."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    pg = await context.new_page()
    await _open_authed_gateway_page(pg, mission_gmail_live_server["base_url"])
    yield pg
    await context.close()
