# Trigger Loop — Design

**Date:** 2026-05-21
**Status:** Design approved, revised after spec review
**Target architecture:** IronClaw Reborn (`crates/ironclaw_*`)
**Target branch:** `reborn-integration` — the Reborn crates and contracts
referenced below exist on `reborn-integration`, not on `staging`. Any review or
implementation worktree must branch from `reborn-integration`.
**Related design:** nearai/ironclaw#4240
(`docs/superpowers/specs/2026-05-29-channel-communication-delivery-resolution.md`)

## 1. Purpose

Add a "trigger loop" to IronClaw Reborn: a way to start an LLM-driven agent
workflow from something other than a live human message. V1 delivers
**scheduled (cron) triggers** — "every morning at 8am, summarize my unread
mail." Webhook and message-regex triggers are planned fast-follow work and the
architecture must not preclude them.

**P0 implementation goal:** ship cron-backed scheduled trigger events first. P0
proves that a stored schedule can fire, enter the Reborn inbound/turn pipeline
as a synthetic inbound message, run in a new dedicated thread, and persist the
resulting thread. Fixed interval and one-shot schedules may share the same
schedule-provider implementation if they are cheap, but cron scheduled events
are the first acceptance target. Webhook, message-regex, and system-event
sources are not P0.

A trigger fire is treated as exactly what it is: a **synthetic inbound
message**. Instead of building a parallel execution engine, a fire fans into
the Reborn inbound pipeline (`InboundTurnService → TurnCoordinator →
TurnRunnerWorker → AgentLoopDriver`). The contracts and crates for that
pipeline exist on `reborn-integration` as implemented slices; full end-to-end
turn-coordination wiring is a Level-3 item still in progress per the contract
freeze index. This design depends on that wiring and must not ship before it.
The "job queue" a trigger extends is the Reborn turn queue.

The reusable abstraction is **trigger source provider**, not product adapter.
Product adapters normalize external products into IronClaw messages. Trigger
sources decide when a stored trigger should fire. Cron, webhook, and
message-regex support can share a `TriggerFire` path without pretending that
cron is an external transport.

## 2. Scope

### In V1

- Schedule trigger source: cron expression first; fixed interval and one-shot
  timestamp may ride along in the same provider if they do not expand the P0
  surface.
- `trigger_create` / `trigger_list` / `trigger_remove` capabilities, invoked
  through the Reborn capability/dispatch surface.
- Typed `TriggerRepository` with PostgreSQL + libSQL parity.
- A background `TriggerPollerWorker`.
- One new dedicated thread/conversation per fire.
- Delivery of the final turn output to a configured default notification
  target resolved by the communication delivery policy — gated on Reborn
  outbound being available (see §6).
- A contract extension to `ironclaw_conversations`: a host-trusted inbound
  ingress method (`handle_inbound_turn_with_trusted_scope`).

### Acceptance criterion

P0/V1 acceptance is cron-triggered events only. Webhook and regex sources are
explicitly **not** acceptance criteria for V1. If Reborn outbound is not ready
at implementation time, delivery (§6) drops to fast-follow and V1 acceptance is:
a cron trigger fires on schedule, runs a turn in a new dedicated thread, and the
thread persists.

### Deferred (fast-follow — architecture must leave room, no implementation in V1)

- Webhook / external HTTP trigger source.
- Regex-on-inbound-message trigger source.
- Internal system-event trigger source.
- `[SILENT]` delivery suppression (agent returns `[SILENT]` → skip delivery).
- Pre-run script injection (script runs before the agent, stdout becomes
  context) and its `{"wakeAgent":false}` wake-gate.
- Per-trigger delivery override (origin / specific channel / local-only).
- `skip-if-running` overlap guard.
- Distributed multi-poller lease.

## 3. Locked decisions

| Decision | Choice |
| --- | --- |
| Execution target per fire | One new dedicated thread/conversation per fire. |
| V1 trigger source | Cron/interval/once only; `TriggerSourceKind` stays an enum so other sources drop in later. |
| Management surface | `trigger_*` capabilities through the Reborn capability/dispatch surface, persisted to a typed repo. |
| Trigger scope | Inherits the creating user's `tenant/user/agent/project` scope, captured at create time (see §7, M4 — deliberate security decision). |
| Delivery | Final turn output delivered to a communication-policy-resolved target, gated on Reborn outbound. |
| Source abstraction | `TriggerSourceProvider` implementations emit `TriggerFire`; they are not product adapters. |
| Submission seam | `TriggerFire` becomes synthetic inbound through `ironclaw_conversations`, host-trusted ingress path. |

## 4. Verified pipeline and the trusted-ingress requirement

A cron fire is **host-internal**, not an untrusted product adapter. This drives
the one contract-sensitive piece of the design.

- `InboundTurnService::handle_inbound_turn()`
  (`crates/ironclaw_conversations/src/inbound.rs:54` on `reborn-integration`)
  calls `resolve_or_create_binding()` — the **untrusted** path. It fails closed
  for unpaired actors and does not trust requested scope hints
  (`conversation-binding.md` §4.2, §4.5).
- The binding service already exposes the correct seam:
  `ConversationBindingService::resolve_or_create_binding_with_trusted_scope(request, trusted_agent_id, trusted_project_id)`
  (`crates/ironclaw_conversations/src/traits.rs:26`). Trusted scope must come
  from host configuration and is persisted on first bind.
- `InboundTurnService` exposes a trusted variant that accepts a host-owned
  `TrustedInboundTurnRequest`.

**Required contract extension.** Add a facade method to `ironclaw_conversations`:

```
InboundTurnService::handle_inbound_turn_with_trusted_scope(
    request: TrustedInboundTurnRequest,
) -> Result<InboundTurnResponse, InboundTurnError>
```

Same body as `handle_inbound_turn` but routing binding resolution through
`resolve_or_create_binding_with_trusted_scope`. This is a contract extension to
`docs/reborn/contracts/conversation-binding.md` — a new required semantic:
*host-internal trusted ingress (scheduler/trigger) submits inbound turns with
host-vetted scope and does not require a paired external actor*. It must be
ratified as a contract change (Level-0 gate, see §10), not silently added.

**Rejected alternative.** Have `ironclaw_triggers` compose
`resolve_or_create_binding_with_trusted_scope` + `accept_inbound_message` +
`submit_turn` itself. This duplicates `InboundTurnService::submit_or_replay`
(`inbound.rs:91-151` — idempotency replay, submit-key rotation) — a second
dispatch pipeline, which `.claude/rules/architecture.md` smell #4 forbids. The
facade method is the honest fix.

### New-thread-per-fire is real, not faked

Conversation binding identity is the stable route tuple
`(space_id, conversation_id, thread_id)` (`conversation-binding.md` §4.8);
per-message external IDs do not fork threads. The synthetic
`ExternalConversationRef` therefore places a per-scheduled-slot value in the
**stable** `thread_id` route field each fire (see §5.5 for how that value is
derived deterministically). Binding resolution sees a novel stable identity for
each scheduled slot → creates exactly one new canonical thread and one
source/reply binding pair (`conversation-binding.md` §4.1).

### Idempotency contract (replaces the earlier hand-wave)

`InboundTurnService` already implements inbound idempotency:
`replay_accepted_inbound_message` looks up a prior acceptance keyed by
`(tenant_id, adapter_kind, adapter_installation_id, external_actor_ref,
external_conversation_ref, external_event_id)`; a hit replays the original
`AcceptedInboundMessage` and `SubmitTurnResponse` instead of submitting a second
turn (`conversation-binding.md` §11-12). The trigger system relies on this:
each fire supplies a **deterministic** route identity and
`external_event_id` (§5.5) so a re-attempt of the same scheduled slot — whether
from a poller crash-retry or a second poller instance — replays rather than
double-submits. These values are computed from `(trigger_id,
fire_slot)` at the trigger source-provider boundary and carried as
`TriggerFireIdentity`; `TriggerRecord` does **not** persist an
`external_event_id` or a separate trigger-fire idempotency ledger. The
conversation layer owns idempotency storage.

## 5. Components

### 5.1 New crate: `ironclaw_triggers`

Owns: the typed `TriggerRepository`, the `TriggerPollerWorker`, the `TriggerId`
/ `TriggerSchedule` / `TriggerSourceKind` domain types, trigger source
providers, `TriggerFire` construction, and the `trigger_*` capability handlers.
Does not own turn execution, binding internals, product adapter lifecycles, or
egress.

Dependency direction: `ironclaw_triggers` depends on `ironclaw_conversations`
(facade) and `ironclaw_host_api` (vocabulary). It must not depend upward on
product/runtime orchestration. `cargo test -p ironclaw_architecture` covers the
new edges.

### 5.2 Data model — `TriggerRecord`

All identifiers are newtypes per `.claude/rules/types.md`. All enums are
wire-stable (`#[serde(rename_all = "snake_case")]`).

```
TriggerId          ULID newtype
tenant_id          TenantId
creator_user_id    UserId
agent_id           Option<AgentId>      captured scope at create
project_id         Option<ProjectId>    captured scope at create
name               String               display
source             TriggerSourceKind    enum, V1 = Schedule only
schedule           TriggerSchedule       enum { Cron(expr), Interval(secs), Once(ts) }
prompt             String                workflow instruction
enabled            bool
state              TriggerState          enum { Scheduled, Paused, Completed }
next_run_at        DateTime              poller bookkeeping
last_run_at        Option<DateTime>
last_fired_slot    Option<DateTime>      last scheduled slot a fire was submitted for
last_status        Option<TriggerRunStatus>
created_at         DateTime
```

`TriggerRunStatus` (L1): `enum { Ok, Error, TimedOut, ApprovalBlocked }` —
`ApprovalBlocked` distinguishes "a tool inside the loop needed a human" (§7)
from a genuine loop failure; `TimedOut` distinguishes a stuck run.

**`last_status` semantics — sync vs async (H5).** `last_status = Ok` at step 4
of the poller loop means "successfully submitted to the turn queue" — a
synchronous outcome. `last_status = Error` means submission failed. The
post-run variants (`ApprovalBlocked`, `TimedOut`) require a turn-lifecycle
feedback path: `ironclaw_triggers` subscribes to an `AgentLoopDriver`
completion event (or equivalent Reborn observer interface) and updates
`last_status` asynchronously when the run ends. If no such observer is
available in V1, `ApprovalBlocked` and `TimedOut` move to fast-follow;
`trigger_list` shows `Ok` for submitted-but-blocked runs until the feedback
path is wired. Implementation must confirm whether Reborn exposes a
turn-lifecycle observer interface and, if so, wire it (H5).

**Required index.** `TriggerRepository` migrations must include a composite
index on `(tenant_id, enabled, state, next_run_at)` covering the poller query.
Without it the query is a full table scan that scales O(n) with trigger count.

`TriggerSourceKind` is a domain enum with only a `Schedule` variant in V1;
webhook / regex / system-event variants are added later without reshaping the
record. This is the trigger crate's own taxonomy — it is **not** the wire
`AdapterKind` (see §5.5, H1).

`TriggerSchedule::Cron(expr)` is validated at `trigger_create` time (L2): the
expression is parsed eagerly with a Rust cron crate (`cron` or `saffron` —
decide during implementation, prefer whichever the workspace already pulls in);
an invalid expression is rejected at create, never deferred to poll time. The
same crate computes `next_run_at`.

### 5.3 Source providers and `TriggerFire`

Trigger sources are provider implementations owned by `ironclaw_triggers`.
Their job is to decide **whether** a persisted trigger should fire, compute its
canonical fire slot, and produce a normalized `TriggerFire`:

```
TriggerFireIdentity {
    trigger_id,
    fire_slot,
    route_thread_id,     // digest("ironclaw-trigger-route", trigger_id, fire_slot)
    external_event_id,   // digest("ironclaw-trigger-event", trigger_id, fire_slot)
}

TriggerFire {
    identity,
    tenant_id,
    creator_user_id,
    agent_id,
    project_id,
    prompt,
}
```

`TriggerFireIdentity.fire_slot` is the provider's canonical dedupe coordinate.
For the V1 schedule provider it is the scheduled cron/interval/once slot.
Future providers must define an equivalent stable coordinate before they can
safely retry: a webhook source might use the validated webhook delivery id,
while a message-regex source might derive one from the source inbound message id
plus trigger id. The route and event ids are derived once when the identity is
constructed, then passed through to synthetic inbound construction.

V1 has one provider: `ScheduleTriggerSourceProvider`, driven by cron /
interval / once records. Future webhook and message-regex support should add
providers that emit the same `TriggerFire` shape:

- Webhook provider: owns HTTP/auth validation for trigger webhooks, then emits a
  `TriggerFire`; it may reuse ingress policy machinery, but it is not a product
  adapter unless it is normalizing an external product conversation.
- Message-regex provider: observes already-normalized inbound messages from real
  product adapters, matches trigger rules, then emits a `TriggerFire`; it must
  not re-normalize or re-bind the original product message.

This keeps ownership crisp: product adapters turn products into inbound user
messages; trigger source providers turn persisted trigger rules into trigger
fires; `InboundTurnService` starts the resulting agent turn.

### 5.4 `TriggerPollerWorker`

A background tokio task modelled on `TurnRunnerWorker`
(`crates/ironclaw_reborn/src/turn_runner.rs`), which is the existing precedent
for a long-lived Reborn background worker. Loop:

1. Tick every `poll_interval` (config, default ~30s).
2. Ask the schedule source provider to query `TriggerRepository` for `enabled &&
   state == Scheduled && next_run_at <= now`, system-wide across all tenants —
   tenant isolation is enforced by the captured scope on each `TriggerRecord`,
   not by query-level scoping. Apply `max_fires_per_tick` (config, default 50)
   as a LIMIT so a thundering-herd of co-scheduled triggers does not overflow a
   single tick; excess triggers fire on subsequent ticks.
3. For each `TriggerFire` emitted by the provider, check active turn-run count
   on threads belonging to this trigger (threads whose `conversation_id` in the
   synthetic `ExternalConversationRef` matches `identity.trigger_id`). If the
   count exceeds `max_concurrent_fires_per_trigger` (config, default 3), skip
   this trigger for the current tick — this is a lightweight V1 back-pressure
   substitute for the deferred `skip-if-running` guard and prevents unbounded
   thread accumulation for short-interval / long-running trigger combinations.
   Then:
   a. Materialize `prompt` into the transcript/content store → `content_ref`.
      During a crash-retry or dual-poller scenario the same prompt is
      materialized twice; this is safe provided the content store is
      content-addressed (same bytes → same ref). Implementation must confirm
      content-store semantics; prefer content-addressed storage.
   b. Build the synthetic `InboundTurnRequest` (§5.5) with
      `identity.route_thread_id` and `identity.external_event_id`.
   c. Call `handle_inbound_turn_with_trusted_scope(trusted_req)`.
4. On submit success: set `last_run_at`,
   `last_fired_slot = identity.fire_slot`,
   `last_status = Ok` (= "submitted to turn queue"), recompute `next_run_at =
   next_slot_after(now)` — advancing from **now**, not from
   `last_fired_slot`. This means missed slots are skipped rather than
   back-filled: if the system was down and multiple cron slots were overdue,
   V1 fires once (the most-recently-overdue slot) and resumes from the next
   future slot. For `Once`, set `state = Completed`.
5. On submit failure: set `last_status = Error`; leave `next_run_at` so the
   next tick retries. Errors are surfaced via `trigger_list`, never silently
   swallowed (`.claude/rules/error-handling.md`).

**At-least-once and the crash window (M1, M2).** The poller is not transactional
across "submit turn" and "persist `last_fired_slot`". A crash in that window,
or a second poller instance during a rolling deploy, will re-attempt the same
scheduled slot. This is **safe by construction**: the provider derives
`TriggerFireIdentity` deterministically from `(trigger_id, fire_slot)`, so step
3b reuses the same stable route identity and `external_event_id`; the re-attempt
hits `InboundTurnService` idempotency replay (§4) and returns the original turn
instead of creating a duplicate. No advisory lock is needed for correctness. The
deterministic slot — not a random
sequence number — is the mechanism; the earlier `{trigger_id}:{fire_seq}` form
was wrong because two pollers would mint different sequence numbers. A single
poller instance is still the V1 default for simplicity; a distributed lease
remains deferred, now as an efficiency optimization rather than a correctness
fix.

The worker is started by the Reborn composition root — the same startup path
that spawns `TurnRunnerWorker`. `ironclaw_reborn_composition` owns wiring it
from config; the worker code lives in `ironclaw_triggers`. Implementation must
confirm the composition root exposes a background-worker spawn hook and add one
if it does not (H3).

### 5.5 Synthetic `InboundTurnRequest` per fire

| Field | Value |
| --- | --- |
| `adapter_kind` | a reserved host-internal ingress value — see note below (H1) |
| `external_conversation_ref` | `{ space_id: "trigger", conversation_id: identity.trigger_id, thread_id: identity.route_thread_id }` → one new thread per fire slot |
| `external_event_id` | `identity.external_event_id` |
| `actor` | `TurnActor { user_id: creator_user_id }` — the creator's real authority, not a fake system actor |
| `content_ref` | trigger prompt, materialized into the content store |
| `route_kind` | direct |

**`adapter_kind` and the transport question (H1).** A trigger fire is not a
product adapter event — it is a host-internal synthetic event produced by a
trigger source provider (§5.3). The trigger crate's own taxonomy is
`TriggerSourceKind` (§5.2). The wire `adapter_kind` on `InboundTurnRequest` is a
separate concern: it identifies the *ingress* to the conversation layer for
binding and idempotency. The trusted-ingress contract extension (§4) must define
how host-internal trigger ingress is represented in `adapter_kind` — either a
reserved value dedicated to host-internal trusted ingress, or a representation
that the conversation contract explicitly marks as non-transport and
unconstructible by product adapters. This is an open contract-extension
question to settle during Level-0 ratification (§10); the design does not
assume a specific `AdapterKind::Trigger` variant.

The per-slot `thread_id` is deterministic, not stored. It is distinct from
`external_event_id`: `identity.route_thread_id` gives one canonical thread per
scheduled slot, while `identity.external_event_id` gives inbound idempotency. A
crash-retry of the same slot recomputes the same `TriggerFireIdentity` and
replays; the replayed
`AcceptedInboundMessage` carries the original thread, so a retry does not
strand a second empty thread. A future trigger-fire ledger could replace this
computed inbound event identity, but V1 should not duplicate idempotency storage
that the conversation layer already owns.

### 5.6 Capabilities (`trigger_*`)

`trigger_create`, `trigger_list`, `trigger_remove` are exposed through the
Reborn capability/dispatch surface (`ironclaw_capabilities` /
`ironclaw_dispatcher`), not the legacy `src/tools` `ToolDispatcher`. This gives
trigger management the same authorization, audit, and scope mediation as any
other Reborn capability (CLAUDE.md "Everything Goes Through Tools", applied to
the Reborn surface). Implementation must confirm the exact registration path in
`ironclaw_capabilities` and how a capability handler receives its
`TriggerRepository` dependency (M3) — likely via the host runtime service
bundle that already carries other repositories.

- `trigger_create(name, schedule, prompt)` — validates the schedule, writes a
  `TriggerRecord`; scope fields stamped from the caller's invocation context.
- `trigger_list()` — caller-scoped list, includes `last_status` so failures are
  visible.
- `trigger_remove(trigger_id)` — caller-scoped delete.

## 6. Execution and delivery

### Execution

The submitted turn rides the normal Reborn queue: `submit_turn` →
one-active-run-per-thread gate → `TurnRunnerWorker` claims the run →
`AgentLoopDriver` runs the LLM loop. No new execution machinery. Each fire is
its own thread, so a trigger never contends with itself for the active-run
lock.

### Delivery (H2 — dependency-gated)

A dedicated trigger thread has no real external channel binding. The intended
mechanism: on turn completion for a trigger thread, route the final assistant
message through the communication delivery resolver
(`2026-05-29-channel-communication-delivery-resolution.md`, proposed in
nearai/ironclaw#4240) and then through Reborn outbound (`ironclaw_outbound`).
The resolver is owned by
`ironclaw_outbound` and receives enough context to distinguish triggered jobs
from live user messages and approval-needed events; it returns only a candidate
`ReplyTargetBindingRef`. `ironclaw_outbound` revalidates the target before send.
The synthetic inbound request owns only trigger ingress identity; it must not
smuggle the default notification destination through inbound binding state. This
must **not** be a direct
`SseManager::broadcast` call (`.claude/rules/gateway-events.md`).

**Honest dependency note.** Reborn user-facing event/SSE transport,
communication delivery resolution, and full product-channel egress are not all
implemented yet. V1 delivery therefore requires: (a) `ironclaw_outbound` able
to validate and prepare delivery to at least one channel, and (b) a configured,
bound communication target for the trigger creator. If either is missing at
implementation time, delivery moves to fast-follow and V1 acceptance is the
reduced criterion in §2 (trigger fires, turn runs, thread persists — the user
reads the thread directly). Delivery design is not on the critical path for
proving the trigger loop itself works.

## 7. Authority and failure semantics

- **Unattended approvals.** A triggered turn runs with no human present.
  Approvals are exact-invocation leases with no auto-approve (`approvals.md`).
  A tool call inside the loop that requires approval **fails closed** — there
  is no live human to grant it. The normal run-state/event path records
  `ApprovalNeeded` / `BlockedApproval`; communication delivery resolution may
  notify the user's approval target if the target supports gate prompts, but it
  does not approve, deny, or resume the invocation. When the turn-lifecycle
  feedback path is wired (H5), the run updates `last_status = ApprovalBlocked`;
  before that path exists, the blocked status is visible in the thread's turn
  history but not reflected in `TriggerRecord.last_status`. V1 recommends
  triggered runs use a run profile whose tool ceiling avoids approval-gated
  tools. A failed-closed approval inside a triggered run is acceptable V1
  behavior, not a bug.
- **Create-time scope capture plus fire-time revalidation (M4).** A trigger
  captures `tenant/user/agent/project` at `trigger_create` time so schedule
  execution is stable and auditable. Before each fire is submitted, the trigger
  worker must revalidate that the creator and captured agent/project scope are
  still authorized. If the tenant, user, agent, or project access has been
  revoked, the fire is rejected fail-closed, `last_status = Error` (or a more
  specific revocation status once modeled), and the trigger should be disabled
  or surfaced for owner/admin action. A durable trigger is not a way to preserve
  revoked authority.
- **Fail-closed submission.** Binding or scope errors set `last_status = Error`
  and surface in `trigger_list`. No `unwrap_or_default` / `.ok()?` on repo or
  ingress calls.
- **Overlap.** Two fires close together produce two threads that both run; the
  dedicated-thread model permits this. V1 allows overlap; a `skip-if-running`
  guard is deferred.
- **Redaction.** The trigger prompt is user content — it crosses the inbound
  boundary as a `content_ref` and never appears in turn state, lifecycle
  events, or logs (`conversation-binding.md` §20, `ironclaw_turns` guardrails).
- **Scope flow.** `tenant / user / agent / project` flows unbroken:
  `trigger_create` → `TriggerRecord` → synthetic inbound → trusted binding →
  `TurnScope` → agent loop. No axis is dropped.
- **Trusted ingress is type-sealed.** The host-internal trigger ingress marker
  and trusted-scope witness must be unconstructible by product adapters. The
  conversation-binding contract must reject host-internal trusted ingress values
  from all untrusted product-adapter paths; a plain public `adapter_kind` value
  is not sufficient.

## 8. Testing

- **Unit:** `next_run_at` / scheduled-slot computation for cron / interval /
  once; cron expression validation rejects bad input at create;
  `TriggerSchedule` / `TriggerRunStatus` serde round-trip; deterministic
  `TriggerFireIdentity` derivation is stable for a fixed `(trigger_id, slot)`
  and changes for a different slot.
- **Caller-level (required — the poller gates turn submission, a side effect):**
  drive `TriggerPollerWorker` against a real `InboundTurnService` plus an
  in-memory `TurnCoordinator`, and assert:
  1. each fire creates a new canonical thread;
  2. binding resolution receives the trusted scope;
  3. a re-run of the same scheduled slot (simulated crash-retry and simulated
     second poller) recomputes the same `TriggerFireIdentity` and replays rather
     than double-submitting.
- **Capability tests** exercise `trigger_*` through the Reborn
  capability/dispatch surface, not the handler in isolation
  (`.claude/rules/testing.md` — test through the caller).
- **Persistence parity:** PostgreSQL and libSQL tests for `TriggerRepository`,
  with migration coverage.
- **Architecture:** `cargo test -p ironclaw_architecture` after the new crate
  and its dependency edges land.
- Per-crate `cargo fmt`, `cargo clippy`, `cargo test`, `cargo doc` evidence for
  touched crates.

## 9. Contract / doc updates required

- `docs/reborn/contracts/conversation-binding.md` — add the host-trusted
  ingress requirement, the `handle_inbound_turn_with_trusted_scope` facade
  method, and the host-internal `adapter_kind` representation (§5.5, H1).
- A new contract doc for the trigger system covering the `TriggerRecord` model,
  source-provider boundary, `TriggerFireIdentity`, poller semantics,
  deterministic-slot idempotency, and scope rules.
- The communication delivery resolution design from nearai/ironclaw#4240
  (`2026-05-29-channel-communication-delivery-resolution.md`) must be promoted
  to contracts before trigger delivery or approval notifications ship.
- `docs/reborn/2026-04-25-current-architecture-map.md` — add `ironclaw_triggers`
  once the slice lands.

## 10. Build sequence (informative — full plan is a separate document)

**Level-0 gate (must ratify before implementation).** The trusted-ingress
contract extension to `conversation-binding.md` (§4) and the host-internal
`adapter_kind` representation (§5.5) must be written and ratified first. This
also depends on the Reborn turn-coordination wiring (Level-3 freeze-index item)
being far enough along to run an end-to-end turn.

1. Contract: `handle_inbound_turn_with_trusted_scope` + host-internal ingress
   representation in `conversation-binding.md`; ratify.
2. Implement `handle_inbound_turn_with_trusted_scope` in
   `ironclaw_conversations` with caller-level tests.
3. New crate `ironclaw_triggers`: `TriggerRecord`, `TriggerRepository` trait,
   `TriggerSourceProvider` / `TriggerFire` / `TriggerFireIdentity` domain
   types, cron validation, in-memory implementation.
4. PostgreSQL + libSQL `TriggerRepository` implementations + parity tests.
5. `TriggerPollerWorker` + caller-level tests (including slot-replay).
6. `trigger_*` capabilities + registration on the Reborn capability surface.
7. Delivery wiring through communication delivery resolution and Reborn outbound
   — only if both are ready (§6); otherwise fast-follow.
8. Composition wiring in `ironclaw_reborn_composition`; architecture tests.

## 11. Rejected review findings (for the record)

The 2026-05-21 spec review was conducted against a worktree based on `staging`,
where the Reborn crates do not exist. The following findings are artifacts of
that branch mismatch and are rejected; the files exist on `reborn-integration`:

- **C1** — `ironclaw_conversations` (`inbound.rs`, `traits.rs`) and
  `conversation-binding.md` exist on `reborn-integration`. Real action taken:
  this doc and the implementation worktree now target `reborn-integration`
  explicitly.
- **C2 (partial)** — `InboundTurnService`, `TurnCoordinator`,
  `TurnRunnerWorker`, `AgentLoopDriver` exist as implemented slices. The valid
  half — full turn-coordination wiring is still in progress — is now addressed
  in §1 and the §10 Level-0 gate.
- **H4** — `InboundTurnService::submit_or_replay` and the idempotency mechanism
  exist (`inbound.rs:91-151`). The valid half — specify the contract — is now
  addressed in §4 "Idempotency contract".
- **M5** — the cited `.claude/rules/*.md` files exist on `reborn-integration`.
