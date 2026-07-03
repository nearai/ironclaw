# Agent Map — ironclaw_slack_v2_adapter

## Start Here

- No crate-local CLAUDE.md exists yet; use this map plus `Cargo.toml` and source files.
- Read `src/lib.rs` first, then:
  - `adapter.rs` — ProductAdapter implementation.
  - `payload.rs` — Slack Events API payload parsing/DTO handling.
  - `render.rs` — Slack outbound rendering.
- Read upstream contracts before changing adapter behavior:
  - `crates/ironclaw_product_adapters/AGENTS.md`
  - `crates/ironclaw_wasm_product_adapters/CLAUDE.md`

## What This Crate Owns

- Slack v2 ProductAdapter tracer-bullet implementation for Reborn issue #3857.
- Slack Events API payload parsing and outbound `chat.postMessage` rendering.
- Adapter-specific mapping between Slack shapes and shared ProductAdapter DTOs.

## Do Not Move In Here

- Legacy v1 `Channel` lifecycle, channel relay state, or Slack pairing flow.
- Host auth verification, canonical conversation/thread binding, or turn coordination.
- Network clients, raw Slack bot tokens/signing secrets, direct DB/filesystem access, or approval-run state.

## Validation

- Fast local check: `cargo test -p ironclaw_slack_v2_adapter`
- Run `cargo test -p ironclaw_product_adapters` when shared DTO assumptions change.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Keep Slack-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve adapter outputs as untrusted parsed DTOs until host/workflow stamps trusted context.
- Approval/auth conversational handling is deferred to the owning Reborn service seam (#3094).
