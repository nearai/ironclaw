# nearai-mcp — NEAR AI hosted-MCP extension

The NEAR AI extension: web search and hosted agent capabilities, discovered
from the NEAR AI MCP server. Extension id: `nearai`. This is a **data-only
package**: a hosted-MCP extension declares `[mcp]` (server, namespace,
max_tools, credentials) in place of a `[runtime]` section, and past activation
a discovered tool is an ordinary tool surface.

- **Surfaces:** `[mcp]` hosted server + 1 statically pinned tool (`nearai.web_search` — model-visible from first boot, replaced by the live catalog after a successful `tools/list` discovery) + `[auth.nearai]` (api_key)
- **Vendor (credential authority):** `nearai` — reuses the assistant's host-managed `llm_nearai_api_key`; no separate account setup
- **Runtime:** MCP loader (discovery owned by `ironclaw_extension_host`)
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`. Embed module: `ironclaw_extension_support::packages::nearai` — deliberately **not** in the `PACKAGES` table, because the shipped `[mcp].server` is a placeholder the host rewrites from operator LLM-admin configuration; read that module's header before "fixing" the omission
- **Tests:** manifest projection — `cargo test -p ironclaw_extension_registry`; host-side discovery/registration — `cargo test -p ironclaw_extension_host`

Family model and the package rules: `crates/extensions/AGENTS.md`.
