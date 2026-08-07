# google-slides — Google Slides tools

The Google Slides extension: create presentations and slides, insert and
format text, shapes, and images, and batch-update decks. Extension id:
`google-slides`. This is a **data-only package**: no crate; the portable tool
half ships as a WASM guest.

- **Surfaces:** 14 tools (`google-slides.create_presentation` … `google-slides.batch_update`) + `[auth.google]`
- **Vendor (credential authority):** `google` — shared with gmail and the other `google-*` extensions
- **Runtime:** `wasm` — committed artifact in `wasm/`, guest source in `wasm-src/`
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`, `wasm/`, `wasm-src/`; embedded by `ironclaw_extension_support::packages::gsuite`
- **Tests / checks:** manifest projection — `cargo test -p ironclaw_extension_registry`;
  artifact freshness — `python3 scripts/ci/check-wasm-artifact-freshness.py`

Family model and the package rules: `crates/extensions/AGENTS.md`.
