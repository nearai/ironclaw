# github — GitHub tools

The GitHub extension: repos, issues, pull requests, reviews, files, releases,
workflows — the largest tool surface in the catalog. Extension id: `github`.
This is a **data-only package**: no crate; its behavior ships as a WASM guest.

- **Surfaces:** 49 tools (`github.get_repo` … `github.handle_webhook`) + `[auth.github]`
- **Vendor (credential authority):** `github`
- **Runtime:** `wasm` — committed artifact in `wasm/`, guest source in `wasm-src/` (excluded from the workspace build graph)
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`, `wasm/`, `wasm-src/`; embedded by `ironclaw_extension_support::packages::github`
- **Tests / checks:** manifest projection — `cargo test -p ironclaw_extension_registry`;
  artifact freshness — `python3 scripts/ci/check-wasm-artifact-freshness.py`
  (rebuild: `./scripts/build-wasm-extensions.sh --first-party`, then `--update`)

Family model and the package rules: `crates/extensions/AGENTS.md`.
