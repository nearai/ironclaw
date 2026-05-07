# Agent Map — ironclaw_common

## Start Here

- No crate-local `CLAUDE.md` exists yet; use this map plus the repo rules below.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these sources of truth before changing shared types:
- `.claude/rules/types.md`
- `CLAUDE.md`

## What This Crate Owns

- Shared low-dependency workspace types and utilities: app events, identity newtypes, timezone validation, preview truncation, and cross-runtime constants.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- Runtime orchestration, persistence, network clients, web/TUI behavior, policy engines, or domain logic owned by more specific Reborn crates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_common`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If a type is serialized over API or persisted data, add compatibility tests for stable names and validation behavior.

## Agent Notes

- Keep this crate minimal; new dependencies here affect much of the workspace.
- Prefer validated newtypes and wire-stable enums over raw strings.
- If a shared type only serves one subsystem, keep it in that subsystem crate until a second real caller exists.
