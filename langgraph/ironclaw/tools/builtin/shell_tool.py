"""Shell tool — mirrors src/tools/builtin/shell.rs.

Always requires approval (requires_approval=True).
"""

from __future__ import annotations

import asyncio
import time
from typing import Any

from ironclaw.tools.registry import ToolDefinition, ToolResult

_DEFAULT_TIMEOUT = 30  # seconds
_MAX_OUTPUT_BYTES = 100_000


async def _execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    command = params.get("command", "")
    timeout = float(params.get("timeout", _DEFAULT_TIMEOUT))
    working_dir = params.get("working_dir") or None

    if not command:
        return ToolResult(output="No command provided", duration_ms=0, error=True)

    try:
        proc = await asyncio.create_subprocess_shell(
            command,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            cwd=working_dir,
        )
        try:
            stdout, _ = await asyncio.wait_for(proc.communicate(), timeout=timeout)
        except asyncio.TimeoutError:
            proc.kill()
            return ToolResult(
                output=f"Command timed out after {timeout}s",
                duration_ms=(time.monotonic() - start) * 1000,
                error=True,
            )

        output = stdout.decode(errors="replace")[:_MAX_OUTPUT_BYTES]
        duration = (time.monotonic() - start) * 1000
        exit_code = proc.returncode or 0
        prefix = f"Exit code: {exit_code}\n\n"
        return ToolResult(
            output=prefix + output,
            duration_ms=duration,
            error=exit_code != 0,
        )
    except Exception as exc:  # noqa: BLE001
        return ToolResult(output=f"Shell error: {exc}", duration_ms=0, error=True)


shell_tool = ToolDefinition(
    name="shell",
    description=(
        "Execute a shell command and return its output (stdout + stderr combined). "
        "Always requires user approval."
    ),
    parameters_schema={
        "type": "object",
        "properties": {
            "command": {"type": "string", "description": "Shell command to execute"},
            "timeout": {
                "type": "number",
                "description": "Timeout in seconds (default: 30)",
            },
            "working_dir": {
                "type": "string",
                "description": "Working directory (optional)",
            },
        },
        "required": ["command"],
    },
    execute=_execute,
    requires_approval=True,
    requires_sanitization=True,
)
