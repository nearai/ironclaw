---
title: Model Context Protocol (MCP)
description: Connect your agent to Model Context Protocol (MCP) servers
---

T3Claw connects to [Model Context Protocol](https://modelcontextprotocol.io/) servers, giving your agent access to external tools and data sources without writing custom integrations.

---

## Quickstart

```bash
# Add an HTTP server
t3claw mcp add notion https://mcp.notion.com/mcp --client-id <your-client-id>
```

```bash
# Add a stdio server (spawns a local process)
t3claw mcp add docs --transport stdio \
  --command npx --arg @mintlify/mcp --arg=--docs --arg https://docs.t3claw.com/mcp
```

```bash
# Add a Unix socket server
t3claw mcp add myserver --transport unix --socket /tmp/mcp.sock
```

```bash
# Test connectivity
t3claw mcp test notion
```

```bash
# List configured servers
t3claw mcp list
```

```bash
# Remove a server
t3claw mcp remove notion
```

```bash
# Toggle a server on/off
t3claw mcp toggle notion

# Explicitly disable or enable
t3claw mcp toggle notion --disable
t3claw mcp toggle notion --enable
```

---

## Transports

| Transport          | Use case                                                          | Example                                                                     |
|--------------------|-------------------------------------------------------------------|-----------------------------------------------------------------------------|
| **HTTP** (default) | **Connects** to a remote server over HTTP(S)                      | `t3claw mcp add name https://mcp.example.com`                             |
| **stdio**          | **Spawns** a local server and **connects** to it via stdin/stdout | `t3claw mcp add docs --transport stdio --command npx --arg @mintlify/mcp` |
| **Unix**           | **Connects** to a server on a Unix domain socket                  | `t3claw mcp add name --transport unix --socket /tmp/mcp.sock`             |

### HTTP with OAuth

Many hosted MCP servers require OAuth 2.1 authentication. T3Claw implements the [MCP Authorization spec](https://spec.modelcontextprotocol.io/specification/2025-03-26/basic/authorization/) with PKCE:

```bash
# Add with OAuth credentials
t3claw mcp add notion https://mcp.notion.com/mcp \
  --client-id YOUR_CLIENT_ID \
  --scopes "read,write"

# Authenticate (opens browser for consent)
t3claw mcp auth notion
```

OAuth tokens are stored securely via T3Claw's secrets store and refreshed automatically.

### stdio with Environment Variables

Stdio servers often need API keys or configuration via environment variables:

```bash
t3claw mcp add docs --transport stdio \
  --command npx --arg @mintlify/mcp \
  --env MINTLIFY_API_KEY=your_api_key
```

---

## Configuration File

Server configs are stored in `~/.t3claw/mcp-servers.json`:

```json
{
  "schema_version": 1,
  "servers": [
    {
      "name": "docs",
      "url": "",
      "transport": {
        "transport": "stdio",
        "command": "npx",
        "args": ["@mintlify/mcp", "--docs", "https://docs.t3claw.com/mcp"]
      },
      "enabled": true,
      "description": "T3Claw docs search"
    },
    {
      "name": "notion",
      "url": "https://mcp.notion.com/mcp",
      "oauth": {
        "client_id": "your-client-id",
        "scopes": ["read", "write"]
      },
      "enabled": true
    }
  ]
}
```

You can edit this file directly, or use `t3claw mcp add` / `t3claw mcp remove` to manage it.

---

## Custom Headers

For servers that use API key authentication instead of OAuth:

```bash
t3claw mcp add myapi https://api.example.com/mcp \
  --header "Authorization:Bearer sk-your-key" \
  --header "X-Custom:value"
```

---

## Built-In Servers

T3Claw ships with a built-in registry of hosted MCP servers. A few examples:

| Server                                         | Transport | What it does                              |
|------------------------------------------------|-----------|-------------------------------------------|
| [Asana](https://mcp.asana.com/v2/mcp)          | HTTP      | Task management, projects, and team coordination  |
| [Cloudflare](https://mcp.cloudflare.com/mcp)   | HTTP      | DNS, Workers, KV, and infrastructure management   |
| [Intercom](https://mcp.intercom.com/mcp)       | HTTP      | Customer messaging, support, and engagement       |
| [Linear](https://mcp.linear.app/sse)           | HTTP      | Issue tracking and project management             |
| [NEAR AI](https://private.near.ai/mcp)         | HTTP      | Built-in tools like web search                    |
| [Notion](https://mcp.notion.com/mcp)           | HTTP      | Pages, databases, and comments                    |
| [Sentry](https://mcp.sentry.dev/mcp)           | HTTP      | Error tracking and performance monitoring         |
| [Stripe](https://mcp.stripe.com)               | HTTP      | Payments, subscriptions, and invoices             |

Browse more servers at:
- [MCP Server Registry](https://github.com/modelcontextprotocol/servers) (official)
- [awesome-mcp-servers](https://github.com/punkpeye/awesome-mcp-servers) (community)

---

## Troubleshooting

```bash
# Check server health
t3claw mcp test <server-name>

# Re-authenticate an OAuth server
t3claw mcp auth <server-name>

# Disable without removing
t3claw mcp toggle <server-name> --disable

# Debug logging
RUST_LOG=t3claw::tools::mcp=debug t3claw mcp test <server-name>
```
