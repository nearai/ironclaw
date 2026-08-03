# Agent Map — ironclaw_conversations

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and backend feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_turns/AGENTS.md`
  - `crates/ironclaw_threads/CLAUDE.md`
  - `crates/ironclaw_extension_contracts/CLAUDE.md` — it declares the external
    actor/conversation ref pair this crate binds on (added 2026-08-02; the dep
    arrived with the WS5 unification and this list did not).
  - `crates/ironclaw_product/CLAUDE.md`
  - `docs/reborn/contracts/events-projections.md`

## What This Crate Owns

- Adapter-safe conversation binding and inbound-turn service contracts.
- Source/reply binding refs, participant checks, message acceptance refs, and
  idempotency semantics — plus the **durable grammar** for external refs
  (`stored_refs`): write the released spelling
  `{space_id, conversation_id, thread_id, message_id}`, read either, so the
  WS5 rename is invisible to storage in both directions and a rollback stays
  safe.
  > Corrected 2026-08-02 (Wave 2 docs-truth audit): this read "External
  > actor/conversation refs, source/reply binding refs, …". The ref **types**
  > are no longer owned here — WS5 unified them onto
  > `ironclaw_extension_contracts::external` after finding the two copies were
  > field-divergent *and* compared differently (this crate's derived `PartialEq`
  > included the per-event message id; the canonical type excludes the
  > reply-target hint by hand), so two refs for one route could be equal or
  > unequal depending on which copy the caller held. Declaring them again here
  > fails `reborn_conversations_threads_attachments.rs`. What this crate owns is
  > the record grammar, above.
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
