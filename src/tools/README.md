# Tool System

## Adding a New Tool

### Built-in Tools (Rust)

1. Create `src/tools/builtin/my_tool.rs`
2. Implement the `Tool` trait
3. Add `mod my_tool;` and `pub use` in `src/tools/builtin/mod.rs`
4. Register in `ToolRegistry::register_builtin_tools()` in `registry.rs`
5. Add tests

### WASM Tools (Recommended)

WASM tools are the preferred way to add new capabilities. They run in a sandboxed environment with explicit capabilities.

1. Create a new crate in `tools-src/<name>/`
2. Implement the WIT interface (`wit/tool.wit`)
3. Create `<name>.capabilities.json` declaring required permissions
4. Build with `cargo build --target wasm32-wasip2 --release`
5. Install with `clawyer tool install path/to/tool.wasm`

See `tools-src/` for examples.

## Tool Architecture Principles

**CRITICAL: Keep tool-specific logic out of the main agent codebase.**

The main agent provides generic infrastructure; tools are self-contained units that declare their requirements through capabilities files.

### What Goes in Tools (capabilities.json)

- API endpoints the tool needs (HTTP allowlist)
- Credentials required (secret names, injection locations)
- Rate limits and timeouts
- Auth setup instructions (see below)
- Workspace paths the tool can read

### What Does NOT Go in Main Agent

- Service-specific auth flows (OAuth for Notion, Slack, etc.)
- Service-specific CLI commands (`auth notion`, `auth slack`)
- Service-specific configuration handling
- Hardcoded API URLs or token formats

### Tool Authentication

Tools declare their auth requirements in `<tool>.capabilities.json` under the `auth` section. Two methods are supported:

#### OAuth (Browser-based login)

For services that support OAuth, users just click through browser login:

```json
{
  "auth": {
    "secret_name": "notion_api_token",
    "display_name": "Notion",
    "oauth": {
      "authorization_url": "https://api.notion.com/v1/oauth/authorize",
      "token_url": "https://api.notion.com/v1/oauth/token",
      "client_id_env": "NOTION_OAUTH_CLIENT_ID",
      "client_secret_env": "NOTION_OAUTH_CLIENT_SECRET",
      "scopes": [],
      "use_pkce": false,
      "extra_params": { "owner": "user" }
    },
    "env_var": "NOTION_TOKEN"
  }
}
```

To enable OAuth for a tool:
1. Register a public OAuth app with the service (e.g., notion.so/my-integrations)
2. Configure redirect URIs: `http://localhost:9876/callback` through `http://localhost:9886/callback`
3. Set environment variables for client_id and client_secret

#### Manual Token Entry (Fallback)

For services without OAuth or when OAuth isn't configured:

```json
{
  "auth": {
    "secret_name": "openai_api_key",
    "display_name": "OpenAI",
    "instructions": "Get your API key from platform.openai.com/api-keys",
    "setup_url": "https://platform.openai.com/api-keys",
    "token_hint": "Starts with 'sk-'",
    "env_var": "OPENAI_API_KEY"
  }
}
```

#### Auth Flow Priority

When running `clawyer tool auth <tool>`:

1. Check `env_var` - if set in environment, use it directly
2. Check `oauth` - if configured, open browser for OAuth flow
3. Fall back to `instructions` + manual token entry

The agent reads auth config from the tool's capabilities file and provides the appropriate flow. No service-specific code in the main agent.

### WASM Tools vs MCP Servers: When to Use Which

Both are first-class in the extension system (`clawyer tool install` handles both), but they have different strengths.

**WASM Tools (cLawyer native)**

- Sandboxed: fuel metering, memory limits, no access except what's allowlisted
- Credentials injected by host runtime, tool code never sees the actual token
- Output scanned for secret leakage before returning to the LLM
- Auth (OAuth/manual) declared in `capabilities.json`, agent handles the flow
- Single binary, no process management, works offline
- Cost: must build yourself in Rust, no ecosystem, synchronous only

**MCP Servers (Model Context Protocol)**

- Growing ecosystem of pre-built servers (GitHub, Notion, Postgres, etc.)
- Any language (TypeScript/Python most common)
- Can do websockets, streaming, background polling
- Cost: external process with full system access (no sandbox), manages own credentials, cLawyer can't prevent leaks

**Decision guide:**

| Scenario | Use |
|----------|-----|
| Good MCP server already exists | **MCP** |
| Handles sensitive credentials (email send, banking) | **WASM** |
| Quick prototype or one-off integration | **MCP** |
| Core capability you'll maintain long-term | **WASM** |
| Needs background connections (websockets, polling) | **MCP** |
| Multiple tools share one OAuth token (e.g., Google suite) | **WASM** |

The LLM-facing interface is identical for both (tool name, schema, execute), so swapping between them is transparent to the agent.
