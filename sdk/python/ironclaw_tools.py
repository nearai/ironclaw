"""IronClaw Programmatic Tool Calling SDK for container scripts.

Thin wrapper using only Python stdlib. Reads connection details from
environment variables injected by the orchestrator:

    IRONCLAW_ORCHESTRATOR_URL  - Base URL of the orchestrator API
    IRONCLAW_JOB_ID            - UUID of the current job
    IRONCLAW_WORKER_TOKEN      - Bearer token scoped to this job

Usage:
    from ironclaw_tools import call_tool, shell, read_file, write_file, http_get

    # Call any registered tool by name
    result = call_tool("echo", {"message": "hello"})
    print(result)  # "hello"

    # Convenience wrappers
    output = shell("ls -la")
    content = read_file("/workspace/README.md")
    write_file("/workspace/output.txt", "results here")
    body = http_get("https://api.example.com/data")
"""

import json
import os
import urllib.request
import urllib.error


def _env(name):
    """Get a required environment variable."""
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(
            f"Missing required environment variable: {name}. "
            "This SDK must be run inside an IronClaw container."
        )
    return value


def _base_url():
    """Build the base URL for tool call requests."""
    orchestrator = _env("IRONCLAW_ORCHESTRATOR_URL").rstrip("/")
    job_id = _env("IRONCLAW_JOB_ID")
    return f"{orchestrator}/worker/{job_id}"


def _token():
    """Get the bearer token."""
    return _env("IRONCLAW_WORKER_TOKEN")


def call_tool(name, params=None, timeout_secs=None):
    """Call a tool on the orchestrator by name.

    Args:
        name: Tool name (e.g., "echo", "shell", "read_file").
        params: Dictionary of parameters to pass to the tool.
        timeout_secs: Optional timeout in seconds (max 300).

    Returns:
        Tool output as a string.

    Raises:
        RuntimeError: If the tool call fails.
    """
    url = f"{_base_url()}/tools/call"
    body = {
        "tool_name": name,
        "parameters": params or {},
    }
    if timeout_secs is not None:
        body["timeout_secs"] = min(int(timeout_secs), 300)

    data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {_token()}",
        },
        method="POST",
    )

    try:
        with urllib.request.urlopen(req, timeout=(timeout_secs or 60) + 5) as resp:
            result = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        body_text = e.read().decode("utf-8", errors="replace") if e.fp else ""
        raise RuntimeError(
            f"Tool call failed: HTTP {e.code}: {body_text}"
        ) from None
    except urllib.error.URLError as e:
        raise RuntimeError(f"Connection to orchestrator failed: {e.reason}") from None

    if not result.get("success"):
        raise RuntimeError(f"Tool '{name}' failed: {result.get('error', 'unknown error')}")

    return result.get("output", "")


def shell(command, timeout_secs=60):
    """Execute a shell command via the orchestrator.

    Args:
        command: Shell command string to execute.
        timeout_secs: Timeout in seconds (default 60).

    Returns:
        Command output as a string.
    """
    return call_tool("shell", {"command": command}, timeout_secs=timeout_secs)


def read_file(path):
    """Read a file via the orchestrator.

    Args:
        path: Absolute path to the file.

    Returns:
        File contents as a string.
    """
    return call_tool("read_file", {"path": path})


def write_file(path, content):
    """Write a file via the orchestrator.

    Args:
        path: Absolute path to write to.
        content: String content to write.

    Returns:
        Write confirmation message.
    """
    return call_tool("write_file", {"path": path, "content": content})


def http_get(url, headers=None, timeout_secs=30):
    """Make an HTTP GET request via the orchestrator's HTTP tool.

    Args:
        url: URL to fetch.
        headers: Optional dictionary of headers.
        timeout_secs: Timeout in seconds (default 30).

    Returns:
        Response body as a string.
    """
    params = {"url": url, "method": "GET"}
    if headers:
        params["headers"] = headers
    return call_tool("http", params, timeout_secs=timeout_secs)
