# google-docs — Google Docs tools

The Google Docs extension: create and read documents, insert/replace/format
text and structure. Extension id: `google-docs`. This is a **data-only
package**: no crate; the portable tool half ships as a WASM guest.

- **Surfaces:** 15 tools, including semantic inspection/edit/table/verification operations and the existing low-level operations, + `[auth.google]`
- **Vendor (credential authority):** `google` — shared with gmail and the other `google-*` extensions
- **Runtime:** `wasm` — committed artifact in `wasm/`, guest source in `wasm-src/`
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`, `wasm/`, `wasm-src/`; embedded by `ironclaw_extension_support::packages::gsuite`
- **Tests / checks:** manifest projection — `cargo test -p ironclaw_extension_registry`;
  artifact freshness — `python3 scripts/ci/check-wasm-artifact-freshness.py`

For structured editing, prefer `inspect_document`, one or more
`apply_text_edits` / `create_table_with_data` calls, and `verify_document`.
That keeps a typical document workflow to 3–4 model-visible capability calls;
the extension handles index discovery, batched cell writes, concurrency checks,
and provider read-back internally. The original 11 low-level operations remain
available for compatibility and unsupported edge cases.

Family model and the package rules: `crates/extensions/AGENTS.md`.
