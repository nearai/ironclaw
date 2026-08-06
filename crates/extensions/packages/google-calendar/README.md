# google-calendar — Google Calendar tools

The Google Calendar extension: list calendars and events, find free slots,
create/update/delete events, manage attendees and reminders. Extension id:
`google-calendar`. This is a **data-only package**: no crate and no WASM
module; its tools execute through the shared native gsuite executor.

- **Surfaces:** 9 tools (`google-calendar.list_calendars` … `google-calendar.set_reminder`) + `[auth.google]`
- **Vendor (credential authority):** `google` — shared with gmail and the other `google-*` extensions
- **Runtime:** `first_party` — executor in `ironclaw_extension_support::gsuite`
- **Contents:** `manifest.toml`, `prompts/`, `schemas/`; embedded by `ironclaw_extension_support::packages::gsuite`
- **Tests:** executor — `cargo test -p ironclaw_extension_support`; manifest projection — `cargo test -p ironclaw_extension_registry`

Family model and the package rules: `crates/extensions/AGENTS.md`.
