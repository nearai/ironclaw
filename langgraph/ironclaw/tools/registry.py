"""Tool registry — mirrors src/tools/registry.rs.

Tools are registered with a name, description, parameters schema, and an
async callable.  The registry is used by the LLM node to populate
``available_tools`` and by the tool-execution node to dispatch calls.
"""

from __future__ import annotations

import asyncio
import time
from dataclasses import dataclass, field
from typing import Any, Awaitable, Callable


@dataclass
class ToolResult:
    """Result of a tool execution."""

    output: str
    duration_ms: float
    error: bool = False


@dataclass
class ToolDefinition:
    """A registered tool."""

    name: str
    description: str
    parameters_schema: dict[str, Any]
    execute: Callable[..., Awaitable[ToolResult]]
    requires_approval: bool = False
    requires_sanitization: bool = True


class ToolRegistry:
    """
    Thread-safe async tool registry.

    Usage
    -----
    >>> registry = ToolRegistry()
    >>> registry.register(my_tool_def)
    >>> result = await registry.execute("echo", {"text": "hi"}, ctx)
    """

    def __init__(self) -> None:
        self._tools: dict[str, ToolDefinition] = {}
        self._lock = asyncio.Lock()

    def register(self, tool: ToolDefinition) -> None:
        """Register a tool (thread-safe, sync)."""
        self._tools[tool.name] = tool

    async def register_async(self, tool: ToolDefinition) -> None:
        """Register a tool (async)."""
        async with self._lock:
            self._tools[tool.name] = tool

    def get(self, name: str) -> ToolDefinition | None:
        return self._tools.get(name)

    def list_tools(self) -> list[ToolDefinition]:
        return list(self._tools.values())

    def to_llm_format(self) -> list[dict[str, Any]]:
        """Return tool definitions in the format expected by LangChain LLMs."""
        return [
            {
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters_schema,
                },
            }
            for t in self._tools.values()
        ]

    async def execute(
        self,
        name: str,
        params: dict[str, Any],
        ctx: Any = None,
    ) -> ToolResult:
        """Execute a registered tool by name."""
        tool = self._tools.get(name)
        if tool is None:
            return ToolResult(
                output=f"Tool '{name}' not found",
                duration_ms=0,
                error=True,
            )

        start = time.monotonic()
        try:
            result = await tool.execute(params, ctx)
            return result
        except Exception as exc:  # noqa: BLE001
            duration = (time.monotonic() - start) * 1000
            return ToolResult(
                output=f"Tool execution failed: {exc}",
                duration_ms=duration,
                error=True,
            )
