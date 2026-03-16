"""Memory/workspace tools — mirrors src/tools/builtin/memory.rs.

Uses simple in-memory storage by default; replace `_store` with a
database-backed implementation for production.
"""

from __future__ import annotations

import time
from typing import Any

from ironclaw.tools.registry import ToolDefinition, ToolResult

# ---------------------------------------------------------------------------
# In-process memory store (replace with DB-backed store in production)
# ---------------------------------------------------------------------------
_store: dict[str, str] = {}


async def _search_execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    query = params.get("query", "").lower()
    limit = int(params.get("limit", 10))
    results = [
        f"{path}: {content[:200]}"
        for path, content in _store.items()
        if query in content.lower() or query in path.lower()
    ][:limit]
    duration = (time.monotonic() - start) * 1000
    output = "\n---\n".join(results) if results else "No results found."
    return ToolResult(output=output, duration_ms=duration)


memory_search_tool = ToolDefinition(
    name="memory_search",
    description="Search persistent workspace memory using full-text search.",
    parameters_schema={
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "Search query"},
            "limit": {"type": "integer", "description": "Maximum results (default: 10)"},
        },
        "required": ["query"],
    },
    execute=_search_execute,
    requires_approval=False,
    requires_sanitization=True,
)


async def _write_execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    path = params.get("path", "")
    content = params.get("content", "")
    if not path:
        return ToolResult(output="Path is required", duration_ms=0, error=True)
    _store[path] = content
    duration = (time.monotonic() - start) * 1000
    return ToolResult(output=f"Saved {len(content)} bytes to '{path}'", duration_ms=duration)


memory_write_tool = ToolDefinition(
    name="memory_write",
    description="Write content to a named path in workspace memory.",
    parameters_schema={
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Memory path (e.g. 'notes/todo.md')"},
            "content": {"type": "string", "description": "Content to store"},
        },
        "required": ["path", "content"],
    },
    execute=_write_execute,
    requires_approval=False,
    requires_sanitization=False,
)


async def _read_execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    path = params.get("path", "")
    content = _store.get(path)
    duration = (time.monotonic() - start) * 1000
    if content is None:
        return ToolResult(output=f"Not found: '{path}'", duration_ms=duration, error=True)
    return ToolResult(output=content, duration_ms=duration)


memory_read_tool = ToolDefinition(
    name="memory_read",
    description="Read content from a named path in workspace memory.",
    parameters_schema={
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Memory path to read"},
        },
        "required": ["path"],
    },
    execute=_read_execute,
    requires_approval=False,
    requires_sanitization=True,
)
