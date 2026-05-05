# ironclaw_mcp guardrails

- Own the Reborn MCP runtime lane: MCP execution request/result types, client abstraction, host-mediated HTTP adapter, JSON-RPC exchange logic, and MCP-specific resource accounting.
- HTTP/SSE transports must go through host-mediated runtime egress. Do not add direct outbound networking, ad-hoc HTTP clients, DNS checks, credential injection, or network policy evaluation here.
- Treat plugin/runtime input as untrusted. Inputs may shape JSON-RPC arguments only; network policy, credentials, timeouts, and body limits must come from host-owned planning/handoff data.
- Preserve session isolation by scope/provider/url and keep session ids validated before reuse.
- Resource reservations supplied by host/runtime dispatch must be reconciled or released exactly once; do not create secondary reservations when a prepared reservation is present.
- Surface only stable, sanitized client/runtime error categories. Do not expose upstream URLs with secrets, raw credentials, response bodies, or transport internals in runtime-visible errors.
- Keep MCP protocol concerns here; extension discovery belongs in `ironclaw_extensions`, network enforcement in `ironclaw_network`/host runtime egress, and product workflow outside this crate.
