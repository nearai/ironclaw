# Reborn Contract — Conversation Binding and Inbound Turns

**Status:** Implemented semantic slice  
**Date:** 2026-05-06  
**Depends on:** [`turn-persistence.md`](turn-persistence.md), [`turns-agent-loop.md`](turns-agent-loop.md), [`migration-compatibility.md`](migration-compatibility.md)

---

## 1. Purpose

Conversation binding is the adapter-safe ingress boundary between external product surfaces and `ironclaw_turns::TurnCoordinator`.
It also defines the planned host-trusted ingress seam used by scheduler and
trigger fires.

Adapters pass structured external actor/conversation refs to this boundary. The boundary returns canonical Reborn refs:

- tenant-scoped `TurnScope`;
- `TurnActor`;
- accepted inbound `AcceptedMessageRef`;
- `SourceBindingRef`;
- `ReplyTargetBindingRef`.

`TurnCoordinator` consumes only those canonical refs. It must not parse Slack, Telegram, Web, CLI, or other external conversation IDs, and it must not persist raw message content.

Reply-target binding is separate from external ingress identity. This contract binds accepted inbound messages to canonical threads and reply targets; it does not define any additional trusted-ingress authority beyond ordinary inbound binding.

---

## 2. Ownership

| Component | Owns | Does not own |
| --- | --- | --- |
| `ConversationBindingService` | Pairing/authenticated actor resolution, external conversation binding lookup/creation keyed by stable conversation identity, explicit conversation-to-thread links, source/reply target binding refs, reply-target validation with adapter installation and external routing data | Raw transcript content, run lifecycle, product payload parsing |
| `SessionThreadService` | Accepted inbound message refs, external event idempotency, message-to-thread/source/reply refs | Durable transcript schema details owned by #3204, turn/run locks |
| `InboundTurnService` | Facade composition: resolve binding, accept message, submit canonical turn; current untrusted ingress entry point and planned host-trusted ingress entry point | Adapter protocol parsing, assistant egress fanout |
| `TurnCoordinator` | Turn/run admission and lifecycle after accepted message refs exist | External actor/conversation parsing, raw message storage |

---

## 3. Implemented semantic slice

`crates/ironclaw_conversations` provides the first contract slice:

- typed external refs: `AdapterKind`, `AdapterInstallationId`, `ExternalActorRef`, `ExternalConversationRef`, `ExternalEventId`;
- `ConversationBindingService`, `SessionThreadService`, and `InboundTurnService` traits/DTOs;
- `InMemoryConversationServices` for semantic contract tests and future adapter wiring spikes;
- optional `RebornLibSqlConversationServices` and `RebornPostgresConversationServices` durable wrappers backed by normalized PostgreSQL/libSQL tables for pairings, threads/participants, bindings, reply targets, event routes, accepted messages, replay records, submit idempotency keys, and submit responses;
- caller-level tests proving the facade submits only canonical refs to `TurnCoordinator`.

This is not the final durable transcript store. The conversation contract stores accepted-message refs and content refs; durable raw transcript content and lazy v1 transcript migration remain downstream of the transcript/thread storage boundary (#3204).

---

## 4. Required semantics

1. Missing authenticated bindings create one new canonical thread and one source/reply binding pair.
2. Unpaired actors fail closed with `BindingRequired`; no message is accepted and no turn is submitted.
3. Different adapter installations/conversations do not auto-merge even for the same paired user.
4. Explicit linking can attach a new external conversation to an existing thread only after actor/thread access checks pass.
5. First-contact binding creation does not trust raw adapter-supplied agent/project scope hints. Scope selection must happen through a trusted thread-creation/authorization seam or by explicit linking to an existing accessible thread.
6. Pairing/authenticated actor resolution is scoped by `(tenant_id, adapter_kind, adapter_installation_id, external_actor_ref)`; a pairing on one tenant or adapter installation does not authorize another.
7. External actor/conversation refs stay structured for equality. String fingerprints, when exposed for diagnostics, must be collision-safe for delimiter-like external IDs.
8. Conversation binding identity uses stable conversation fields `(space_id, conversation_id, thread_id)`; per-message external IDs do not fork bindings or canonical threads.
9. Explicit linking resolves the target thread inside the requested tenant; a caller cannot attach a different tenant's thread by reusing or guessing a thread id.
10. Explicit linking is idempotent for the same target thread and fails closed rather than silently retargeting an already-bound external conversation to a different thread, including when only per-message external IDs differ.
11. External inbound idempotency is keyed by `(tenant_id, source_binding_ref, external_event_id)` and replays the original accepted message ref and canonical actor without inserting a duplicate message only after route/ref validation passes; duplicate retries with mismatched stable routes fail closed. Implementations must reserve installation-wide external event IDs at resolution time and reject replays on a different stable conversation route before creating a second canonical binding/thread.
12. Adapter retries after a transient turn-submission failure must retry `TurnCoordinator.submit_turn(...)` with the same accepted message ref, actor, original received timestamp, and original run-profile request until the accepted message is marked submitted, even if live pairing state changes after the original acceptance. Retry attempts must not reuse a submit idempotency key that the turn store can permanently replay as a transient failure; retries after a successful submission replay the original `SubmitTurnResponse` and do not submit a duplicate turn. Thread-busy admission for a user message is NOT a transient failure: it yields a durable `RejectedBusy` terminal outcome (settled; the user resends a new message rather than the adapter retrying the same accepted message). A duplicate delivery of an external event whose prior admission settled as `RejectedBusy` replays that terminal outcome using the original submit idempotency key and does not resubmit the turn.
13. Bound group/channel messages are authorized against thread participants when the adapter explicitly marks the external conversation route as shared; external channel membership alone is insufficient.
14. Source binding and reply target binding refs are distinct. Egress must validate the stored reply target for the current actor/thread before sending, and validation returns the adapter kind, adapter installation id, and full external conversation route needed to address the reply. Reply routes are owner-scoped by default to the exact external actor pairing key, not just `UserId`; explicit shared/group markers may monotonically widen ordinary reply routes to thread participants. Authority-bearing outbound payloads such as approval prompts and auth prompts must not use shared/group widening and require exact-owner validation.
15. Accepted inbound messages mint message-scoped reply target refs that snapshot the exact external route and route access policy for that inbound message. Stable binding-level reply target refs strip per-message IDs; reply routing for message-scoped refs must preserve them. Ingress writes must use the canonical binding-level reply ref, not an older message-scoped reply snapshot.
16. Accepted inbound message writes must validate that the supplied source binding ref and reply target binding ref belong to the same tenant/thread binding, and that caller-supplied external routes match the stable binding identity. Only per-message external IDs may vary; loose caller-supplied ref/route tuples are rejected fail-closed.
17. Conversation ingress must preserve typed `ironclaw_turns::TurnError` failures rather than flattening them to strings, so adapters can keep status/category/retry semantics without parsing display text.
18. Public serialized external refs must enforce the same invariants as constructors. Deserialization cannot bypass empty/control-character/oversized ref validation.
19. Public external route components may be up to 512 bytes for adapter compatibility. Durable PostgreSQL/libSQL implementations must not rely on a raw wide composite btree key for `(tenant, adapter_kind, installation, space, conversation, thread)` uniqueness; use typed rows plus a collision-resistant digest/indirection key derived from length-prefixed components.
20. Message content crosses this boundary as a content ref. Raw user text is owned by the transcript/content storage boundary, not turn state.
21. Host-trusted ingress is a host-only boundary for schedulers and trigger fires. The conversation-owned trusted trigger submitter implements `TrustedTriggerFireSubmitter` and must perform the same replay lookup as `handle_inbound_turn()` first, before any new binding creation or trusted-scope application, so duplicate scheduled-slot retries hit the existing inbound idempotency record instead of minting a second turn. The raw `TrustedInboundTurnRequest` constructor and concrete submitter type stay private inside `ironclaw_conversations`; no public DTO or facade exposes `trusted_agent_id` or `trusted_project_id`. Composition computes the trigger-to-conversation binding identity once while materializing the prompt, and the sealed trigger request carries that canonical binding into conversations for private trusted request construction. Adapter-supplied `requested_agent_id` / `requested_project_id` hints are not present on the host-trusted trigger path and are discarded before binding resolution.
22. Host-trusted trigger fires are submitted only through the conversation-owned submitter trait object returned by `trusted_trigger_fire_submitter(...)` and wired by host-owned composition services from durable host state. Product adapters cannot build this submitter or submit trusted trigger requests under workspace architecture rules. Trusted trigger ingress details live in [`triggers.md`](triggers.md); this contract owns only the replay-first trusted-scope adapter. No delivery target data belongs in `ExternalConversationRef`, `TurnActor`, `adapter_kind`, or any trusted ingress identity.

---

## 5. Verification

Current semantic coverage lives in:

```text
crates/ironclaw_conversations/tests/inbound_contract.rs
```

Run:

```bash
cargo test -p ironclaw_conversations --test inbound_contract
```

The planned trusted ingress implementation must add a caller-level regression
proving that a duplicate trusted trigger fire performs replay before binding
creation and reuses the original accepted message and turn submission.
