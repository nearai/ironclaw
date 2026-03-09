# MCP Server Examples

IronClaw supports the [Model Context Protocol](https://modelcontextprotocol.io/) for extending your agent with third-party tools. This guide shows how to add popular MCP servers.

## Quick Start

```bash
# Add a remote MCP server (HTTP transport)
ironclaw mcp add <name> <url>

# Add a local MCP server (stdio transport)
ironclaw mcp add <name> --transport stdio --command <cmd> --arg <arg>

# Test the connection
ironclaw mcp test <name>

# List configured servers
ironclaw mcp list --verbose
```

## Transport Types

| Transport | Use Case |
|-----------|----------|
| **HTTP** | Remote/hosted servers — simplest setup |
| **Stdio** | Local servers — spawns a child process |
| **Unix** | Local servers via Unix domain socket |

## Example Servers

### Fulcra Context (Personal Health & Biometrics)

Access personal context data — biometrics, sleep, activity, calendar, and location — via the [Fulcra Life API](https://github.com/fulcradynamics/fulcra-context-mcp). Data is consent-scoped and user-controlled, making it a natural fit for IronClaw's privacy-first architecture.

**Remote (hosted):**

```bash
ironclaw mcp add fulcra https://mcp.fulcradynamics.com/mcp \
  --description "Personal context data — biometrics, sleep, activity, calendar, location"
```

**Local (stdio):**

```bash
ironclaw mcp add fulcra --transport stdio \
  --command uvx \
  --arg fulcra-context-mcp@latest \
  --description "Personal context data — biometrics, sleep, activity, calendar, location"
```

After adding, authenticate with your Fulcra account:

```bash
ironclaw mcp auth fulcra
```

Available tools: `get_heart_rate`, `get_sleep_analysis`, `get_steps`, `get_active_energy`, `get_calendar_events`, `get_location_history`, and more.

### Filesystem

Read and write files in a sandboxed directory using the [reference filesystem server](https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem).

```bash
ironclaw mcp add filesystem --transport stdio \
  --command npx \
  --arg -y \
  --arg @modelcontextprotocol/server-filesystem \
  --arg /path/to/allowed/dir
```

### PostgreSQL

Query your database with the [Postgres MCP server](https://github.com/modelcontextprotocol/servers/tree/main/src/postgres).

```bash
ironclaw mcp add postgres --transport stdio \
  --command npx \
  --arg -y \
  --arg @modelcontextprotocol/server-postgres \
  --arg "postgresql://user:pass@localhost/mydb"
```

## Configuration File

MCP servers are stored in `~/.ironclaw/mcp-servers.json` (or in the database if configured). Example:

```json
{
  "servers": [
    {
      "name": "fulcra",
      "url": "https://mcp.fulcradynamics.com/mcp",
      "enabled": true,
      "description": "Personal context data — biometrics, sleep, activity, calendar, location"
    }
  ]
}
```

## When to Use MCP vs WASM

See the [decision guide in TOOLS.md](../tools-src/TOOLS.md) for guidance on choosing between MCP servers and native WASM tools. In general:

- **MCP**: Best when a good server already exists, for quick prototypes, or when you need streaming/background connections
- **WASM**: Best for sensitive credentials, core capabilities, or when you need the full sandbox security model

## Finding More Servers

- [MCP Server Registry](https://github.com/modelcontextprotocol/servers) — Official reference implementations
- [Glama MCP Directory](https://glama.ai/mcp/servers) — Community-curated directory
- [awesome-mcp-servers](https://github.com/punkpeye/awesome-mcp-servers) — Community list
