from __future__ import annotations

import asyncio
import json
import os
import re
import select
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
E2E_DIR = ROOT / "tests" / "e2e"
DEFAULT_VENV = E2E_DIR / ".venv"

class CanaryError(RuntimeError):
    pass


@dataclass
class ProbeResult:
    provider: str
    mode: str
    success: bool
    latency_ms: int
    details: dict[str, Any] = field(default_factory=dict)


@dataclass
class GatewayStack:
    base_url: str
    gateway_token: str
    db_path: Path
    mock_llm_url: str
    gateway_proc: subprocess.Popen[str]
    mock_llm_proc: subprocess.Popen[str]
    tempdirs: list[tempfile.TemporaryDirectory[str]]
    # http_url and channels_dir are populated when start_gateway_stack
    # is invoked. http_url is the HTTP-channel webhook endpoint
    # (separate from the gateway's REST API port). channels_dir is the
    # WASM channels base directory — needed by Telegram-channel-install
    # scenarios that patch the per-channel capabilities.json.
    http_url: str = ""
    channels_dir: str = ""


def run(cmd: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    rendered = " ".join(shlex.quote(part) for part in cmd)
    print(f"+ {rendered}", flush=True)
    subprocess.run(cmd, cwd=cwd or ROOT, env=env, check=True)


def venv_python(venv_dir: Path) -> Path:
    if os.name == "nt":
        return venv_dir / "Scripts" / "python.exe"
    return venv_dir / "bin" / "python"


def bootstrap_python(venv_dir: Path) -> Path:
    if not venv_dir.exists():
        run([sys.executable, "-m", "venv", str(venv_dir)])
    python = venv_python(venv_dir)
    run([str(python), "-m", "pip", "install", "--upgrade", "pip"])
    run([str(python), "-m", "pip", "install", "-e", str(E2E_DIR)])
    return python


def install_playwright(python: Path, mode: str) -> None:
    resolved = mode
    if mode == "auto":
        resolved = "with-deps" if os.environ.get("CI") else "plain"
    if resolved == "skip":
        return
    cmd = [str(python), "-m", "playwright", "install"]
    if resolved == "with-deps":
        cmd.append("--with-deps")
    cmd.append("chromium")
    run(cmd, cwd=E2E_DIR)


def cargo_build() -> None:
    run(["cargo", "build", "--no-default-features", "--features", "libsql"], cwd=ROOT)


def env_str(name: str, default: str | None = None) -> str | None:
    value = os.environ.get(name, default)
    if value is None:
        return None
    value = value.strip()
    return value or None


def env_secret(name: str) -> str | None:
    """Read a canary secret, preferring the `<NAME>_PATH` file variant.

    The CI workflow materialises sensitive secrets (tokens, client
    secrets, passwords) into mode-0600 tempfiles rather than exposing
    them directly as job env vars, and then exports `<NAME>_PATH`
    pointing at the file. This helper reads from that file when the
    path is set; otherwise it falls back to the raw env var so local
    development via `config.env` (see
    `scripts/auth_live_canary/config.example.env`) keeps working
    unchanged.

    Trailing newlines are stripped so a file written with
    `printf '%s\\n' "$SECRET"` matches a raw env var carrying the
    same value. Empty files collapse to `None` (same shape as an
    unset var).
    """
    path = env_str(f"{name}_PATH")
    if path:
        try:
            value = Path(path).read_text(encoding="utf-8")
        except OSError:
            return None
        value = value.rstrip("\r\n")
        return value or None
    return env_str(name)


def required_env(name: str, *, message: str | None = None) -> str:
    value = env_str(name)
    if value:
        return value
    raise CanaryError(message or f"{name} is required")


def required_secret(name: str, *, message: str | None = None) -> str:
    """File-aware variant of `required_env` for sensitive secrets."""
    value = env_secret(name)
    if value:
        return value
    raise CanaryError(message or f"{name} is required")


def generate_secrets_master_key() -> str:
    return os.urandom(32).hex()


def reserve_loopback_port() -> int:
    """Pick a free loopback port by binding and closing a throwaway socket.

    Known TOCTOU: the kernel releases the port the moment this
    function returns, so a concurrent process COULD claim it before
    the caller's subprocess re-binds. For subprocesses that accept
    `--port 0` and print the bound port on stdout (e.g. `mock_llm.py`),
    prefer the "bind-then-report" pattern via `wait_for_port_line`
    instead — that pattern is race-free because the child is the only
    party that ever binds.

    This helper remains for callers whose subprocess expects a
    pre-chosen port via env var (e.g. the ironclaw gateway, which
    reads `GATEWAY_PORT` as a fixed u16 and does not support
    port-0 discovery). The race window there is on the order of
    milliseconds on an otherwise idle canary runner; if you see
    `EADDRINUSE` failures in practice, wrap the subprocess start in
    a retry loop that re-reserves on bind failure.
    """
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_for_port_line(
    proc: subprocess.Popen[str],
    pattern: re.Pattern[str],
    timeout: float,
) -> re.Match[str]:
    # Use select() so the deadline is actually enforced; readline() alone can
    # block forever if the child never prints a newline.
    deadline = time.monotonic() + timeout
    stdout = proc.stdout
    if stdout is None:
        raise CanaryError("process has no stdout pipe")
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise CanaryError("Timed out waiting for service port announcement")
        ready, _, _ = select.select([stdout], [], [], min(remaining, 0.5))
        if not ready:
            if proc.poll() is not None:
                raise CanaryError("process exited before printing its port")
            continue
        line = stdout.readline()
        if not line:
            if proc.poll() is not None:
                raise CanaryError("process exited before printing its port")
            continue
        match = pattern.search(line)
        if match:
            return match


async def wait_for_ready(url: str, timeout: float = 60.0, interval: float = 0.5) -> None:
    import httpx

    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient(timeout=10.0) as client:
        while time.monotonic() < deadline:
            try:
                response = await client.get(url)
                if response.status_code == 200:
                    return
            except httpx.HTTPError:
                pass
            await asyncio.sleep(interval)
    raise CanaryError(f"Timed out waiting for readiness: {url}")


def stop_process(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is not None:
        return
    proc.send_signal(signal.SIGINT)
    try:
        proc.wait(timeout=10)
        return
    except subprocess.TimeoutExpired:
        proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


async def api_request(
    method: str,
    base_url: str,
    path: str,
    *,
    token: str,
    json_body: Any | None = None,
    timeout: float = 30.0,
) -> Any:
    import httpx

    headers = {"Authorization": f"Bearer {token}"}
    async with httpx.AsyncClient(timeout=timeout) as client:
        response = await client.request(
            method,
            f"{base_url}{path}",
            headers=headers,
            json=json_body,
        )
    return response


def write_results(output_dir: Path, results: list[ProbeResult], base_url: str) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    path = output_dir / "results.json"
    payload = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "base_url": base_url,
        "results": [asdict(result) for result in results],
    }
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def load_e2e_helpers(*names: str) -> tuple[Any, ...]:
    sys.path.insert(0, str(E2E_DIR))
    helpers = __import__("helpers", fromlist=list(names))
    return tuple(getattr(helpers, name) for name in names)


def build_gateway_env(
    *,
    owner_user_id: str,
    gateway_port: int,
    http_port: int,
    gateway_token: str,
    db_path: Path,
    home_dir: Path,
    tools_dir: Path,
    channels_dir: Path,
    mock_llm_url: str,
    secrets_master_key: str,
    extra_env: dict[str, str] | None = None,
) -> dict[str, str]:
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(home_dir),
        "IRONCLAW_BASE_DIR": str(home_dir / ".ironclaw"),
        "RUST_LOG": os.environ.get("RUST_LOG", "ironclaw=info"),
        "RUST_BACKTRACE": "1",
        "IRONCLAW_OWNER_ID": owner_user_id,
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": gateway_token,
        "GATEWAY_USER_ID": owner_user_id,
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_url,
        "LLM_MODEL": "mock-model",
        # Even though the mock LLM ignores the API key, the
        # openai_compatible provider refuses to instantiate without one.
        # Without this the provider falls back to NearAI's DB default.
        "LLM_API_KEY": "mock-api-key",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": str(db_path),
        "SECRETS_MASTER_KEY": secrets_master_key,
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "true",
        "WASM_TOOLS_DIR": str(tools_dir),
        "WASM_CHANNELS_DIR": str(channels_dir),
        "ONBOARD_COMPLETED": "true",
    }
    if extra_env:
        env.update({key: value for key, value in extra_env.items() if value})
    return env


async def _pin_mock_llm_settings(
    base_url: str, gateway_token: str, mock_llm_url: str
) -> None:
    """Pin LLM backend/base_url/model via the settings API.

    Required because the gateway's DB settings take priority over the
    LLM_BACKEND / LLM_BASE_URL / LLM_MODEL env vars; the freshly-seeded
    DB defaults llm_backend to `nearai`, which sends the agent into an
    interactive auth flow that hangs in CI. See tests/e2e/CLAUDE.md.
    """
    import httpx  # local import: keep top-level import set unchanged

    headers = {"Authorization": f"Bearer {gateway_token}"}
    writes = [
        ("llm_backend", "openai_compatible"),
        ("openai_compatible_base_url", mock_llm_url),
        ("selected_model", "mock-model"),
    ]
    async with httpx.AsyncClient(timeout=15.0) as client:
        for key, value in writes:
            response = await client.put(
                f"{base_url}/api/settings/{key}",
                headers=headers,
                json={"value": value},
            )
            if response.status_code not in (200, 201, 204):
                raise CanaryError(
                    f"Failed to pin LLM setting {key}: "
                    f"{response.status_code} {response.text[:300]}"
                )


def _drain_to_file(stream: Any, path: Path) -> threading.Thread:
    """Drain a subprocess stdout/stderr stream to a file in a daemon thread.

    Without this, ``subprocess.Popen(stdout=PIPE)`` deadlocks: the kernel
    pipe buffer (64 KiB on Linux, varies on macOS) fills under sustained
    log output and the child blocks on its next write. That manifests on
    CI as IronClaw freezing mid-request — locally the pipe fills more
    slowly so the symptom is masked. See PR #2978-ish (this fix).
    """

    def _drain() -> None:
        try:
            with path.open("a", encoding="utf-8", errors="replace") as fh:
                for line in stream:
                    fh.write(line)
                    fh.flush()
        except Exception:  # noqa: BLE001
            pass

    thread = threading.Thread(target=_drain, daemon=True)
    thread.start()
    return thread


async def start_gateway_stack(
    *,
    venv_dir: Path,
    owner_user_id: str,
    secrets_master_key: str | None = None,
    temp_prefix: str,
    gateway_token_prefix: str,
    extra_gateway_env: dict[str, str] | None = None,
    oauth_proxy: bool = False,
    log_dir: Path | None = None,
) -> GatewayStack:
    secrets_master_key = secrets_master_key or generate_secrets_master_key()
    python = venv_python(venv_dir)
    # Race-free port acquisition: `mock_llm.py --port 0` binds the
    # kernel-assigned port itself and prints `MOCK_LLM_PORT=<N>` on
    # startup, which `wait_for_port_line` reads below. Using
    # `reserve_loopback_port()` here would open a TOCTOU window where
    # another process could claim the port between reservation and
    # subprocess bind.
    mock_llm_proc = subprocess.Popen(
        [str(python), str(E2E_DIR / "mock_llm.py"), "--port", "0"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )

    tempdirs = [
        tempfile.TemporaryDirectory(prefix=f"{temp_prefix}-db-"),
        tempfile.TemporaryDirectory(prefix=f"{temp_prefix}-home-"),
        tempfile.TemporaryDirectory(prefix=f"{temp_prefix}-tools-"),
        tempfile.TemporaryDirectory(prefix=f"{temp_prefix}-channels-"),
    ]
    db_tmp, home_tmp, tools_tmp, channels_tmp = tempdirs

    try:
        match = wait_for_port_line(
            mock_llm_proc,
            re.compile(r"MOCK_LLM_PORT=(\d+)"),
            timeout=30.0,
        )
        mock_llm_url = f"http://127.0.0.1:{match.group(1)}"
        await wait_for_ready(f"{mock_llm_url}/v1/models", timeout=30.0)

        # Now that the port-discovery line has been consumed, drain the
        # rest of mock_llm.py's stdout to a log file so the pipe never
        # fills (64 KiB pipe buffers on Linux deadlock the child once
        # full).
        if log_dir is not None and mock_llm_proc.stdout is not None:
            log_dir.mkdir(parents=True, exist_ok=True)
            _drain_to_file(mock_llm_proc.stdout, log_dir / "mock_llm.log")

        if oauth_proxy:
            proxy_env = {
                "IRONCLAW_OAUTH_EXCHANGE_URL": mock_llm_url,
                "IRONCLAW_OAUTH_CALLBACK_URL": "https://oauth.test.example/oauth/callback",
                "IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK": "1",
            }
            extra_gateway_env = {**(extra_gateway_env or {}), **proxy_env}

        gateway_port = reserve_loopback_port()
        http_port = reserve_loopback_port()
        gateway_token = f"{gateway_token_prefix}-{uuid.uuid4().hex[:12]}"
        db_path = Path(db_tmp.name) / "canary.db"
        home_dir = Path(home_tmp.name)
        env = build_gateway_env(
            owner_user_id=owner_user_id,
            gateway_port=gateway_port,
            http_port=http_port,
            gateway_token=gateway_token,
            db_path=db_path,
            home_dir=home_dir,
            tools_dir=Path(tools_tmp.name),
            channels_dir=Path(channels_tmp.name),
            mock_llm_url=mock_llm_url,
            secrets_master_key=secrets_master_key,
            extra_env=extra_gateway_env,
        )
        gateway_proc = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "ironclaw"), "--no-onboard"],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            env=env,
        )
        # Same deadlock guard as mock_llm above — drain ironclaw's
        # stdout/stderr so a chatty `RUST_LOG=info` doesn't fill the pipe
        # buffer and freeze the request handler mid-response.
        if log_dir is not None and gateway_proc.stdout is not None:
            log_dir.mkdir(parents=True, exist_ok=True)
            _drain_to_file(gateway_proc.stdout, log_dir / "gateway.log")
        base_url = f"http://127.0.0.1:{gateway_port}"
        await wait_for_ready(f"{base_url}/api/health", timeout=60.0)

        # Pin the LLM provider via the settings API. Setting LLM_BACKEND /
        # LLM_BASE_URL / LLM_MODEL via env is not enough — IronClaw's DB
        # setting takes priority over env, and the freshly-seeded DB
        # defaults llm_backend to `nearai`, so the env config is ignored
        # and the agent attempts an interactive NearAI auth flow that
        # never completes in CI. Mirrors the pattern documented in
        # tests/e2e/CLAUDE.md.
        await _pin_mock_llm_settings(base_url, gateway_token, mock_llm_url)
        return GatewayStack(
            base_url=base_url,
            gateway_token=gateway_token,
            db_path=db_path,
            mock_llm_url=mock_llm_url,
            gateway_proc=gateway_proc,
            mock_llm_proc=mock_llm_proc,
            tempdirs=tempdirs,
            http_url=f"http://127.0.0.1:{http_port}",
            channels_dir=str(channels_tmp.name),
        )
    except Exception:
        stop_process(mock_llm_proc)
        for tempdir in tempdirs:
            tempdir.cleanup()
        raise


def stop_gateway_stack(stack: GatewayStack) -> None:
    stop_process(stack.gateway_proc)
    stop_process(stack.mock_llm_proc)
    for tempdir in stack.tempdirs:
        tempdir.cleanup()
