"""Shared Reborn WebUI v2 E2E harness.

The legacy Playwright suite has mature shared fixtures in ``conftest.py`` for
the ``ironclaw`` gateway. Reborn WebUI v2 is a different product surface: it
boots ``ironclaw serve``, serves the React SPA at the root path, and uses
``/api/webchat/v2/*`` endpoints. Keep that setup here so browser and served API
scenarios exercise the real Reborn binary without duplicating process plumbing.
"""

import asyncio
import json
import os
import re
import shutil
import signal
import socket
import uuid
from pathlib import Path

import httpx
import pytest
from playwright.async_api import Error as PlaywrightError

from fixtures.mock_oauth_idp import MockOidcProfile, start_mock_oauth_idp
from hermetic_process import forward_hermetic_process_env
from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2, wait_for_ready

USER_ID = "reborn-v2-e2e-user"
DEFAULT_PROFILE = "local-dev"
YOLO_PROFILE = "local-dev-yolo"
DEFAULT_MODEL = "mock-model"
VISION_MODEL = "gpt-4o"
ACCEPTED_SEND_OUTCOMES = {"submitted", "already_submitted"}
SSO_GOOGLE_CLIENT_ID = "reborn-v2-e2e-google-client"
DEFAULT_ARTIFACT_MAX_BYTES = 256 * 1024 * 1024
MAX_SERVER_LOG_BYTES = 16 * 1024 * 1024
_ARTIFACT_PENDING_SENTINEL = ".pytest-outcome-pending"
_ARTIFACT_FAILED_SENTINEL = ".pytest-outcome-failed"
_ARTIFACT_BUNDLES_BY_NODE: dict[
    str,
    list[tuple[Path, Path, int]],
] = {}
_ARTIFACT_FAILED_NODES: set[str] = set()
_process_log_drains: dict[
    object,
    tuple[tuple[asyncio.Task[None], ...], Path, int],
] = {}

# Shared tenant secret for the test-tools/market-data fixture (test-tools/README.md).
# `IRONCLAW_REBORN_DEV_SECRET__<handle>` is read once at `serve` boot, so it must
# be present in the process env before start — see
# reborn_v2_private_installs_yolo_server below.
MARKET_DATA_DEV_SECRET = "e2e-market-data-shared-key"


def _directory_size(path: Path) -> int:
    return sum(
        entry.stat().st_size
        for entry in path.rglob("*")
        if entry.is_file()
    )


def _mark_artifact_bundle_outcome(
    artifact_dir: Path,
    outcome: str,
) -> None:
    pending = artifact_dir / _ARTIFACT_PENDING_SENTINEL
    failed = artifact_dir / _ARTIFACT_FAILED_SENTINEL
    pending.unlink(missing_ok=True)
    failed.unlink(missing_ok=True)
    if outcome == "pending":
        pending.touch()
    elif outcome == "failed":
        failed.touch()
    elif outcome != "passed":
        raise ValueError(f"unsupported artifact outcome: {outcome}")


def _artifact_bundle_is_protected(artifact_dir: Path) -> bool:
    return (
        (artifact_dir / _ARTIFACT_PENDING_SENTINEL).exists()
        or (artifact_dir / _ARTIFACT_FAILED_SENTINEL).exists()
    )


def _enforce_artifact_budget(
    artifact_root: Path,
    max_bytes: int,
    current_artifact_dir: Path | None,
) -> None:
    """Keep the complete upload tree bounded, pruning protected bundles last."""
    if not artifact_root.exists():
        return

    browser_artifact_root = artifact_root / "browser"
    bundles = (
        [path for path in browser_artifact_root.iterdir() if path.is_dir()]
        if browser_artifact_root.exists()
        else []
    )
    bundle_sizes = {path: _directory_size(path) for path in bundles}
    total_bytes = _directory_size(artifact_root)
    if total_bytes <= max_bytes:
        return

    oldest_first = sorted(
        (
            path
            for path in bundles
            if path != current_artifact_dir
            and not _artifact_bundle_is_protected(path)
        ),
        key=lambda path: path.stat().st_mtime_ns,
    )
    for path in oldest_first:
        if total_bytes <= max_bytes:
            break
        total_bytes -= bundle_sizes[path]
        shutil.rmtree(path)

    if total_bytes <= max_bytes:
        return

    protected_bundles = {
        path
        for path in bundles
        if path.exists() and _artifact_bundle_is_protected(path)
    }
    largest_first = sorted(
        (
            path
            for path in artifact_root.rglob("*")
            if path.is_file()
            and path.name
            not in {_ARTIFACT_PENDING_SENTINEL, _ARTIFACT_FAILED_SENTINEL}
        ),
        key=lambda path: (
            any(bundle in path.parents for bundle in protected_bundles),
            -path.stat().st_size,
        ),
    )
    for path in largest_first:
        if total_bytes <= max_bytes:
            break
        file_size = path.stat().st_size
        path.unlink()
        total_bytes -= file_size


def _server_log_max_bytes(artifact_max_bytes: int) -> int:
    """Reserve a bounded fraction of the shard budget for each server stream."""
    return max(1, min(MAX_SERVER_LOG_BYTES, artifact_max_bytes // 16))


async def _drain_stream_to_bounded_file(
    stream: asyncio.StreamReader,
    path: Path,
    max_bytes: int,
) -> None:
    """Continuously drain a process stream while retaining a bounded tail."""
    retained_bytes = max(1, max_bytes // 2)
    with path.open("w+b") as output:
        while chunk := await stream.read(64 * 1024):
            output.seek(0, os.SEEK_END)
            output.write(chunk)
            if output.tell() <= max_bytes:
                continue

            output.flush()
            output.seek(-retained_bytes, os.SEEK_END)
            tail = output.read(retained_bytes)
            output.seek(0)
            output.write(tail)
            output.truncate()


async def _finalize_process_logs(proc) -> None:
    drain_state = _process_log_drains.pop(proc, None)
    if drain_state is None:
        return

    tasks, artifact_root, artifact_max_bytes = drain_state
    await asyncio.gather(*tasks, return_exceptions=True)
    try:
        _enforce_artifact_budget(artifact_root, artifact_max_bytes, None)
    except OSError:
        # Diagnostics cleanup must never replace the scenario's real result.
        pass


def _register_artifact_bundle(
    node_id: str,
    artifact_root: Path,
    artifact_dir: Path,
    max_bytes: int,
) -> None:
    _ARTIFACT_BUNDLES_BY_NODE.setdefault(node_id, []).append(
        (artifact_root, artifact_dir, max_bytes)
    )
    outcome = "failed" if node_id in _ARTIFACT_FAILED_NODES else "pending"
    _mark_artifact_bundle_outcome(artifact_dir, outcome)


def _mark_registered_artifact_bundles_failed(node_id: str) -> None:
    _ARTIFACT_FAILED_NODES.add(node_id)
    for _, artifact_dir, _ in _ARTIFACT_BUNDLES_BY_NODE.get(node_id, []):
        if artifact_dir.exists():
            try:
                _mark_artifact_bundle_outcome(artifact_dir, "failed")
            except OSError:
                # Diagnostics bookkeeping must not replace the scenario failure.
                pass


def _finalize_registered_artifact_bundles(node_id: str) -> None:
    failed = node_id in _ARTIFACT_FAILED_NODES
    bundles = _ARTIFACT_BUNDLES_BY_NODE.pop(node_id, [])
    _ARTIFACT_FAILED_NODES.discard(node_id)
    for _, artifact_dir, _ in bundles:
        if artifact_dir.exists():
            try:
                _mark_artifact_bundle_outcome(
                    artifact_dir,
                    "failed" if failed else "passed",
                )
            except OSError:
                # Diagnostics bookkeeping must not replace the scenario result.
                pass

    roots: dict[tuple[Path, int], Path] = {}
    for artifact_root, artifact_dir, max_bytes in bundles:
        roots[(artifact_root, max_bytes)] = artifact_dir
    for (artifact_root, max_bytes), current_artifact_dir in roots.items():
        try:
            _enforce_artifact_budget(
                artifact_root,
                max_bytes,
                current_artifact_dir,
            )
        except OSError:
            # Pytest calls this from teardown; preserve the scenario result.
            pass


def _artifact_max_bytes() -> int:
    raw_value = os.environ.get("IRONCLAW_E2E_ARTIFACT_MAX_BYTES", "").strip()
    if not raw_value:
        return DEFAULT_ARTIFACT_MAX_BYTES
    try:
        max_bytes = int(raw_value)
    except ValueError as error:
        raise ValueError(
            "IRONCLAW_E2E_ARTIFACT_MAX_BYTES must be a positive integer"
        ) from error
    if max_bytes <= 0:
        raise ValueError(
            "IRONCLAW_E2E_ARTIFACT_MAX_BYTES must be a positive integer"
        )
    return max_bytes


class _ArtifactContext:
    """Browser context that persists diagnostics when CI requests them."""

    def __init__(
        self,
        context,
        artifact_dir: Path,
        artifact_root: Path,
        artifact_max_bytes: int,
    ):
        self._context = context
        self._artifact_dir = artifact_dir
        self._artifact_root = artifact_root
        self._artifact_max_bytes = artifact_max_bytes
        self._closed = False

    def __getattr__(self, name):
        return getattr(self._context, name)

    async def close(self) -> None:
        if self._closed:
            return
        self._closed = True

        for index, page in enumerate(self._context.pages, start=1):
            if page.is_closed():
                continue
            try:
                await page.screenshot(
                    path=str(self._artifact_dir / f"page-{index}.png"),
                    full_page=True,
                )
            except PlaywrightError:
                pass

        try:
            await self._context.tracing.stop(
                path=str(self._artifact_dir / "trace.zip")
            )
        except PlaywrightError:
            pass
        await self._context.close()
        try:
            _enforce_artifact_budget(
                self._artifact_root,
                self._artifact_max_bytes,
                self._artifact_dir,
            )
        except OSError:
            # Diagnostics cleanup must never replace the scenario's real result.
            pass


class _ArtifactBrowser:
    """Browser proxy that records each context under a unique artifact path."""

    def __init__(
        self,
        browser,
        artifact_root: Path,
        artifact_max_bytes: int,
    ):
        self._browser = browser
        self._artifact_root = artifact_root
        self._browser_artifact_root = artifact_root / "browser"
        self._artifact_max_bytes = artifact_max_bytes

    def __getattr__(self, name):
        return getattr(self._browser, name)

    async def new_context(self, *args, **kwargs):
        node_id = os.environ.get("PYTEST_CURRENT_TEST", "browser-context").split(
            " (", 1
        )[0]
        readable_name = re.sub(r"[^A-Za-z0-9_.-]+", "-", node_id).strip("-")[-160:]
        context_name = f"{readable_name}-{uuid.uuid4().hex[:8]}"
        artifact_dir = self._browser_artifact_root / context_name
        artifact_dir.mkdir(parents=True, exist_ok=True)
        _register_artifact_bundle(
            node_id,
            self._artifact_root,
            artifact_dir,
            self._artifact_max_bytes,
        )
        kwargs.setdefault("record_video_dir", str(artifact_dir / "videos"))
        kwargs.setdefault("record_video_size", {"width": 960, "height": 540})
        context = await self._browser.new_context(*args, **kwargs)
        await context.tracing.start(
            screenshots=True,
            snapshots=False,
            sources=False,
        )
        return _ArtifactContext(
            context,
            artifact_dir,
            self._artifact_root,
            self._artifact_max_bytes,
        )


def find_free_port() -> int:
    """Ask the OS for an available loopback port as a startup hint."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def read_log(path: Path, limit: int = 8192) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")[-limit:]
    except OSError:
        return ""


def forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value
    forward_hermetic_process_env(env)


async def stop_process(proc, *, sig=signal.SIGINT, timeout: float = 10) -> None:
    """Signal a subprocess and wait for exit without re-reading stdio pipes."""
    if proc.returncode is not None:
        await _finalize_process_logs(proc)
        return
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        await proc.wait()
        await _finalize_process_logs(proc)
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await asyncio.wait_for(proc.wait(), timeout=5)
    await _finalize_process_logs(proc)


def write_config_toml(
    path: Path,
    mock_llm_server: str,
    profile: str = DEFAULT_PROFILE,
    model: str = DEFAULT_MODEL,
) -> None:
    """Seed a sparse Reborn config that selects the mock OpenAI-compatible LLM."""
    path.write_text(
        f"""api_version = "ironclaw.runtime/v1"

[boot]
profile = "{profile}"

[identity]
default_owner = "{USER_ID}"
tenant = "reborn-v2-e2e"
default_agent = "reborn-v2-e2e-agent"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "{model}"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "{mock_llm_server}/v1"
""",
        encoding="utf-8",
    )


async def start_reborn_webui_v2_server(
    *,
    ironclaw_reborn_binary: str,
    mock_llm_server: str,
    home_dir: Path,
    profile: str = DEFAULT_PROFILE,
    model: str = DEFAULT_MODEL,
    log_prefix: str = "reborn-v2",
    extra_env: dict[str, str] | None = None,
    use_listener_as_webui_base_url: bool = False,
) -> tuple[object, str]:
    """Start ``ironclaw serve`` and return ``(process, base_url)``."""
    configured_artifact_root = os.environ.get(
        "IRONCLAW_E2E_ARTIFACT_DIR", ""
    ).strip()
    artifact_root = (
        Path(configured_artifact_root).resolve()
        if configured_artifact_root
        else None
    )
    artifact_max_bytes = (
        _artifact_max_bytes() if artifact_root else DEFAULT_ARTIFACT_MAX_BYTES
    )
    binary_path = str(Path(ironclaw_reborn_binary).resolve())
    reborn_home = home_dir / "reborn-home"
    reborn_home.mkdir(parents=True, exist_ok=True)
    workspace_dir = home_dir / "workspace"
    workspace_dir.mkdir(parents=True, exist_ok=True)
    write_config_toml(
        reborn_home / "config.toml",
        mock_llm_server,
        profile=profile,
        model=model,
    )

    proc = None
    last_stderr = ""
    last_port = None

    for attempt in range(1, 4):
        port = find_free_port()
        last_port = port
        base_url = f"http://127.0.0.1:{port}"
        if artifact_root:
            log_dir = artifact_root / "server-logs" / home_dir.name
            log_dir.mkdir(parents=True, exist_ok=True)
        else:
            log_dir = home_dir
        stdout_path = log_dir / f"{log_prefix}-attempt-{attempt}.stdout.log"
        stderr_path = log_dir / f"{log_prefix}-attempt-{attempt}.stderr.log"

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home_dir),
            "IRONCLAW_REBORN_HOME": str(reborn_home),
            "IRONCLAW_REBORN_PROFILE": profile,
            "IRONCLAW_REBORN_WEBUI_TOKEN": REBORN_V2_AUTH_TOKEN,
            "IRONCLAW_REBORN_WEBUI_USER_ID": USER_ID,
            # Recorded provider fixtures assert the pre-disclosure request
            # shape. Keep this shared deterministic harness explicit rather
            # than inheriting the production default.
            "REBORN_TOOL_DISCLOSURE": "off",
            "MOCK_LLM_API_KEY": "mock-api-key",
            "NO_PROXY": "127.0.0.1,localhost,::1",
            "no_proxy": "127.0.0.1,localhost,::1",
            "RUST_LOG": "ironclaw=warn,ironclaw_runner=warn",
            "RUST_BACKTRACE": "1",
        }
        if extra_env:
            env.update(extra_env)
        if use_listener_as_webui_base_url:
            env["IRONCLAW_REBORN_WEBUI_BASE_URL"] = base_url
        forward_coverage_env(env)

        args = [
            binary_path,
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
        ]
        if profile == YOLO_PROFILE:
            args.insert(2, "--confirm-host-access")

        if artifact_root:
            proc = await asyncio.create_subprocess_exec(
                *args,
                stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                env=env,
                cwd=workspace_dir,
            )
            if proc.stdout is None or proc.stderr is None:
                raise RuntimeError("server log pipes were not created")
            log_max_bytes = _server_log_max_bytes(artifact_max_bytes)
            _process_log_drains[proc] = (
                (
                    asyncio.create_task(
                        _drain_stream_to_bounded_file(
                            proc.stdout,
                            stdout_path,
                            log_max_bytes,
                        )
                    ),
                    asyncio.create_task(
                        _drain_stream_to_bounded_file(
                            proc.stderr,
                            stderr_path,
                            log_max_bytes,
                        )
                    ),
                ),
                artifact_root,
                artifact_max_bytes,
            )
        else:
            with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
                proc = await asyncio.create_subprocess_exec(
                    *args,
                    stdin=asyncio.subprocess.DEVNULL,
                    stdout=out,
                    stderr=err,
                    env=env,
                    cwd=workspace_dir,
                )
        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            return proc, base_url
        except TimeoutError:
            if proc.returncode is None:
                await stop_process(proc, timeout=2)
            last_stderr = read_log(stderr_path)
            proc = None

    pytest.fail(
        f"Reborn WebUI v2 server failed to start after 3 attempts.\n"
        f"Last attempted port: {last_port}\n"
        f"stderr:\n{last_stderr}"
    )


async def close_reborn_server(proc) -> None:
    if proc is not None and proc.returncode is None:
        await stop_process(proc, sig=signal.SIGINT, timeout=10)
        if proc.returncode is None:
            await stop_process(proc, sig=signal.SIGTERM, timeout=5)
    elif proc is not None:
        await _finalize_process_logs(proc)


async def kill_reborn_server(proc) -> None:
    """Hard-kill (SIGKILL) the reborn process, skipping graceful shutdown entirely.

    Used by durability scenarios that need to prove on-disk state survives an
    unclean process death, as opposed to `close_reborn_server`'s SIGINT/SIGTERM path.
    """
    if proc is not None and proc.returncode is None:
        await stop_process(proc, sig=signal.SIGKILL, timeout=5)
    elif proc is not None:
        await _finalize_process_logs(proc)


async def enable_reborn_global_auto_approve(
    base_url: str, *, token: str = REBORN_V2_AUTH_TOKEN
) -> None:
    """Enable the Tools settings global auto-approve switch for this test user."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers(token)) as client:
        response = await client.post(
            f"{base_url}/api/webchat/v2/settings/tools",
            json={"enabled": True},
            timeout=15,
        )
        response.raise_for_status()


@pytest.fixture(scope="module")
async def reborn_v2_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Start ``ironclaw serve`` with the default local-dev profile."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=DEFAULT_PROFILE,
    )
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_sso_server(
    ironclaw_reborn_sso_binary, mock_llm_server, tmp_path_factory
):
    """Start ``ironclaw serve`` with Google SSO backed by a local mock IDP."""
    profiles = (
        MockOidcProfile(
            subject="alice-subject",
            email="alice@example.com",
            display_name="Alice E2E",
        ),
        MockOidcProfile(
            subject="bob-subject",
            email="bob@example.com",
            display_name="Bob E2E",
        ),
    )
    async for provider in start_mock_oauth_idp(oidc_profiles=profiles):
        home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-sso-home")
        proc, base_url = await start_reborn_webui_v2_server(
            ironclaw_reborn_binary=ironclaw_reborn_sso_binary,
            mock_llm_server=mock_llm_server,
            home_dir=home_dir,
            profile=DEFAULT_PROFILE,
            log_prefix="reborn-v2-sso",
            use_listener_as_webui_base_url=True,
            extra_env={
                "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID": SSO_GOOGLE_CLIENT_ID,
                "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET": "mock-google-secret",
                "IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS": "example.com",
                # Defeat ambient repo/user .env values so this fixture never
                # advertises or contacts a provider it did not start itself.
                "IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID": "",
                "IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET": "",
                "IRONCLAW_REBORN_TEST_WEBUI_GOOGLE_AUTH_ENDPOINT": (
                    provider.authorize_url
                ),
                "IRONCLAW_REBORN_TEST_WEBUI_GOOGLE_TOKEN_ENDPOINT": provider.token_url,
            },
        )
        try:
            yield {"base_url": base_url, "provider": provider}
        finally:
            await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_yolo_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Start ``ironclaw serve`` with auto-approval local-dev-yolo profile."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-yolo-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-v2-yolo",
    )
    await enable_reborn_global_auto_approve(base_url)
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_private_installs_yolo_server(
    ironclaw_reborn_binary, mock_llm_server, tmp_path_factory
):
    """Yolo-profile server with the market-data tenant-shared dev secret seeded.

    Used by the private-tool-installs scenario (#5459 P1): auto-approve so
    installed third-party WASM capabilities dispatch without an approval
    gate, plus the market-data fixture's shared API key present at boot.
    """
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-private-installs-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-v2-private-installs-yolo",
        extra_env={"IRONCLAW_REBORN_DEV_SECRET__market_data_api_key": MARKET_DATA_DEV_SECRET},
    )
    await enable_reborn_global_auto_approve(base_url)
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture
async def reborn_v2_restartable_server(
    ironclaw_reborn_binary, mock_llm_server, tmp_path_factory
):
    """Start/stop Reborn against one persistent home directory.

    `stop(hard=True)` SIGKILLs the process instead of shutting it down
    gracefully, for durability scenarios that need to prove on-disk state
    survives an unclean death — the caller can read the killed PID off
    `state["proc"].pid` beforehand for a post-kill leak check.
    """
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-restartable-home")
    state = {"proc": None, "base_url": None}

    async def start() -> str:
        proc, base_url = await start_reborn_webui_v2_server(
            ironclaw_reborn_binary=ironclaw_reborn_binary,
            mock_llm_server=mock_llm_server,
            home_dir=home_dir,
            profile=DEFAULT_PROFILE,
            log_prefix="reborn-v2-restartable",
        )
        state["proc"] = proc
        state["base_url"] = base_url
        return base_url

    async def stop(*, hard: bool = False) -> None:
        if hard:
            await kill_reborn_server(state["proc"])
        else:
            await close_reborn_server(state["proc"])
        state["proc"] = None

    await start()
    try:
        yield state, start, stop
    finally:
        await stop()


@pytest.fixture(scope="module")
async def reborn_v2_loop_limited_yolo_server(
    ironclaw_reborn_binary, mock_llm_server, tmp_path_factory
):
    """Start Reborn yolo mode with a low planned-profile loop budget."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-loop-limited-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-v2-loop-limited-yolo",
        extra_env={
            "IRONCLAW_REBORN_PLANNED_DEFAULT_ITERATION_LIMIT": "1",
        },
    )
    await enable_reborn_global_auto_approve(base_url)
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_vision_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Start Reborn with a vision-classified model id backed by the mock LLM."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-vision-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=DEFAULT_PROFILE,
        model=VISION_MODEL,
        log_prefix="reborn-v2-vision",
    )
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_browser():
    """Chromium instance for Reborn v2 tests, independent of the legacy gateway."""
    from playwright.async_api import async_playwright

    headless = os.environ.get("HEADED", "").strip() not in ("1", "true")
    artifact_root = os.environ.get("IRONCLAW_E2E_ARTIFACT_DIR", "").strip()
    artifact_max_bytes = (
        _artifact_max_bytes() if artifact_root else DEFAULT_ARTIFACT_MAX_BYTES
    )
    async with async_playwright() as p:
        browser = None
        for attempt in range(3):
            try:
                browser = await p.chromium.launch(headless=headless, timeout=60000)
                break
            except PlaywrightError:
                if attempt == 2:
                    raise
                await asyncio.sleep(1)
        if artifact_root:
            yield _ArtifactBrowser(
                browser,
                Path(artifact_root).resolve(),
                artifact_max_bytes,
            )
        else:
            yield browser
        await browser.close()


@pytest.fixture
async def reborn_v2_page_factory(reborn_v2_server, reborn_v2_browser):
    """Create managed Reborn pages, optionally preparing routes before navigation."""
    contexts = []

    async def open_page(
        *,
        path: str = "/",
        before_navigation=None,
        ready_selector: str | None = SEL_V2["chat_composer"],
    ):
        context, page = await create_reborn_v2_page(
            reborn_v2_browser,
            reborn_v2_server,
            path=path,
            before_navigation=before_navigation,
            ready_selector=ready_selector,
        )
        contexts.append(context)
        return {"context": context, "page": page}

    yield open_page

    for context in reversed(contexts):
        await context.close()


@pytest.fixture
async def reborn_v2_page(reborn_v2_server, reborn_v2_browser):
    """Fresh authenticated page on the Reborn v2 SPA."""
    context, page = await create_reborn_v2_page(
        reborn_v2_browser,
        reborn_v2_server,
    )
    yield page
    await context.close()


@pytest.fixture
async def reborn_v2_yolo_page(reborn_v2_yolo_server, reborn_v2_browser):
    """Fresh authenticated yolo-profile page with downloads enabled."""
    context = await reborn_v2_browser.new_context(
        viewport={"width": 1280, "height": 720}, accept_downloads=True
    )
    page = await context.new_page()
    await open_reborn_v2_page(page, reborn_v2_yolo_server)
    yield page
    await context.close()


@pytest.fixture
async def reborn_v2_vision_page(reborn_v2_vision_server, reborn_v2_browser):
    """Fresh authenticated page backed by a vision-classified mock model."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await open_reborn_v2_page(page, reborn_v2_vision_server)
    yield page
    await context.close()


async def open_reborn_v2_page(
    page,
    base_url: str,
    path: str = "/",
    ready_selector: str | None = SEL_V2["chat_composer"],
) -> None:
    separator = "&" if "?" in path else "?"
    await page.goto(f"{base_url}{path}{separator}token={REBORN_V2_AUTH_TOKEN}")
    if ready_selector is not None:
        await page.wait_for_selector(ready_selector, timeout=15000)


async def create_reborn_v2_page(
    browser,
    base_url: str,
    *,
    path: str = "/",
    before_navigation=None,
    ready_selector: str | None = SEL_V2["chat_composer"],
):
    """Create one page and run optional setup before its first navigation."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    try:
        page = await context.new_page()
        if before_navigation is not None:
            await before_navigation(page)
        await open_reborn_v2_page(
            page,
            base_url,
            path=path,
            ready_selector=ready_selector,
        )
        return context, page
    except BaseException:
        await context.close()
        raise


def reborn_bearer_headers(token: str = REBORN_V2_AUTH_TOKEN) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


async def fetch_extension_oauth_requirement(
    client: httpx.AsyncClient,
    base_url: str,
    package_id: str,
) -> dict:
    """Read the opaque OAuth requirement declared by an installed manifest."""
    response = await client.get(
        f"{base_url}/api/webchat/v2/extensions/{package_id}/setup",
        timeout=15,
    )
    response.raise_for_status()
    requirements = [
        secret
        for secret in response.json().get("secrets", [])
        if (secret.get("setup") or {}).get("kind") == "oauth"
    ]
    assert len(requirements) == 1, (
        f"expected exactly one manifest-declared OAuth requirement for {package_id}; "
        f"got {requirements}"
    )
    return requirements[0]


def client_action_id() -> str:
    """Idempotency key accepted by ``product_surface_inbound::parse_client_action_id``."""
    return str(uuid.uuid4())


async def create_thread(client: httpx.AsyncClient, base_url: str) -> str:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads",
        json={"client_action_id": client_action_id()},
        timeout=15,
    )
    response.raise_for_status()
    return response.json()["thread"]["thread_id"]


async def _submit_message(
    client: httpx.AsyncClient, base_url: str, thread_id: str, content: str
) -> dict:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/messages",
        json={"client_action_id": client_action_id(), "content": content},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text
    return response.json()


async def send_message(
    client: httpx.AsyncClient, base_url: str, thread_id: str, content: str
) -> dict:
    body = await _submit_message(client, base_url, thread_id, content)
    outcome = body.get("outcome")
    assert outcome in ACCEPTED_SEND_OUTCOMES, (
        f"Message was not accepted for a run; outcome={outcome!r}, body={body}"
    )
    return body


async def fetch_timeline(client: httpx.AsyncClient, base_url: str, thread_id: str) -> dict:
    response = await client.get(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/timeline",
        timeout=15,
    )
    response.raise_for_status()
    return response.json()


async def wait_for_assistant_message(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Poll the timeline until a finalized assistant message appears."""
    last_timeline: dict = {}
    for _ in range(int(timeout * 2)):
        try:
            last_timeline = await fetch_timeline(client, base_url, thread_id)
        except httpx.HTTPError:
            await asyncio.sleep(0.5)
            continue
        finalized = [
            message
            for message in last_timeline.get("messages", [])
            if message.get("kind") == "assistant"
            and message.get("status") == "finalized"
            and (message.get("content") or "").strip()
        ]
        if finalized:
            return finalized[-1]
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for a finalized assistant message in thread {thread_id}. "
        f"Last timeline: {last_timeline}"
    )


def capability_preview_payload(message: dict) -> dict | None:
    """Parse a `capability_display_preview` timeline message's JSON content.

    Returns `None` for any other message kind.
    """
    if message.get("kind") != "capability_display_preview":
        return None
    content = message.get("content")
    assert isinstance(content, str), f"preview content must be a string: {message!r}"
    try:
        return json.loads(content)
    except json.JSONDecodeError as error:
        raise AssertionError(f"preview content is not valid JSON: {content!r}") from error


async def wait_for_capability_preview(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    capability_id: str,
    *,
    output_fragment: str | None = None,
    timeout: float = 45.0,
) -> dict:
    """Poll the timeline until a `capability_display_preview` for `capability_id`
    appears (optionally containing `output_fragment` in its output)."""
    last_timeline: dict = {}
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        last_timeline = await fetch_timeline(client, base_url, thread_id)
        for message in last_timeline.get("messages", []):
            preview = capability_preview_payload(message)
            if not preview or preview.get("capability_id") != capability_id:
                continue
            output = preview.get("output_preview") or preview.get("output_summary") or ""
            if output_fragment and output_fragment.lower() not in output.lower():
                continue
            return preview
        await asyncio.sleep(0.25)

    raise AssertionError(
        f"Timed out waiting for {capability_id!r} preview in thread {thread_id}. "
        f"Last timeline: {last_timeline}"
    )


def finalized_assistant_count(timeline: dict) -> int:
    return sum(
        1
        for message in timeline.get("messages", [])
        if message.get("kind") == "assistant"
        and message.get("status") == "finalized"
        and (message.get("content") or "").strip()
    )


async def send_and_settle(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    content: str,
    expected: int,
) -> None:
    """Send a text turn and wait until ``expected`` assistant replies finalize."""
    submit_body: dict = {}
    last_submit_error = None
    for _ in range(12):
        try:
            submit_body = await _submit_message(client, base_url, thread_id, content)
            last_submit_error = None
        except httpx.HTTPError as error:
            last_submit_error = error
            await asyncio.sleep(0.5)
            continue
        outcome = submit_body.get("outcome")
        if outcome in ACCEPTED_SEND_OUTCOMES:
            break
        if outcome == "rejected_busy":
            await asyncio.sleep(0.5)
            continue
        raise AssertionError(
            f"Message was not accepted for a run; outcome={outcome!r}, body={submit_body}"
        )
    else:
        raise AssertionError(
            f"Thread {thread_id} remained busy before accepting a new turn; "
            f"last submit response: {submit_body}; last submit error: {last_submit_error!r}"
        )

    for _ in range(90):
        try:
            timeline = await fetch_timeline(client, base_url, thread_id)
        except httpx.HTTPError:
            await asyncio.sleep(0.5)
            continue
        if finalized_assistant_count(timeline) >= expected:
            return
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Thread {thread_id} did not reach {expected} finalized assistant replies; "
        f"submit response: {submit_body}"
    )
