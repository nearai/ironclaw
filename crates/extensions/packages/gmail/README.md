# gmail — Gmail tools

The Gmail extension: list, read, send, draft, reply, and trash mail as the
connected Google account. Extension id: `gmail`. This is a **data-only
package**: no crate and no WASM module; its tools execute through the shared
native gsuite executor.

- **Surfaces:** 6 tools (`gmail.list_messages` … `gmail.trash_message`) + `[auth.google]`
- **Vendor (credential authority):** `google` — shared with the five `google-*` extensions; recipes for one vendor must match apart from scopes, and scopes union across active extensions
- **Runtime:** `first_party` — executor in `ironclaw_extension_support::gsuite`
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`; embedded by `ironclaw_extension_support::packages::gmail`
- **Tests:** executor — `cargo test -p ironclaw_extension_support`; manifest projection — `cargo test -p ironclaw_extension_registry`

Family model and the package rules: `crates/extensions/AGENTS.md`.
