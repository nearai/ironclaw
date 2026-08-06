# Agent Map — ironclaw_conversations

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and backend feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/kernel/ironclaw_turns/AGENTS.md` — background only; since 2026-08-04 this
    crate has **no normal dependency** on it. The seam is the port in
    `src/turn_submission.rs`, implemented by
    `crates/app/ironclaw_composition/src/automation/conversation_turn_submitter.rs`.
  - `crates/domains/ironclaw_threads/CLAUDE.md`
  - `crates/contracts/ironclaw_extension_contracts/CLAUDE.md` — it declares the external
    actor/conversation ref pair this crate binds on (added 2026-08-02; the dep
    arrived with the WS5 unification and this list did not).
  - `crates/product/ironclaw_assistant/CLAUDE.md`
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
- Canonical turn-submission inputs: `TurnScope`, `TurnActor`, `AcceptedMessageRef`, `SourceBindingRef`, and `ReplyTargetBindingRef` — all `ironclaw_host_api::turn`'s.
- The **turn-submission port** (`src/turn_submission.rs`): the one-method
  `ConversationTurnSubmitter`, its `ConversationTurnSubmission` request, the
  `ConversationInboundClassification` trust value, and the
  `TurnSubmissionError` cone. Declared here, implemented by
  `ironclaw_composition` (WS5 port inversion, 2026-08-04).

## Do Not Move In Here

- Concrete Slack/Telegram/Web/CLI payload parsing; product adapters normalize protocol payloads first.
- Raw user/assistant message content in turn-facing records; transcript content belongs to thread/transcript storage.
- Capability runtime internals, runtime dispatch, model/provider behavior, or UI transport.
- Silent retargeting of explicit links or route drift during adapter retries.
- Trusted-trigger prompt safety scanning, and with it any `ironclaw_safety`
  dependency. The scan is `ironclaw_triggers`' — it runs at the mint of the
  sealed `TrustedTriggerSubmitRequest`, so it covers every
  `TrustedTriggerFireSubmitter` rather than only the one wired here (moved
  2026-08-04, PROPOSAL §6.4.2; the absent dependency is pinned by
  `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`).

## Validation

- Fast local check: `cargo test -p ironclaw_conversations`
- Backend parity check when durable adapters change: run crate tests with all relevant features and DB harness settings.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`

## Agent Notes

- Binding resolution must fail closed for unknown threads, invalid refs, tenant/installation mismatches, participant-policy denial, or delimiter-like external IDs.
- Source binding refs and reply target binding refs are distinct; egress must revalidate current reply targets.
- Preserve the typed `TurnSubmissionError` the `ConversationTurnSubmitter` port returns; do not flatten turn failures to strings. Branch on `retry()`, project `category()`/`adapter_status_code()`, never parse the message. *(Amended 2026-08-04, WS5 port inversion — this invariant used to name `ironclaw_turns::TurnError`, which this crate no longer depends on. The `TurnError` → port-error mapping lives in the host adapter, `ironclaw_composition::automation::conversation_turn_submitter`, and preserves the class partition plus both accessors.)*
- Do not reintroduce a `TurnCoordinator` handle or an `ironclaw_turns` normal
  dependency here; that is the layer-matrix exception this crate closed. The
  port is declared in `src/turn_submission.rs` and implemented by composition.
  `ironclaw_turns` remains a **dev**-dependency only, so the test fakes can
  stand in for that adapter on the real `SubmitTurnRequest` shape.
