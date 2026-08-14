# google-sheets — Google Sheets tools

The Google Sheets extension: create spreadsheets, read/write/append/clear
values, and manage and format sheets. Extension id: `google-sheets`. This is a
**data-only package**: no crate; the portable tool half ships as a WASM guest.

- **Surfaces:** 11 tools (`google-sheets.create_spreadsheet` … `google-sheets.format_cells`) + `[auth.google]`
- **Vendor (credential authority):** `google` — shared with gmail and the other `google-*` extensions
- **Runtime:** `wasm` — committed artifact in `wasm/`, guest source in `wasm-src/`
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`, `wasm/`, `wasm-src/`; embedded by `ironclaw_extension_support::packages::gsuite`
- **Tests / checks:** manifest projection — `cargo test -p ironclaw_extension_registry`;
  artifact freshness — `python3 scripts/ci/check-wasm-artifact-freshness.py`

Family model and the package rules: `crates/extensions/AGENTS.md`.
