"""File tools — mirrors src/tools/builtin/file.rs."""

from __future__ import annotations

import os
import time
from pathlib import Path
from typing import Any

from ironclaw.tools.registry import ToolDefinition, ToolResult

# Workspace root — can be overridden via WORKSPACE_ROOT env var
WORKSPACE_ROOT = Path(os.environ.get("WORKSPACE_ROOT", Path.home() / ".ironclaw" / "workspace"))


def _safe_path(path_str: str) -> Path | None:
    """Resolve a path relative to the workspace root, rejecting traversal attacks."""
    try:
        resolved = (WORKSPACE_ROOT / path_str).resolve()
        resolved.relative_to(WORKSPACE_ROOT.resolve())
        return resolved
    except ValueError:
        return None


# ---------------------------------------------------------------------------
# read_file
# ---------------------------------------------------------------------------


async def _read_execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    path_str = params.get("path", "")
    path = _safe_path(path_str)
    if path is None:
        return ToolResult(output=f"Path traversal rejected: {path_str}", duration_ms=0, error=True)
    try:
        content = path.read_text(encoding="utf-8")
        duration = (time.monotonic() - start) * 1000
        return ToolResult(output=content, duration_ms=duration)
    except FileNotFoundError:
        return ToolResult(output=f"File not found: {path_str}", duration_ms=0, error=True)
    except Exception as exc:  # noqa: BLE001
        return ToolResult(output=f"Read error: {exc}", duration_ms=0, error=True)


read_file_tool = ToolDefinition(
    name="read_file",
    description="Read the contents of a file in the workspace.",
    parameters_schema={
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Path relative to the workspace root"},
        },
        "required": ["path"],
    },
    execute=_read_execute,
    requires_approval=False,
    requires_sanitization=True,
)


# ---------------------------------------------------------------------------
# write_file
# ---------------------------------------------------------------------------


async def _write_execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    path_str = params.get("path", "")
    content = params.get("content", "")
    path = _safe_path(path_str)
    if path is None:
        return ToolResult(output=f"Path traversal rejected: {path_str}", duration_ms=0, error=True)
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        duration = (time.monotonic() - start) * 1000
        return ToolResult(output=f"Written {len(content)} bytes to {path_str}", duration_ms=duration)
    except Exception as exc:  # noqa: BLE001
        return ToolResult(output=f"Write error: {exc}", duration_ms=0, error=True)


write_file_tool = ToolDefinition(
    name="write_file",
    description="Write content to a file in the workspace. Creates parent directories as needed.",
    parameters_schema={
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Path relative to the workspace root"},
            "content": {"type": "string", "description": "Content to write"},
        },
        "required": ["path", "content"],
    },
    execute=_write_execute,
    requires_approval=True,
    requires_sanitization=False,
)


# ---------------------------------------------------------------------------
# list_dir
# ---------------------------------------------------------------------------


async def _list_execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    path_str = params.get("path", "")
    path = _safe_path(path_str) if path_str else WORKSPACE_ROOT
    if path is None:
        return ToolResult(output=f"Path traversal rejected: {path_str}", duration_ms=0, error=True)
    try:
        if not path.exists():
            return ToolResult(output=f"Directory not found: {path_str}", duration_ms=0, error=True)
        entries = sorted(path.iterdir(), key=lambda p: (p.is_file(), p.name))
        lines = []
        for entry in entries:
            kind = "F" if entry.is_file() else "D"
            lines.append(f"[{kind}] {entry.name}")
        output = "\n".join(lines) if lines else "(empty)"
        duration = (time.monotonic() - start) * 1000
        return ToolResult(output=output, duration_ms=duration)
    except Exception as exc:  # noqa: BLE001
        return ToolResult(output=f"List error: {exc}", duration_ms=0, error=True)


list_dir_tool = ToolDefinition(
    name="list_dir",
    description="List the contents of a directory in the workspace.",
    parameters_schema={
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Path relative to the workspace root. Defaults to workspace root.",
            },
        },
        "required": [],
    },
    execute=_list_execute,
    requires_approval=False,
    requires_sanitization=True,
)
