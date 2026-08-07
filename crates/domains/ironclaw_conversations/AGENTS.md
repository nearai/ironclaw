# ironclaw_conversations — working rules

Orientation (what this crate is, surface, deps, tests) lives in
[`README.md`](./README.md); the family boundary in
[`../AGENTS.md`](../AGENTS.md). This file is the canonical crate-local rules —
consolidated 2026-08-05 from the former `CLAUDE.md` guardrails (now a pointer)
per `docs/reborn/guidance-conventions.md` rule 1. One correction was made in
the consolidation: the transcript-boundary rule below used to name
`InboundConversationService` as the transcript storage boundary, which
inverted the WS5 rename — the transcript service is `ironclaw_threads`'
`SessionThreadService`; `InboundConversationService` is *this* crate's
binding/acceptance service.

## Ownership boundary

- Own adapter-safe conversation binding and inbound-turn service contracts
  only: source/reply binding refs, participant checks, message acceptance
  refs, idempotency semantics, and the durable grammar for external refs
  (`stored_refs`).
- Do not parse concrete Slack/Telegram/Web/CLI payloads here. Product/channel
  adapters normalize protocol payloads before calling these services.
- Do not persist raw user or assistant message content in turn-facing
  records. Use content/message refs; durable transcript content belongs to
  `ironclaw_threads` (`SessionThreadService` / the transcript storage
  boundary), never to this crate's stores.
- The external ref pair (`ExternalActorRef` / `ExternalConversationRef`) is
  declared only in `ironclaw_extension_contracts::external` — declaring either
  here again fails `reborn_conversations_threads_attachments.rs`, and so does
  any new type name shared with `ironclaw_threads`.
- Keep turn-submission inputs canonical: `TurnScope`, `TurnActor`,
  `AcceptedMessageRef`, `SourceBindingRef`, `ReplyTargetBindingRef` — imported
  from `ironclaw_host_api::turn`, never through a turn-kernel re-export.

## Binding and route identity

- Binding resolution must fail closed for unpaired actors,
  unknown/inaccessible threads, invalid refs, participant-policy denials,
  tenant/adapter-installation mismatches, and delimiter-like external IDs that
  could collide if flattened into strings.
- Conversation binding identity excludes per-message external IDs; bind on
  stable `(space_id, conversation_id, topic_id)` route identity so adapters
  that include message IDs do not fork canonical threads. `topic_id` is the
  *channel's* sub-conversation (Slack thread, Telegram topic) and is never an
  `ironclaw_host_api::ids::ThreadId` — that collision is the naming trap WS5
  removed. `thread_id` in this crate means the canonical thread, with one
  deliberate exception: the durable record grammar still spells the route's
  topic `thread_id` so a rollback can read it (see `stored_refs`, which
  writes the released spelling and reads either).
- Source binding refs and reply target binding refs are distinct. Egress
  paths must validate reply targets against the current external actor/thread
  before sending, preserving adapter kind, adapter installation, and full
  external route fields. Reply routes are owner-scoped to the exact external
  actor pairing key unless the adapter explicitly marks the route
  shared/group; shared markers may widen existing routes only from direct to
  shared.
- Accepted inbound messages must snapshot message-scoped reply targets and
  route access policy; stable conversation bindings ignore and strip
  per-message IDs, but egress routing for accepted messages must preserve
  them. New ingress writes use the canonical binding-level reply ref, not an
  older message-scoped reply snapshot.
- Accepted inbound message writes must reject mixed source/reply binding refs
  that do not belong to the same tenant/thread binding, and reject external
  route snapshots whose stable identity differs from the binding.
- Serde deserialization for external ref types must delegate to the same
  validation rules as constructors.
- Future durable binding repositories must avoid raw wide composite unique
  indexes for external route fields; use typed rows plus a collision-resistant
  digest/indirection key derived from length-prefixed components.
- Automatic first-contact binding must not trust raw adapter-supplied
  agent/project scope hints; only host-owned trusted scope passed through
  `resolve_or_create_binding_with_trusted_scope` may be persisted on automatic
  bind.
- Lookup-only binding resolution must not create threads, bindings,
  route-access widening, external-event route reservations, or
  accepted-message state.
- Explicit links are idempotent only for the same target thread; never
  silently retarget an already-bound external conversation to another thread.

## Idempotency and turn submission

- Accepted-message idempotency and turn-submission idempotency are separate:
  adapter retries must reuse the accepted message ref, canonical actor,
  original received timestamp, and original run-profile request until the
  message is marked submitted, even if live pairing state changes after
  acceptance; duplicate deliveries after submission replay the stored
  submission outcome. Reserve installation-wide external event IDs during
  resolution and reject route drift before creating a second thread. For
  transient turn-submission failures, rotate the submit idempotency key on
  each retry so turn-store replay cannot strand the message. Thread-busy
  admission of a user message is a terminal `RejectedBusy` (permanent) —
  adapters do NOT retry-until-submitted; a duplicate delivery after
  `RejectedBusy` replays the terminal outcome without resubmitting, keeping
  the original submit idempotency key for all permanent rejections.
- Preserve the typed `TurnSubmissionError` the `ConversationTurnSubmitter`
  port returns instead of flattening turn failures to strings. Branch on its
  `retry()` class (`RetryableAfterKeyRotation` / `RetryableWithSameKey` /
  `Permanent`) and project its `category()` / `adapter_status_code()`; never
  re-derive either from the rendered message. *(Amended 2026-08-04, WS5 port
  inversion: this crate no longer depends on `ironclaw_turns`, so the
  invariant that named `ironclaw_turns::TurnError` now names the port error.
  The host adapter —
  `ironclaw_composition::automation::conversation_turn_submitter` — owns the
  total `TurnError` → port-error mapping and carries the coordinator's
  rendered cause verbatim.)*
- Do not take a `TurnCoordinator` or any other turn-kernel handle in this
  crate. The inbound orchestration reaches turn submission through the
  one-method `ConversationTurnSubmitter` port declared in
  `src/turn_submission.rs`; composition implements it. Adding an
  `ironclaw_turns` normal dependency re-opens the layer-matrix exception this
  crate closed — it remains a **dev**-dependency only, so test fakes can
  stand in for the adapter on the real `SubmitTurnRequest` shape.
- Do **not** scan trusted-trigger prompts here, and do not re-add
  `ironclaw_safety` to this crate's dependencies. Until 2026-08-04 the
  conversations-owned `TrustedTriggerFireSubmitter` re-ran the injection scan
  itself; because it lived in one *implementation* of the seam, swapping or
  adding a submitter silently dropped the guard. PROPOSAL §6.4.2 moved the
  scan behind the seam: `ironclaw_triggers` runs it while minting the sealed
  `TrustedTriggerSubmitRequest`, so a request that reaches this crate has
  already passed. The missing dependency is pinned by this crate's
  `BoundaryRule` in `reborn_dependency_boundaries.rs`.

## Durable adapters

- Durable persistence in this crate is limited to conversation binding,
  accepted-message idempotency, and turn-submission state-store records over
  `ScopedFilesystem`; transcript content and thread storage stay behind their
  owning storage boundaries.
- Any future durable adapter expansion must name the scoped storage boundary
  first, keep raw message content out of turn-facing rows, and add backend
  parity tests with migration coverage.

## Validation

- Fast local check: `cargo test -p ironclaw_conversations`
- Contract suites: `conversation_state_store_contract`, `inbound_contract`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`

## Neighbors to read before changing behavior

- `crates/domains/ironclaw_threads/AGENTS.md` — the transcript owner.
- `crates/contracts/ironclaw_extension_contracts/CLAUDE.md` — declares the
  external actor/conversation ref pair this crate binds on.
- `crates/app/ironclaw_composition/src/automation/conversation_turn_submitter.rs`
  — the production implementation of the submission port.
- `crates/kernel/ironclaw_turns/AGENTS.md` — background only; no normal
  dependency exists.
- `docs/reborn/contracts/events-projections.md`
