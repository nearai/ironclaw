# web-access — web search and page fetch

The web-access extension: search the web and fetch page content. Extension id:
`web-access`. This is a **data-only package**: no crate and no WASM module;
its tools execute through the shared native web-access executor.

- **Surfaces:** 2 tools (`web-access.search`, `web-access.get_content`) — no `[auth.*]` recipe
- **Vendor (credential authority):** none
- **Runtime:** `first_party` — executor in `ironclaw_extension_support::web_access`
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`; embedded by `ironclaw_extension_support::packages::web_access`
- **Tests:** executor — `cargo test -p ironclaw_extension_support`; manifest projection — `cargo test -p ironclaw_extension_registry`

Family model and the package rules: `crates/extensions/AGENTS.md`.
