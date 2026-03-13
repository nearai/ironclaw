# Configuring MCP Servers

IronClaw connects to [Model Context Protocol](https://modelcontextprotocol.io/) servers, giving your agent access to external tools and data sources without writing custom integrations.

## Quick Start

```bash
# Add an HTTP server
ironclaw mcp add notion https://mcp.notion.com --client-id <your-client-id>

# Add a stdio server (spawns a local process)
ironclaw mcp add filesystem --transport stdio \
  --command npx --arg @anthropic/mcp-filesystem --arg /home/user/documents

# Add a Unix socket server
ironclaw mcp add myserver --transport unix --socket /tmp/mcp.sock

# Test connectivity
ironclaw mcp test notion

# List configured servers
ironclaw mcp list

# Remove a server
ironclaw mcp remove notion
```

## Transports

| Transport | Use case | Example |
|-----------|----------|---------|
| **HTTP** (default) | Remote/hosted servers with a public URL | `ironclaw mcp add name https://mcp.example.com` |
| **stdio** | Local servers that run as a subprocess | `ironclaw mcp add name --transport stdio --command uvx --arg some-mcp-server` |
| **Unix** | Local servers listening on a Unix domain socket | `ironclaw mcp add name --transport unix --socket /tmp/mcp.sock` |

### HTTP with OAuth

Many hosted MCP servers require OAuth 2.1 authentication. IronClaw implements the [MCP Authorization spec](https://spec.modelcontextprotocol.io/specification/2025-03-26/basic/authorization/) with PKCE:

```bash
# Add with OAuth credentials
ironclaw mcp add github https://mcp.github.com \
  --client-id YOUR_CLIENT_ID \
  --scopes "repo,read:org"

# Authenticate (opens browser for consent)
ironclaw mcp auth github
```

OAuth tokens are stored securely via IronClaw's secrets store and refreshed automatically.

### stdio with Environment Variables

Stdio servers often need API keys or configuration via environment variables:

```bash
ironclaw mcp add postgres --transport stdio \
  --command npx --arg @anthropic/mcp-postgres \
  --env DATABASE_URL=postgresql://localhost/mydb

ironclaw mcp add fulcra --transport stdio \
  --command uvx --arg fulcra-context-mcp \
  --env FULCRA_CLIENT_ID=your_client_id \
  --env FULCRA_CLIENT_SECRET=your_secret
```

## Configuration File

Server configs are stored in `~/.ironclaw/mcp-servers.json`:

```json
{
  "servers": {
    "filesystem": {
      "name": "filesystem",
      "url": "",
      "transport": {
        "transport": "stdio",
        "command": "npx",
        "args": ["@anthropic/mcp-filesystem", "/home/user/documents"],
        "env": {}
      },
      "enabled": true,
      "description": "Sandboxed file access"
    },
    "notion": {
      "name": "notion",
      "url": "https://mcp.notion.com",
      "oauth": {
        "client_id": "your-client-id",
        "scopes": ["read", "write"]
      },
      "enabled": true
    }
  }
}
```

You can edit this file directly, or use `ironclaw mcp add` / `ironclaw mcp remove` to manage it.

## Custom Headers

For servers that use API key authentication instead of OAuth:

```bash
ironclaw mcp add myapi https://api.example.com/mcp \
  --header "Authorization:Bearer sk-your-key" \
  --header "X-Custom:value"
```

## Example Servers

A few community servers that work well with IronClaw:

| Server | Transport | What it does |
|--------|-----------|-------------|
| [@anthropic/mcp-filesystem](https://www.npmjs.com/package/@anthropic/mcp-filesystem) | stdio | Sandboxed file system access |
| [@anthropic/mcp-postgres](https://www.npmjs.com/package/@anthropic/mcp-postgres) | stdio | PostgreSQL queries |
| [fulcra-context-mcp](https://github.com/fulcradynamics/fulcra-context-mcp) | stdio | Personal health and context data (biometrics, sleep, activity, calendar) |

Browse more servers at:
- [MCP Server Registry](https://github.com/modelcontextprotocol/servers) (official)
- [awesome-mcp-servers](https://github.com/punkpeye/awesome-mcp-servers) (community)
- [Glama MCP Directory](https://glama.ai/mcp/servers)

## MCP vs WASM Tools

IronClaw supports both MCP servers and native WASM tools. Use MCP when you want to connect to an existing server or need network access. Use WASM when you need sandboxed, low-latency tool execution. See `docs/plans/mcp-vs-wasm.md` for a detailed comparison.

## Troubleshooting

```bash
# Check server health
ironclaw mcp test <server-name>

# Re-authenticate an OAuth server
ironclaw mcp auth <server-name>

# Disable without removing
# Edit ~/.ironclaw/mcp-servers.json, set "enabled": false

# Debug logging
RUST_LOG=ironclaw::tools::mcp=debug ironclaw mcp test <server-name>
```
