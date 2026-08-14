# google-drive — Google Drive tools

The Google Drive extension: list, fetch, upload, update, share, and organize
files and folders, including shared drives. Extension id: `google-drive`. This
is a **data-only package**: no crate; the portable tool half ships as a WASM
guest.

- **Surfaces:** 12 tools (`google-drive.list_files` … `google-drive.list_shared_drives`) + `[auth.google]`
- **Vendor (credential authority):** `google` — shared with gmail and the other `google-*` extensions
- **Runtime:** `wasm` — committed artifact in `wasm/`, guest source in `wasm-src/`
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`, `wasm/`, `wasm-src/`; embedded by `ironclaw_extension_support::packages::gsuite`
- **Tests / checks:** manifest projection — `cargo test -p ironclaw_extension_registry`;
  artifact freshness — `python3 scripts/ci/check-wasm-artifact-freshness.py`

Family model and the package rules: `crates/extensions/AGENTS.md`.
