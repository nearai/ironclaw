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


async def start_gateway_stack(
    *,
    venv_dir: Path,
    owner_user_id: str,
    secrets_master_key: str | None = None,
    temp_prefix: str,
    gateway_token_prefix: str,
    extra_gateway_env: dict[str, str] | None = None,
    oauth_proxy: bool = False,
) -> GatewayStack:
    secrets_master_key = secrets_master_key or generate_secrets_master_key()
    python = venv_python(venv_dir)
    mock_llm_port = reserve_loopback_port()
    mock_llm_proc = subprocess.Popen(
        [str(python), str(E2E_DIR / "mock_llm.py"), "--port", str(mock_llm_port)],
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
        base_url = f"http://127.0.0.1:{gateway_port}"
        await wait_for_ready(f"{base_url}/api/health", timeout=60.0)
        return GatewayStack(
            base_url=base_url,
            gateway_token=gateway_token,
            db_path=db_path,
            mock_llm_url=mock_llm_url,
            gateway_proc=gateway_proc,
            mock_llm_proc=mock_llm_proc,
            tempdirs=tempdirs,
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
