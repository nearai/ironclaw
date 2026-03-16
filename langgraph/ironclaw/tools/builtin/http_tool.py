"""HTTP tools — mirrors src/tools/builtin/http.rs."""

from __future__ import annotations

import time
from typing import Any

import httpx

from ironclaw.tools.registry import ToolDefinition, ToolResult

_TIMEOUT = httpx.Timeout(30.0)
_MAX_RESPONSE_BYTES = 1_000_000  # 1 MB


async def _get_execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    url = params.get("url", "")
    headers = params.get("headers", {})
    try:
        async with httpx.AsyncClient(timeout=_TIMEOUT, follow_redirects=True) as client:
            resp = await client.get(url, headers=headers)
            body = resp.text[:_MAX_RESPONSE_BYTES]
            duration = (time.monotonic() - start) * 1000
            return ToolResult(
                output=f"Status: {resp.status_code}\n\n{body}",
                duration_ms=duration,
            )
    except Exception as exc:  # noqa: BLE001
        return ToolResult(output=f"HTTP GET failed: {exc}", duration_ms=0, error=True)


http_get_tool = ToolDefinition(
    name="http_get",
    description="Perform an HTTP GET request and return the response body.",
    parameters_schema={
        "type": "object",
        "properties": {
            "url": {"type": "string", "description": "URL to request"},
            "headers": {
                "type": "object",
                "description": "Optional HTTP headers",
                "additionalProperties": {"type": "string"},
            },
        },
        "required": ["url"],
    },
    execute=_get_execute,
    requires_approval=False,
    requires_sanitization=True,
)


async def _post_execute(params: dict[str, Any], ctx: Any = None) -> ToolResult:
    start = time.monotonic()
    url = params.get("url", "")
    body = params.get("body", "")
    headers = params.get("headers", {})
    content_type = params.get("content_type", "application/json")
    headers.setdefault("Content-Type", content_type)
    try:
        async with httpx.AsyncClient(timeout=_TIMEOUT, follow_redirects=True) as client:
            resp = await client.post(url, content=body.encode(), headers=headers)
            resp_body = resp.text[:_MAX_RESPONSE_BYTES]
            duration = (time.monotonic() - start) * 1000
            return ToolResult(
                output=f"Status: {resp.status_code}\n\n{resp_body}",
                duration_ms=duration,
            )
    except Exception as exc:  # noqa: BLE001
        return ToolResult(output=f"HTTP POST failed: {exc}", duration_ms=0, error=True)


http_post_tool = ToolDefinition(
    name="http_post",
    description="Perform an HTTP POST request with a body and return the response.",
    parameters_schema={
        "type": "object",
        "properties": {
            "url": {"type": "string", "description": "URL to request"},
            "body": {"type": "string", "description": "Request body"},
            "content_type": {
                "type": "string",
                "description": "Content-Type header (default: application/json)",
            },
            "headers": {
                "type": "object",
                "description": "Optional additional HTTP headers",
                "additionalProperties": {"type": "string"},
            },
        },
        "required": ["url", "body"],
    },
    execute=_post_execute,
    requires_approval=True,
    requires_sanitization=True,
)
