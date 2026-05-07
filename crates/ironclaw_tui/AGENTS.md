# Agent Map — ironclaw_tui

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these sources of truth before changing behavior:
- `crates/ironclaw_tui/CLAUDE.md`
- `docs/channels/local.md`
- `src/NETWORK_SECURITY.md`

## What This Crate Owns

- Ratatui terminal UI primitives: `TuiApp`, event/input loop, layout/theme rendering, widgets, approval overlays, model/thread pickers, and clipboard-gated UI behavior.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- Agent loop execution, channel pairing/auth, web gateway behavior, database access, tool dispatch, or host-specific approval policy.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_tui`
- UI/API check after public type changes: run the narrowest caller test that uses `src/channels/tui.rs` or the TUI channel bridge.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Keep the crate independent from the main `ironclaw` crate; bridge integration belongs in `src/channels/tui.rs`.
- Preserve keyboard shortcuts and approval affordances documented in `CLAUDE.md` unless the UX change is explicit.
- When adding widgets, update the widget registry and rendering path together.
