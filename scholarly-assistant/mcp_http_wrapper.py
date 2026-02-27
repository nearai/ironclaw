#!/usr/bin/env python3
"""HTTP wrapper for stdio-based MCP servers.

This wrapper allows IronClaw to communicate with Python MCP servers
that use stdio transport by exposing them via HTTP endpoints.
"""
import asyncio
import json
import subprocess
import sys
from pathlib import Path
from aiohttp import web


class McpHttpWrapper:
    """Wraps a stdio MCP server with an HTTP interface."""

    def __init__(self, command, args, working_dir=None):
        self.command = command
        self.args = args
        self.working_dir = working_dir

    async def handle_request(self, request):
        """Handle an incoming HTTP request and forward to MCP server via stdio."""
        try:
            data = await request.json()

            # Start the MCP server process
            proc = await asyncio.create_subprocess_exec(
                self.command,
                *self.args,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                cwd=self.working_dir
            )

            # Send request to stdio
            request_line = json.dumps(data) + '\n'
            stdout, stderr = await proc.communicate(request_line.encode())

            if proc.returncode != 0:
                error_msg = stderr.decode() if stderr else "Process failed"
                print(f"ERROR: {error_msg}", file=sys.stderr)
                return web.json_response(
                    {
                        "jsonrpc": "2.0",
                        "id": data.get("id"),
                        "error": {
                            "code": -32603,
                            "message": error_msg
                        }
                    },
                    status=500
                )

            # Parse response
            try:
                response = json.loads(stdout.decode())
                return web.json_response(response)
            except json.JSONDecodeError as e:
                print(f"JSON Parse Error: {e}", file=sys.stderr)
                print(f"Raw stdout: {stdout.decode()}", file=sys.stderr)
                return web.json_response(
                    {
                        "jsonrpc": "2.0",
                        "id": data.get("id"),
                        "error": {
                            "code": -32700,
                            "message": f"Parse error: {e}"
                        }
                    },
                    status=500
                )

        except Exception as e:
            print(f"Exception: {e}", file=sys.stderr)
            return web.json_response(
                {
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32603,
                        "message": str(e)
                    }
                },
                status=500
            )


async def main():
    """Start the HTTP wrapper server."""
    print("Starting MCP HTTP Wrapper...")

    # Configuration - update paths as needed
    semantic_scholar_path = Path.home() / "semanticscholar-MCP-Server"

    # Check if Semantic Scholar server exists
    if not semantic_scholar_path.exists():
        print(f"WARNING: Semantic Scholar path not found: {semantic_scholar_path}")
        print("  Clone it with: git clone https://github.com/JackKuo666/semanticscholar-MCP-Server.git")

    # Semantic Scholar server
    semantic_scholar = McpHttpWrapper(
        'python',
        ['semantic_scholar_server.py'],
        working_dir=str(semantic_scholar_path) if semantic_scholar_path.exists() else None
    )

    # ArXiv server (if installed via uv)
    arxiv = McpHttpWrapper('uv', ['tool', 'run', 'arxiv-mcp-server'])

    # Create web application
    app = web.Application()
    app.router.add_post('/semantic-scholar', semantic_scholar.handle_request)
    app.router.add_post('/arxiv', arxiv.handle_request)

    # Health check endpoint
    async def health(request):
        return web.json_response({
            "status": "ok",
            "servers": {
                "semantic_scholar": semantic_scholar_path.exists(),
                "arxiv": True  # Assume available if uv is installed
            }
        })

    app.router.add_get('/health', health)

    # Start server
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, 'localhost', 3100)
    await site.start()

    print("✓ MCP HTTP wrapper running on http://localhost:3100")
    print("")
    print("Endpoints:")
    print("  - Semantic Scholar: POST http://localhost:3100/semantic-scholar")
    print("  - ArXiv:           POST http://localhost:3100/arxiv")
    print("  - Health check:     GET http://localhost:3100/health")
    print("")
    print("Configure IronClaw with ~/.ironclaw/mcp-servers.json")
    print("")
    print("Press Ctrl+C to stop")

    # Keep running
    try:
        await asyncio.Event().wait()
    except KeyboardInterrupt:
        print("\nShutting down...")


if __name__ == '__main__':
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        sys.exit(0)
