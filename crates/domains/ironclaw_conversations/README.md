# ironclaw_conversations

The adapter-safe boundary between product/channel adapters and turn
submission: it resolves external actor/conversation identifiers into canonical
tenant/thread/binding references, enforces inbound idempotency, and owns the
one-method port through which inbound orchestration reaches the turn
coordinator. **It is not the transcript** — `ironclaw_threads` owns canonical
threads and message content; this crate owns the *binding* from an external
conversation to one of them.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_conversations` · **Manifest:** `crates/domains/ironclaw_conversations/Cargo.toml`
- **Use this when:** binding/pairing an external conversation or actor to a
  canonical thread, accepting an inbound message exactly once, replaying a
  duplicate delivery, or classifying trusted-trigger submission failures.
- **Don't use this when:** you need message content or thread history →
  `ironclaw_threads`; you are parsing a channel payload → the channel's
  extension package; you need to *hold* a `TurnCoordinator` → you don't, from
  here — implement or reuse the `ConversationTurnSubmitter` adapter in
  `ironclaw_composition::automation::conversation_turn_submitter`.

## Public surface

- `InboundConversationService`, `ConversationBindingService`,
  `ConversationActorPairingService` — binding resolution, pairing, acceptance.
- `ConversationStateStore` + `RebornFilesystemConversationServices` (durable,
  over `ScopedFilesystem`) and `InMemoryConversationServices` (a real service
  used in tests, not a stub).
- `ConversationTurnSubmitter` (`src/turn_submission.rs`) — the one-method
  submission port, with `ConversationTurnSubmission`,
  `ConversationInboundClassification`, and the `TurnSubmissionError` cone
  (`retry()` / `category()` / `adapter_status_code()`). Declared here,
  implemented by composition (WS5 port inversion, 2026-08-04).
- `InboundTurnService` and `trusted_trigger_fire_submitter` — inbound
  orchestration plus the conversations-side trusted-trigger submitter.
- The durable grammar in `stored_refs`: writes the released on-disk spelling
  `{space_id, conversation_id, thread_id, message_id}` and reads either
  spelling, so the WS5 ref rename stays invisible to storage.
- Deliberately **not** exported: `ExternalActorRef` / `ExternalConversationRef`
  — their one home is `ironclaw_extension_contracts::external`.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_extension_contracts`,
  `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_triggers` (consumes —
  never mints — the trusted-submission binding). `ironclaw_turns` is a
  **dev-dependency only**, documented in the manifest, so test fakes can stand
  in for the composition adapter on the real `SubmitTurnRequest` shape.
- **Consumed by (3):** `ironclaw_assistant`, `ironclaw_composition`,
  `ironclaw_extension_host`.

## Invariants

- No normal `ironclaw_turns` dependency and no coordinator handle — the
  layer-matrix exception this crate closed; pinned by
  `reborn_crate_dependency_boundaries_hold` and the layer matrix.
- No `ironclaw_safety` dependency: the trusted-trigger prompt scan runs at the
  mint in `ironclaw_triggers`, not in any submitter impl — the absence is
  pinned by this crate's `BoundaryRule` (`reborn_dependency_boundaries.rs`).
- No `ironclaw_threads` dependency (transcript content never lands here) —
  same `BoundaryRule`.
- Shares no declared type name with `ironclaw_threads`; the external ref pair
  is declared only by `ironclaw_extension_contracts` — both pinned in
  `reborn_conversations_threads_attachments.rs`.
- Trusted-trigger submitter ownership stays conversations/composition —
  `conversation_trusted_trigger_submitter_stays_conversation_or_composition_owned`
  and `untrusted_ingress_paths_cannot_submit_host_trusted_inbound`.
- Binding resolution fails closed; idempotency and route-drift rules are in
  [`AGENTS.md`](./AGENTS.md), the canonical working-rules file.

## Tests

```bash
cargo test -p ironclaw_conversations
cargo test -p ironclaw_conversations --test conversation_state_store_contract
cargo test -p ironclaw_conversations --test inbound_contract
```

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: `families/domains.md` and PROPOSAL §6.4.2 (the port-inversion
  history lives there).
