"""Echo tool — mirrors src/tools/builtin/echo.rs."""

from __future__ import annotations

import time
from typing import Any

from ironclaw.tools.registry import ToolDefinition, ToolResult


async def _execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    text = params.get("text", "")
    duration = (time.monotonic() - start) * 1000
    return ToolResult(output=str(text), duration_ms=duration)


echo_tool = ToolDefinition(
    name="echo",
    description="Echo the provided text back verbatim. Useful for testing.",
    parameters_schema={
        "type": "object",
        "properties": {
            "text": {"type": "string", "description": "Text to echo"},
        },
        "required": ["text"],
    },
    execute=_execute,
    requires_approval=False,
    requires_sanitization=False,
)
