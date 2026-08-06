# `crates/events/` — evidence, derived views, transport streams

**Layer(s):** `substrates` · **Crates:** 4 — `ironclaw_event_log`, `ironclaw_event_store`, `ironclaw_event_projections`, `ironclaw_event_streams` · **Security posture:** append-only, redacted-by-construction durable evidence feeds rebuildable, access-checked read models over a transport-neutral, admission-controlled stream; only `ironclaw_event_store` may write a durable log, and no crate in this family may decide authority.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/events/
├── ironclaw_event_log            evidence vocabulary & log traits
├── ironclaw_event_store          durable backends & fail-closed profiles
├── ironclaw_event_projections    replay-derived read models
└── ironclaw_event_streams        admission-checked stream delivery
```

## Role

`crates/events/` is the system's record of what happened, kept structurally distinct from what a screen shows right now. It is a one-way pipeline, four stages deep: a producer emits a redacted event or audit envelope; `ironclaw_event_log`'s traits accept it; `ironclaw_event_store` selects a durable backend and appends the entry under a monotonic cursor; `ironclaw_event_projections` folds the log into scoped, metadata-only read models on demand; `ironclaw_event_streams` authorizes and admits a live or replay subscription over those projections, consulting the outbound-delivery domain only to read push candidates. Nothing in this family runs backward: a projection cannot mutate the log it was folded from, and a stream cannot invent state that did not arrive by replaying one.

## Boundaries — what makes this family distinct

- **vs `substrates/`:** substrate crates (filesystem, libsql_runtime, secrets, network, safety) are timeless mechanism — containment, connection admission, encryption, egress hardening — with no notion of "what happened, in order." `events/` is the temporal evidentiary record built on top of that mechanism: `ironclaw_event_store` opens a durable backend through the storage fabric and routes every append through it. Substrate has no history; `events/` is the history, and its only substrate dependency is the storage fabric it appends into.
- **vs `domains/`:** domain crates own typed record grammar for a subject — a thread, a trigger, a memory document. `events/` owns the cross-cutting fact stream that domains and kernel crates emit into, not a subject of its own; it has no thread record and no trigger record, only the redacted shape of "a thing happened" or "an audit-relevant decision was made." Domains model what a thing is at this instant; `events/` models what happened, replayably, independent of which domain caused it.
- **vs `product/` (projections vs product views):** `ironclaw_event_projections` produces metadata-only, replay-derived, scope-checked read models with no write ports and no materialized store of their own. `ironclaw_assistant` assembles those projections into user-facing views and owns presentation and command semantics. A projection has no opinion about how a screen renders or what a user may do next; the dependency points only from product down into events, never the reverse.
- **This family's internal structure holds three distinct contracts — evidence, projection, stream — by design, and the separation is load-bearing.** `ironclaw_event_log` is pure vocabulary and traits with no storage driver, so every producer that only needs to record a fact stays free of any database or TLS dependency. `ironclaw_event_store` is a second, separate crate purely for driver-cone isolation and fail-closed backend-selection policy; it depends on `ironclaw_event_log`, never the reverse. `ironclaw_event_projections` is a third, separate crate because it must remain provably non-writing: it depends on nothing but the evidence vocabulary and the neutral authority vocabulary, never on a storage driver. `ironclaw_event_streams` is a fourth, separate crate because it alone is trusted to read the outbound-delivery domain's push candidates; no other crate in this family may hold that dependency.

## What belongs here / What never belongs here

**Belongs:** redacted event and audit vocabulary and append-log traits with no drivers; durable backend selection and fail-closed production-profile validation; replay-derived, scope- and cursor-bounded read models with no materialized store; transport-neutral subscription authorization, admission control, and redaction validation before anything crosses a wire.

**Never belongs here:** SSE, WebSocket, or webhook framing, or any concrete transport — that is the product family's job; product view assembly, command handling, or presentation logic; a second write authority — no projection or stream may write back into a durable log or invent state that did not come from replaying one; vendor names; raw secrets, raw host paths, raw tool input or output, approval reasons, invocation fingerprints, or lease material in any persisted or streamed shape.

## Dependency direction

`ironclaw_event_log` depends on the neutral authority vocabulary crate only — it is the leaf of the family. `ironclaw_event_store` depends on `ironclaw_event_log` and the storage-fabric crate. `ironclaw_event_projections` depends on `ironclaw_event_log` and the neutral authority vocabulary crate, and nothing else. `ironclaw_event_streams` depends on `ironclaw_event_projections`, the neutral authority vocabulary crate (including its turn vocabulary), and the outbound-delivery domain crate for exactly one read-only method. Nothing outside this family depends into it except through `ironclaw_event_log`'s and `ironclaw_event_projections`' public traits and DTOs — with one assembly-tier exception: the composition root consumes `ironclaw_event_store`'s backend-selection factory to construct the durable logs it hands everyone else. The family never depends on the kernel, loop, extension, or product families.

## Security & authority

This family holds the durable audit/event append and replay-cursor responsibility the kernel perimeter delegates outward, and it enforces one governing rule: projections are rebuildable and never authority. Evidence becomes safe to persist only after redaction at the source, in `ironclaw_event_log`'s constructors. It becomes safe to project only after scope-checking and metadata-only folding in `ironclaw_event_projections`, which holds no write port with which it could ever become a second source of truth. It becomes safe to deliver only after `ironclaw_event_streams` authorizes a subscription independently of whether the outbound-delivery domain has separately authorized a push — watch and push are always two decisions, never one, and a subscriber's read access to a projection never implies delivery eligibility.

## Crates

### `ironclaw_event_log`

- **Purpose:** the redacted event and audit vocabulary and the sink/log traits every producer in the system records through, with no storage driver of its own.
- **Owns:** the monotonic per-stream `EventCursor` and its replay vocabulary, including an explicit replay-gap error for a cursor older than the earliest retained entry; the redacted `RuntimeEvent`/`RuntimeEventKind` and `SecurityAuditEvent` shapes and their sanitizing constructors; the best-effort `EventSink`/`AuditSink` trait pair and the explicit-error `DurableEventLog`/`DurableAuditLog` trait pair; an in-memory reference implementation of each, for tests and reference loops.
- **Never contains:** storage drivers of any kind; projection replay or fold policy; transports; byte-offset or line-indexed replay helpers — replay is cursor-based only, so a durable backend can compact its storage without ever breaking a consumer's resume position.
- **Public surface:** `EventSink`/`AuditSink` (best-effort; a sink failure must never alter a runtime or control-plane outcome); `DurableEventLog`/`DurableAuditLog` (explicit-error, monotonic cursor, replay-gap on rebase).
- **Depends on:** the neutral authority vocabulary crate only.
- **Never depends on:** anything above the substrate tier; any storage-driver crate.
- **Security & authority role:** owns the redaction invariant at the point of construction. Every constructor collapses an unsafe error category into a bounded, safe classification and truncates free-form summaries, so nothing durable or replayable can ever carry a raw secret, host path, token, approval reason, fingerprint, or lease.
- **Why a separate crate:** it is the one neutral contract every producer in the system needs, and its independence from any storage driver is exactly what lets `ironclaw_event_store` exist as an isolated crate at all — if evidence vocabulary and durable-backend selection lived together, every producer would compile a database and TLS stack it never touches.

### `ironclaw_event_store`

- **Purpose:** durable backend selection and fail-closed production-profile policy for event and audit logs — the composition-facing side of the substrate, kept separate so storage drivers never leak upstream.
- **Owns:** the backend-selection configuration type and its fail-closed production-profile validation, so a production deployment can never silently fall back to a non-durable or ambiguous backend; the concrete durable-log adapters that implement `ironclaw_event_log`'s traits over the storage fabric, anchored at a dedicated events root; a coalescing sink for high-frequency producers.
- **Never contains:** projection logic, transport fanout, or workflow policy; any backend-specific error type in its public surface — errors are redacted and backend-generic regardless of which durable backend produced them.
- **Public surface:** the backend-selection entry point, which returns a paired durable event and audit log handle; `DurableEventLog`/`DurableAuditLog` implementations for each supported backend.
- **Depends on:** `ironclaw_event_log`; `ironclaw_common`; the storage-fabric crate; the neutral authority vocabulary crate.
- **Never depends on:** anything above the substrate tier; any projection or transport crate.
- **Security & authority role:** enforces fail-closed backend selection as policy, not convention. A production profile must explicitly accept single-node durability modes and must reject cleartext or ambiguous remote targets; there is no implicit fallback to an in-memory or non-durable backend once a deployment declares itself production.
- **Why a separate crate:** it is the only crate in the family permitted to carry a database or TLS driver cone. Isolating it here means every other producer and consumer in this family — and everything that in turn depends on them — never compiles that cone at all.

### `ironclaw_event_projections`

- **Purpose:** replay-derived, metadata-only read models with scope, cursor, and rebase semantics — never a materialized store, never authority.
- **Owns:** `EventProjectionService`/`AuditProjectionService` and their replay implementations; scoped request/cursor/snapshot/replay vocabulary for both the event and the audit log; read-model DTOs such as a thread timeline, a run-status projection, and a capability-activity projection; a bounded replay page size and a rebase ceiling past which a consumer must request a fresh snapshot rather than an incremental replay.
- **Never contains:** any durable log or any store of its own; a write path back into evidence, under any name; direct storage-driver dependencies of any kind; a second stream manager — subscription and admission belong exclusively to `ironclaw_event_streams`.
- **Public surface:** the projection service traits, scoped by tenant, actor, and read-scope; an explicit rebase-required error a consumer must handle by requesting a fresh snapshot rather than assuming lost entries were silently skipped.
- **Depends on:** `ironclaw_event_log`; the neutral authority vocabulary crate.
- **Never depends on:** any domain crate; any kernel crate; any storage-driver crate; the loop or product families.
- **Security & authority role:** the family's clearest authority-consumer boundary — a projection failure must be observable but never mutating, and the crate's dependency surface makes that a structural fact rather than a review discipline: it has nothing to write with.
- **Why a separate crate:** keeping projection folding fully isolated from stream subscription means a projection failure can never touch a live subscription, and keeping it fully isolated from storage drivers means "projections never write authority" is enforced by what the crate is permitted to link, not only by what its code happens to do.

### `ironclaw_event_streams`

- **Purpose:** the transport-neutral stream manager — authorization, admission control, bounded live/replay stitching, lag and rebase handling, and read-only outbound push-candidate lookup. It never sends.
- **Owns:** `EventStreamManager` and its injected collaborators — projection access policy, subscription admission policy, live-update source, redaction validator, and outbound-state lookup; a scoped, RAII admission permit for long-lived subscriptions, so an abandoned subscription always releases its slot; the actor/scope/view/target authorization check that must pass before any snapshot, replay, or live delivery is returned.
- **Never contains:** SSE, WebSocket, or any channel-specific framing; any send path; any durable store of its own.
- **Public surface:** the stream-manager construction entry point, generic over its injected collaborators; the admission-permit type.
- **Depends on:** `ironclaw_event_projections`; the neutral authority vocabulary crate, including its turn vocabulary; the outbound-delivery domain crate, for exactly one read-only push-candidate lookup method.
- **Never depends on:** any transport framework; any channel-specific crate; a write path into the outbound-delivery domain — the dependency is read-only by design.
- **Security & authority role:** enforces the family's strictest boundary. Every value crossing this crate toward a subscriber fails closed on raw prompts, tool input or output, secrets, host paths, provider errors, invocation fingerprints, approval reasons, lease material, or backend diagnostics. Subscription authorization and delivery authorization are kept as two independent decisions on principle.
- **Why a separate crate:** the three-contract split — durable evidence, derived projections, transport streams — is a standing design invariant, and this crate is its delivery leg: the only member of the family trusted to read outbound delivery state, so the other three crates can be reasoned about — and depended upon — without ever considering delivery semantics at all.

## Family AGENTS.md requirements

The family root's `AGENTS.md` states, as the governing law of the family: projections never write authority; streams never invent state; only the store isolates drivers. Every crate in the family carries guidance restating its own slice of that law ✎ **(amended 2026-08-05: this required "both an `AGENTS.md` and a `CLAUDE.md`"; superseded by `docs/reborn/guidance-conventions.md` — one canonical home per fact, any second file a pointer)** — what it owns, what it must never persist or expose, and which of its own dependencies is the one deliberate exception, if any, to the family's "no storage driver outside the store" rule. The family root additionally states the one rule no single crate's guide can state alone: this is a one-way pipeline — evidence, then store, then projection, then stream — and a dependency arrow pointing backward through that order is always wrong, regardless of what any individual crate's local rules might otherwise permit. It also states, as the family's admission list, what belongs here at all: redacted evidence vocabulary and log traits, durable backend selection and fail-closed profile validation, replay-derived read models, and admission-checked stream delivery — the four pipeline stages, and nothing else.
