# Scholarly Research Setup for IronClaw

This guide shows how to set up IronClaw as a PhD-level research assistant with access to academic paper databases.

## Quick Start

1. Install MCP servers for paper search
2. Configure IronClaw to use them
3. Test the integration
4. Start researching!

## Step 1: Install MCP Servers

### Semantic Scholar MCP Server

Provides access to 200M+ papers in Semantic Scholar's database.

**Install:**
```bash
# Install dependencies
pip install semanticscholar mcp

# Clone the repository
git clone https://github.com/JackKuo666/semanticscholar-MCP-Server.git
cd semanticscholar-MCP-Server

# Note the full path - you'll need it for config
pwd
```

**Tools provided:**
- `search_papers` - Search papers by query, year range, and fields
- `get_paper_details` - Get comprehensive paper information
- `get_author_details` - Get author profiles with h-index and citations
- `get_citations` - Fetch citations and references for papers

### ArXiv MCP Server

Provides access to arXiv research papers with download capability.

**Install via uv (recommended):**
```bash
uv tool install arxiv-mcp-server
```

**Tools provided:**
- `search_papers` - Query arXiv with date ranges and subject filters
- `download_paper` - Download papers by arXiv ID
- `list_papers` - View locally stored papers
- `read_paper` - Access content from downloaded papers

## Step 2: HTTP Wrapper for Python MCP Servers

Since IronClaw's MCP client uses HTTP transport and these servers use stdio, we need a wrapper.

Create `mcp_http_wrapper.py`:

```python
#!/usr/bin/env python3
"""HTTP wrapper for stdio-based MCP servers."""
import asyncio
import json
import subprocess
import sys
from pathlib import Path
from aiohttp import web

class McpHttpWrapper:
    def __init__(self, command, args, working_dir=None):
        self.command = command
        self.args = args
        self.working_dir = working_dir

    async def handle_request(self, request):
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
                return web.json_response(
                    {"error": {"code": -32603, "message": error_msg}},
                    status=500
                )

            # Parse response
            try:
                response = json.loads(stdout.decode())
                return web.json_response(response)
            except json.JSONDecodeError as e:
                return web.json_response(
                    {"error": {"code": -32700, "message": f"Parse error: {e}"}},
                    status=500
                )

        except Exception as e:
            return web.json_response(
                {"error": {"code": -32603, "message": str(e)}},
                status=500
            )

async def main():
    # Configuration - update paths as needed
    semantic_scholar_path = Path.home() / "semanticscholar-MCP-Server"

    # Semantic Scholar server
    semantic_scholar = McpHttpWrapper(
        'python',
        ['semantic_scholar_server.py'],
        working_dir=str(semantic_scholar_path) if semantic_scholar_path.exists() else None
    )

    # ArXiv server (if installed via uv)
    arxiv = McpHttpWrapper('uv', ['tool', 'run', 'arxiv-mcp-server'])

    app = web.Application()
    app.router.add_post('/semantic-scholar', semantic_scholar.handle_request)
    app.router.add_post('/arxiv', arxiv.handle_request)

    # Health check endpoint
    async def health(request):
        return web.json_response({"status": "ok"})

    app.router.add_get('/health', health)

    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, 'localhost', 3100)
    await site.start()

    print("✓ MCP HTTP wrapper running on http://localhost:3100")
    print("  - Semantic Scholar: http://localhost:3100/semantic-scholar")
    print("  - ArXiv: http://localhost:3100/arxiv")
    print("  - Health check: http://localhost:3100/health")
    print("\nPress Ctrl+C to stop")

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
```

Install dependencies and run:
```bash
pip install aiohttp
python mcp_http_wrapper.py
```

## Step 3: Configure IronClaw

Create or edit `~/.ironclaw/mcp-servers.json`:

```json
{
  "servers": [
    {
      "name": "semantic_scholar",
      "url": "http://localhost:3100/semantic-scholar",
      "enabled": true,
      "description": "Semantic Scholar paper database - 200M+ papers"
    },
    {
      "name": "arxiv",
      "url": "http://localhost:3100/arxiv",
      "enabled": true,
      "description": "ArXiv preprint repository - physics, CS, math"
    }
  ],
  "schema_version": 1
}
```

## Step 4: Test the Integration

```bash
# Test the HTTP wrapper
curl http://localhost:3100/health

# Test Semantic Scholar endpoint
curl http://localhost:3100/semantic-scholar \
  -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"tools/list","id":1}'

# Start ironclaw
ironclaw run
```

In the TUI, you should see tools prefixed with server names:
- `semantic_scholar_search_papers`
- `semantic_scholar_get_paper_details`
- `arxiv_search_papers`
- `arxiv_download_paper`

## Step 5: PhD Research Workflows

### Literature Review

```
> Search Semantic Scholar for papers on "large language models" from 2022-2024
> Get details for paper [paper-id]
> Write a summary to workspace/papers/[author-year].md
```

### Daily Paper Monitoring

Create a cron routine:
```bash
ironclaw routine create daily-papers \
  --trigger cron "0 9 * * *" \
  --action "Search for new papers on [your research topic] from last week"
```

### Paper Analysis

```
> Download ArXiv paper 2401.12345
> Extract key methodology and results
> Store analysis in workspace/papers/
```

## Advanced: Literature Review Skill

Create `~/.ironclaw/skills/literature-review.skill.md`:

```yaml
---
name: literature-review
version: 0.1.0
description: Systematic literature review assistant
activation:
  keywords: ["literature", "review", "papers", "research"]
  patterns: ["search.*papers", "find.*research", "literature review"]
  max_context_tokens: 3000
---

# Literature Review Assistant

## Search Strategy

1. Use semantic_scholar_search_papers for broad coverage (200M+ papers)
2. Use arxiv_search_papers for recent preprints
3. Filter by year range and field of study
4. Check citation counts for influence

## Documentation

- Store summaries in workspace/papers/[author-year].md
- Track citations in workspace/bibliography/references.json
- Note research gaps in workspace/notes/gaps.md

## Paper Selection Criteria

- Relevance to research question
- Citation count (indicates impact)
- Author h-index (indicates credibility)
- Recency (prioritize recent work)
- Methodology rigor

## Synthesis Process

1. Group papers by theme/methodology
2. Identify common findings
3. Track contradictory results
4. Note research gaps
5. Map citation relationships
```

## Troubleshooting

### HTTP Wrapper Issues

```bash
# Check if wrapper is running
curl http://localhost:3100/health

# View wrapper logs
python mcp_http_wrapper.py  # Check console output

# Test endpoint directly
curl -X POST http://localhost:3100/semantic-scholar \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"tools/list","id":1}'
```

### IronClaw Not Seeing Tools

1. Verify config: `cat ~/.ironclaw/mcp-servers.json`
2. Check ironclaw logs: `RUST_LOG=ironclaw::tools::mcp=debug ironclaw run`
3. Ensure servers are enabled in config
4. Restart ironclaw after config changes

### Rate Limiting

Semantic Scholar API limits:
- Free tier: 100 requests/5 minutes
- Get API key: https://www.semanticscholar.org/product/api

## Next Steps

### Build Custom Tools

1. **PDF Parser WASM Tool** - Extract text from papers
2. **Bibliography Manager WASM Tool** - Manage citations
3. **Citation Graph Tool** - Visualize relationships
4. **Literature Matrix Tool** - Compare papers

### Workspace Organization

```
~/.ironclaw/workspace/
├── papers/              # Paper summaries
├── bibliography/        # Citations
├── notes/              # Research notes
├── experiments/        # Results
└── writing/           # Thesis chapters
```

## Resources

- [Semantic Scholar API](https://api.semanticscholar.org/)
- [ArXiv API](https://info.arxiv.org/help/api/index.html)
- [MCP Spec](https://spec.modelcontextprotocol.io/)
- [IronClaw GitHub](https://github.com/nearai/ironclaw)

---

*Setup guide by Andy - The Puppet Master*
*For Joaquín's PhD project*
