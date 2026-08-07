# notion-mcp — Notion hosted-MCP extension

The Notion extension: tools discovered from Notion's hosted MCP server rather
than declared statically. Extension id: `notion`. This is a **data-only
package**, and the worked example of the hosted-MCP shape: `[mcp]` (server,
namespace, max_tools, credentials) in place of `[runtime]` + `[[tools]]`; past
activation a discovered tool is an ordinary tool surface.

- **Surfaces:** `[mcp]` hosted server (no static `[[tools]]`) + `[auth.notion]`
- **Vendor (credential authority):** `notion`
- **Runtime:** MCP loader (discovery owned by `ironclaw_extension_host`)
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`; embedded by `ironclaw_extension_support::packages::notion`
- **Tests:** manifest projection — `cargo test -p ironclaw_extension_registry`; host-side discovery/registration — `cargo test -p ironclaw_extension_host`

Family model and the package rules: `crates/extensions/AGENTS.md`.
