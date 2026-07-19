from __future__ import annotations

import asyncio
import json
import os
import shlex
import signal
import socket
import subprocess
import sys
import time
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


def run(
    cmd: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None
) -> None:
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
    try:
        run(cmd, cwd=E2E_DIR)
    except subprocess.CalledProcessError:
        if resolved != "with-deps":
            raise
        print(
            "[live-canary] playwright install --with-deps failed; "
            "retrying browser-only install",
            flush=True,
        )
        run([str(python), "-m", "playwright", "install", "chromium"], cwd=E2E_DIR)


def env_str(name: str, default: str | None = None) -> str | None:
    value = os.environ.get(name, default)
    if value is None:
        return None
    value = value.strip()
    return value or None


def env_secret(name: str) -> str | None:
    """Read a secret, preferring its mode-0600 ``<NAME>_PATH`` file."""
    path = env_str(f"{name}_PATH")
    if path:
        try:
            value = Path(path).read_text(encoding="utf-8")
        except OSError:
            return None
        value = value.rstrip("\r\n")
        return value or None
    return env_str(name)


def reserve_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


async def wait_for_ready(
    url: str, timeout: float = 60.0, interval: float = 0.5
) -> None:
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
