# Reborn Contract — Conversation Binding and Inbound Turns

**Status:** Implemented semantic slice  
**Date:** 2026-05-06  
**Depends on:** [`turn-persistence.md`](turn-persistence.md), [`turns-agent-loop.md`](turns-agent-loop.md), [`migration-compatibility.md`](migration-compatibility.md)

---

## 1. Purpose

Conversation binding is the adapter-safe ingress boundary between external product surfaces and `ironclaw_turns::TurnCoordinator`.

Adapters pass structured external actor/conversation refs to this boundary. The boundary returns canonical Reborn refs:

- tenant-scoped `TurnScope`;
- `TurnActor`;
- accepted inbound `AcceptedMessageRef`;
- `SourceBindingRef`;
- `ReplyTargetBindingRef`.

`TurnCoordinator` consumes only those canonical refs. It must not parse Slack, Telegram, Web, CLI, or other external conversation IDs, and it must not persist raw message content.

---

## 2. Ownership

| Component | Owns | Does not own |
| --- | --- | --- |
| `ConversationBindingService` | Pairing/authenticated actor resolution, external conversation binding lookup/creation, explicit conversation-to-thread links, source/reply target binding refs, reply-target validation | Raw transcript content, run lifecycle, product payload parsing |
| `SessionThreadService` | Accepted inbound message refs, external event idempotency, message-to-thread/source/reply refs | Durable transcript schema details owned by #3204, turn/run locks |
| `InboundTurnService` | Facade composition: resolve binding, accept message, submit canonical turn | Adapter protocol parsing, assistant egress fanout |
| `TurnCoordinator` | Turn/run admission and lifecycle after accepted message refs exist | External actor/conversation parsing, raw message storage |

---

## 3. Implemented semantic slice

`crates/ironclaw_conversations` provides the first contract slice:

- typed external refs: `AdapterKind`, `AdapterInstallationId`, `ExternalActorRef`, `ExternalConversationRef`, `ExternalEventId`;
- `ConversationBindingService`, `SessionThreadService`, and `InboundTurnService` traits/DTOs;
- `InMemoryConversationServices` for semantic contract tests and future adapter wiring spikes;
- caller-level tests proving the facade submits only canonical refs to `TurnCoordinator`.

This is not the final durable transcript store. PostgreSQL/libSQL storage and lazy v1 transcript migration remain downstream of #3204.

---

## 4. Required semantics

1. Missing authenticated bindings create one new canonical thread and one source/reply binding pair.
2. Unpaired actors fail closed with `BindingRequired`; no message is accepted and no turn is submitted.
3. Different adapter installations/conversations do not auto-merge even for the same paired user.
4. Explicit linking can attach a new external conversation to an existing thread only after actor/thread access checks pass.
5. Pairing/authenticated actor resolution is scoped by `(tenant_id, adapter_kind, adapter_installation_id, external_actor_ref)`; a pairing on one tenant or adapter installation does not authorize another.
6. External actor/conversation refs stay structured for equality. String fingerprints, when exposed for diagnostics, must be collision-safe for delimiter-like external IDs.
7. Explicit linking resolves the target thread inside the requested tenant; a caller cannot attach a different tenant's thread by reusing or guessing a thread id.
8. External inbound idempotency is keyed by `(tenant_id, source_binding_ref, external_event_id)` and replays the original accepted message ref without submitting a duplicate turn.
9. Bound group/channel messages are authorized against thread participants; external channel membership alone is insufficient.
10. Source binding and reply target binding refs are distinct. Egress must validate the stored reply target for the current actor/thread before sending.
11. Message content crosses this boundary as a content ref. Raw user text is owned by the transcript/content storage boundary, not turn state.

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
