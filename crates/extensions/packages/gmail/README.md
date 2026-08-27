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

`gmail.get_message` returns producer-owned semantic JSON: selected message
headers, a decoded text or Markdown body, and bounded attachment metadata.
Provider MIME/base64 envelopes and unselected routing/authentication headers do
not enter the durable tool result. Encrypted messages are reported explicitly
without exposing ciphertext as readable content.

For Gmail `format=full`, the API has already applied MIME transfer encoding;
the producer decodes only Gmail's `body.data` base64url wrapper. This remains a
Gmail-local transformation because the generic durable-result path must not
guess provider semantics. Promote a shared conversion only when a second
producer has the same semantic contract; unsupported MIME bodies remain
explicitly unavailable rather than triggering a generic fallback.

Family model and the package rules: `crates/extensions/AGENTS.md`.
