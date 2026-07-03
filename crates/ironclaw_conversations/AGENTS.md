# Agent Map — ironclaw_conversations

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and backend feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_turns/AGENTS.md`
  - `crates/ironclaw_threads/CLAUDE.md`
  - `crates/ironclaw_product_adapters/CLAUDE.md`
  - `docs/reborn/contracts/events-projections.md`

## What This Crate Owns

- Adapter-safe conversation binding and inbound-turn facade contracts.
- External actor/conversation refs, source/reply binding refs, participant checks, message acceptance refs, and idempotency semantics.
- Binding/state-store persistence for conversation binding, accepted-message idempotency, and turn-submission state.
- Canonical `TurnCoordinator` inputs: `TurnScope`, `TurnActor`, `AcceptedMessageRef`, `SourceBindingRef`, and `ReplyTargetBindingRef`.

## Do Not Move In Here

- Concrete Slack/Telegram/Web/CLI payload parsing; product adapters normalize protocol payloads first.
- Raw user/assistant message content in turn-facing records; transcript content belongs to thread/transcript storage.
- Capability runtime internals, runtime dispatch, model/provider behavior, or UI transport.
- Silent retargeting of explicit links or route drift during adapter retries.

## Validation

- Fast local check: `cargo test -p ironclaw_conversations`
- Backend parity check when durable adapters change: run crate tests with all relevant features and DB harness settings.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Binding resolution must fail closed for unknown threads, invalid refs, tenant/installation mismatches, participant-policy denial, or delimiter-like external IDs.
- Source binding refs and reply target binding refs are distinct; egress must revalidate current reply targets.
- Preserve typed `ironclaw_turns::TurnError`; do not flatten turn failures to strings.
