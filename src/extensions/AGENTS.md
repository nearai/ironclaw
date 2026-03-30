# Extensions — AGENTS.md

## OVERVIEW
Extension lifecycle manager: search, install, authenticate, activate, remove for WASM tools/channels and MCP servers.

## WHERE TO LOOK
- `manager.rs` (8,376 lines) — ExtensionManager central dispatcher; Telegram owner-binding, OAuth flows, WASM channel hot-activation
- `mod.rs` (1,109 lines) — ExtensionKind enum, all result/error types, AuthHint/AuthStatus state machine
- `registry.rs` (847 lines) — ExtensionRegistry with fuzzy search; builtin + discovery cache
- `discovery.rs` (507 lines) — OnlineDiscovery; probes mcp.{service}.com, GitHub MCP-server topic search

## KEY CONCEPTS

### ExtensionKind
- `McpServer` — hosted MCP server, HTTP transport, OAuth 2.1 auth
- `WasmTool` — sandboxed WASM module, capabilities-based auth
- `WasmChannel` — WASM channel with hot-activation (Telegram, Slack, Discord)
- `ChannelRelay` — external channel via relay service

### Lifecycle
`search → install → authenticate/configure → activate → remove`

### AuthHint Variants
- `Dcr` — Dynamic Client Registration (zero-config OAuth)
- `OAuthPreConfigured` — needs pre-configured client_id/secret
- `CapabilitiesAuth` — WASM tool auth from capabilities.json
- `ChannelRelayOAuth` — OAuth via channel-relay service

## ANTI-PATTERNS
- NEVER assume McpServer == HTTP; WASM channels use tunnel/webhook config instead
- NEVER skip Telegram owner-binding verification for bot channels
- NEVER hardcode OAuth callback paths; use normalize_oauth_callback_path()
