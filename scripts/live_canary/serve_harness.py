"""Hermetic launcher for live-canary ``ironclaw serve`` processes."""

from __future__ import annotations

import subprocess
import time
from pathlib import Path

from scripts.live_canary.common import stop_process, wait_for_ready


class ServeLaunchError(RuntimeError):
    """Raised when a serve process does not become ready within its bound."""


def serve_command(binary: Path, port: int) -> list[str]:
    """Return the shipping CLI invocation used by retained Python canaries."""
    return [
        str(binary),
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        str(port),
    ]


async def start_serve(
    *,
    binary: Path,
    port: int,
    env: dict[str, str],
    output_dir: Path,
    workspace_dir: Path | None = None,
    readiness_timeout: float = 90.0,
    log_stem: str = "ironclaw-serve",
) -> tuple[subprocess.Popen[str], str]:
    """Start ``ironclaw serve`` with bounded readiness and captured logs."""
    base_url = f"http://127.0.0.1:{port}"
    output_dir.mkdir(parents=True, exist_ok=True)
    workspace = workspace_dir or output_dir / "workspace"
    workspace.mkdir(parents=True, exist_ok=True)
    stdout_path = output_dir / f"{log_stem}.stdout.log"
    stderr_path = output_dir / f"{log_stem}.stderr.log"
    separator = (
        f"\n--- ironclaw serve start "
        f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} ---\n"
    )

    with (
        stdout_path.open("a", encoding="utf-8") as stdout,
        stderr_path.open("a", encoding="utf-8") as stderr,
    ):
        stdout.write(separator)
        stderr.write(separator)
        stdout.flush()
        stderr.flush()
        proc = subprocess.Popen(
            serve_command(binary, port),
            stdin=subprocess.DEVNULL,
            stdout=stdout,
            stderr=stderr,
            text=True,
            env=env,
            cwd=workspace,
        )

    try:
        await wait_for_ready(
            f"{base_url}/api/health",
            timeout=readiness_timeout,
        )
    except Exception as exc:
        stop_process(proc)
        tail = ""
        if stderr_path.exists():
            tail = "\n".join(
                stderr_path.read_text(
                    encoding="utf-8",
                    errors="replace",
                ).splitlines()[-80:]
            )
        raise ServeLaunchError(
            f"ironclaw serve did not become healthy at {base_url}: {exc}\n{tail}"
        ) from exc

    return proc, base_url
