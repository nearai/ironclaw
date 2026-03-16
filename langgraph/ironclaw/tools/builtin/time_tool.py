"""Time tool — mirrors src/tools/builtin/time.rs."""

from __future__ import annotations

import time
from datetime import datetime, timezone
from typing import Any
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from ironclaw.tools.registry import ToolDefinition, ToolResult


async def _execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    tz_name = params.get("timezone", "UTC")

    try:
        tz = ZoneInfo(tz_name)
    except ZoneInfoNotFoundError:
        duration = (time.monotonic() - start) * 1000
        return ToolResult(
            output=f"Unknown timezone: {tz_name}",
            duration_ms=duration,
            error=True,
        )

    now = datetime.now(tz)
    output = now.isoformat()
    duration = (time.monotonic() - start) * 1000
    return ToolResult(output=output, duration_ms=duration)


time_tool = ToolDefinition(
    name="current_time",
    description="Return the current date and time in ISO 8601 format.",
    parameters_schema={
        "type": "object",
        "properties": {
            "timezone": {
                "type": "string",
                "description": "IANA timezone name (e.g. 'America/New_York', 'UTC'). Defaults to UTC.",
            },
        },
        "required": [],
    },
    execute=_execute,
    requires_approval=False,
    requires_sanitization=False,
)
