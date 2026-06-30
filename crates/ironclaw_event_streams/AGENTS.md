# Agent Map - ironclaw_event_streams

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts before changing behavior:
  - `docs/reborn/contracts/events.md`
  - `docs/reborn/contracts/events-projections.md`
  - `crates/ironclaw_event_projections/AGENTS.md`
  - `crates/ironclaw_outbound/AGENTS.md`

## What This Crate Owns

- Transport-neutral projection stream management for Reborn product-facing reads.
- Access, admission, bounded subscription buffers, live-update forwarding, and explicit lag/rebase signals.
- Stream-boundary redaction validation for product-safe projection DTOs.
- Outbound push-candidate lookup through outbound policy, without performing transport sends.

## Do Not Move In Here

- Axum, SSE, WebSocket, Telegram, Slack, OpenAI/Responses, or channel framing.
- Durable event-store adapters or direct event-row reads.
- Product workflow turn submission, conversation binding, runtime dispatch, or host execution.
- Raw prompts, tool input/output, secrets, host paths, provider/runtime diagnostics, or backend details.

## Validation

- Fast local check: `cargo test -p ironclaw_event_streams --locked`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn --locked`
- Run `cargo clippy -p ironclaw_event_streams --all-targets -- -D warnings` before requesting review.

## Agent Notes

- Keep this crate transport-neutral; concrete transports should adapt stream items elsewhere.
- Authorize actor/scope/view/target before snapshot, replay, or live subscription work.
- Treat tenant/user scope as authority-bearing for admission and projection stream reads.
- Test through `EventStreamManager` when a helper gates access, admission, redaction, source subscription, or outbound lookup.
