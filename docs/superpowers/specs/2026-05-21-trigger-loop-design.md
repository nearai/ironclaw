# Trigger Loop — Design

**Date:** 2026-05-21
**Status:** Design approved, revised after spec review
**Target architecture:** IronClaw Reborn (`crates/ironclaw_*`)
**Target branch:** `reborn-integration` — the Reborn crates and contracts
referenced below exist on `reborn-integration`, not on `staging`. Any review or
implementation worktree must branch from `reborn-integration`.

## 1. Purpose

Add a "trigger loop" to IronClaw Reborn: a way to start an LLM-driven agent
workflow from something other than a live human message. V1 delivers
**scheduled (cron) triggers** — "every morning at 8am, summarize my unread
mail." Webhook and message-regex triggers are planned fast-follow work and the
architecture must not preclude them.

A trigger fire is treated as exactly what it is: a **synthetic inbound
message**. Instead of building a parallel execution engine, a fire fans into
the Reborn inbound pipeline (`InboundTurnService → TurnCoordinator →
TurnRunnerWorker → AgentLoopDriver`). The contracts and crates for that
pipeline exist on `reborn-integration` as implemented slices; full end-to-end
turn-coordination wiring is a Level-3 item still in progress per the contract
freeze index. This design depends on that wiring and must not ship before it.
The "job queue" a trigger extends is the Reborn turn queue.

## 2. Scope

### In V1

- Schedule trigger source: cron expression, fixed interval, one-shot timestamp.
- `trigger_create` / `trigger_list` / `trigger_remove` capabilities, invoked
  through the Reborn capability/dispatch surface.
- Typed `TriggerRepository` with PostgreSQL + libSQL parity.
- A background `TriggerPollerWorker`.
- One new dedicated thread/conversation per fire.
- Delivery of the final turn output to a configured default notification
  channel — gated on Reborn outbound being available (see §6).
- A contract extension to `ironclaw_conversations`: a host-trusted inbound
  ingress method (`handle_inbound_turn_with_trusted_scope`).

### Acceptance criterion

Cron triggers only. Webhook and regex sources are explicitly **not** acceptance
criteria for V1. If Reborn outbound is not ready at implementation time,
delivery (§6) drops to fast-follow and V1 acceptance is: a trigger fires on
schedule, runs a turn in a new dedicated thread, and the thread persists.

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
| Delivery | Final turn output delivered to a configured default notification channel, gated on Reborn outbound. |
| Submission seam | Synthetic inbound through `ironclaw_conversations`, host-trusted ingress path. |

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
- `InboundTurnService` does **not** currently expose a trusted variant.

**Required contract extension.** Add a facade method to `ironclaw_conversations`:

```
InboundTurnService::handle_inbound_turn_with_trusted_scope(
    request: InboundTurnRequest,
    trusted_agent_id: Option<AgentId>,
    trusted_project_id: Option<ProjectId>,
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
`ExternalConversationRef` therefore places a fresh value in the **stable**
`thread_id` route field each fire (see §5.4 for how that value is derived
deterministically). Binding resolution sees a novel stable identity → creates
exactly one new canonical thread and one source/reply binding pair
(`conversation-binding.md` §4.1).

### Idempotency contract (replaces the earlier hand-wave)

`InboundTurnService` already implements inbound idempotency:
`replay_accepted_inbound_message` looks up a prior acceptance keyed by
`(tenant_id, adapter_kind, adapter_installation_id, external_actor_ref,
external_conversation_ref, external_event_id)`; a hit replays the original
`AcceptedInboundMessage` and `SubmitTurnResponse` instead of submitting a second
turn (`conversation-binding.md` §11-12). The trigger system relies on this:
each fire supplies a **deterministic** `external_event_id` (§5.4) so a
re-attempt of the same scheduled slot — whether from a poller crash-retry or a
second poller instance — replays rather than double-submits. The trigger
system stores nothing of its own for idempotency; the conversation layer owns
it.

## 5. Components

### 5.1 New crate: `ironclaw_triggers`

Owns: the typed `TriggerRepository`, the `TriggerPollerWorker`, the `TriggerId`
/ `TriggerSchedule` / `TriggerSourceKind` domain types, and the `trigger_*`
capability handlers. Does not own turn execution, binding internals, or egress.

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
delivery           TriggerDelivery       enum, V1 = DefaultChannel
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

`TriggerSourceKind` is a domain enum with only a `Schedule` variant in V1;
webhook / regex / system-event variants are added later without reshaping the
record. This is the trigger crate's own taxonomy — it is **not** the wire
`AdapterKind` (see §5.4, H1).

`TriggerSchedule::Cron(expr)` is validated at `trigger_create` time (L2): the
expression is parsed eagerly with a Rust cron crate (`cron` or `saffron` —
decide during implementation, prefer whichever the workspace already pulls in);
an invalid expression is rejected at create, never deferred to poll time. The
same crate computes `next_run_at`.

### 5.3 `TriggerPollerWorker`

A background tokio task modelled on `TurnRunnerWorker`
(`crates/ironclaw_reborn/src/turn_runner.rs`), which is the existing precedent
for a long-lived Reborn background worker. Loop:

1. Tick every `poll_interval` (config, default ~30s).
2. Query `TriggerRepository` for `enabled && state == Scheduled &&
   next_run_at <= now`, scoped per tenant.
3. For each due trigger, compute the **scheduled fire slot** — the canonical
   timestamp the trigger was due for (the cron/interval slot, not wall-clock
   now). Then:
   a. Materialize `prompt` into the transcript/content store → `content_ref`.
   b. Build the synthetic `InboundTurnRequest` (§5.4) with a deterministic
      `external_event_id` derived from `(trigger_id, scheduled_fire_slot)`.
   c. Call `handle_inbound_turn_with_trusted_scope(req, agent_id, project_id)`.
4. On submit success: set `last_run_at`, `last_fired_slot = scheduled_slot`,
   `last_status = Ok`, recompute `next_run_at`. For `Once`, set
   `state = Completed`.
5. On submit failure: set `last_status = Error`; leave `next_run_at` so the
   next tick retries. Errors are surfaced via `trigger_list`, never silently
   swallowed (`.claude/rules/error-handling.md`).

**At-least-once and the crash window (M1, M2).** The poller is not transactional
across "submit turn" and "persist `last_fired_slot`". A crash in that window,
or a second poller instance during a rolling deploy, will re-attempt the same
scheduled slot. This is **safe by construction**: step 3b derives
`external_event_id` deterministically from `(trigger_id, scheduled_fire_slot)`,
so the re-attempt hits `InboundTurnService` idempotency replay (§4) and returns
the original turn instead of creating a duplicate. No advisory lock is needed
for correctness. The deterministic slot — not a random sequence number — is the
mechanism; the earlier `{trigger_id}:{fire_seq}` form was wrong because two
pollers would mint different sequence numbers. A single poller instance is
still the V1 default for simplicity; a distributed lease remains deferred, now
as an efficiency optimization rather than a correctness fix.

The worker is started by the Reborn composition root — the same startup path
that spawns `TurnRunnerWorker`. `ironclaw_reborn_composition` owns wiring it
from config; the worker code lives in `ironclaw_triggers`. Implementation must
confirm the composition root exposes a background-worker spawn hook and add one
if it does not (H3).

### 5.4 Synthetic `InboundTurnRequest` per fire

| Field | Value |
| --- | --- |
| `adapter_kind` | a reserved host-internal ingress value — see note below (H1) |
| `external_conversation_ref` | `{ space_id: "trigger", conversation_id: trigger_id, thread_id: <per-fire ULID> }` → new thread |
| `external_event_id` | deterministic: digest of `(trigger_id, scheduled_fire_slot)` |
| `actor` | `TurnActor { user_id: creator_user_id }` — the creator's real authority, not a fake system actor |
| `content_ref` | trigger prompt, materialized into the content store |
| `route_kind` | direct |

**`adapter_kind` and the transport question (H1).** A trigger fire is not a
transport adapter — it is a host-internal synthetic event. The trigger crate's
own taxonomy is `TriggerSourceKind` (§5.2). The wire `adapter_kind` on
`InboundTurnRequest` is a separate concern: it identifies the *ingress* to the
conversation layer. The trusted-ingress contract extension (§4) must define how
host-internal ingress is represented in `adapter_kind` — either a reserved
value dedicated to host-internal trusted ingress, or a representation that the
conversation contract explicitly marks as non-transport. This is an open
contract-extension question to settle during Level-0 ratification (§10); the
design does not assume a specific `AdapterKind::Trigger` variant.

The per-fire `thread_id` ULID is fresh per fire so each fire forks a new
canonical thread. It is distinct from `external_event_id`: the ULID gives
thread uniqueness, the deterministic `external_event_id` gives idempotency. A
crash-retry of the same slot reuses the same `external_event_id` and replays —
the replayed `AcceptedInboundMessage` carries the original thread, so a retry
does not strand a second empty thread.

### 5.5 Capabilities (`trigger_*`)

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
message to a configured **default notification channel** through Reborn
outbound (`ironclaw_outbound`), using a validated reply target — the trigger's
inbound carries a `reply_target_binding_ref` addressing the default channel's
binding, and egress delivers through the standard validated reply-target path.
This must **not** be a direct `SseManager::broadcast` call
(`.claude/rules/gateway-events.md`).

**Honest dependency note.** Reborn user-facing event/SSE transport and full
product-channel egress are not all implemented yet. V1 delivery therefore
requires: (a) `ironclaw_outbound` able to deliver to at least one channel, and
(b) a configured, bound default-notification destination. If either is missing
at implementation time, delivery moves to fast-follow and V1 acceptance is the
reduced criterion in §2 (trigger fires, turn runs, thread persists — the user
reads the thread directly). Delivery design is not on the critical path for
proving the trigger loop itself works.

## 7. Authority and failure semantics

- **Unattended approvals.** A triggered turn runs with no human present.
  Approvals are exact-invocation leases with no auto-approve (`approvals.md`).
  A tool call inside the loop that requires approval **fails closed** — there
  is no human to grant it. The run records `last_status = ApprovalBlocked`.
  V1 recommends triggered runs use a run profile whose tool ceiling avoids
  approval-gated tools. A failed-closed approval inside a triggered run is
  acceptable V1 behavior, not a bug.
- **Create-time scope capture is a deliberate security decision (M4).** A
  trigger captures `tenant/user/agent/project` at `trigger_create` time and
  runs with that scope on every fire. If the creator's agent/project access is
  later revoked, the trigger still fires with the originally captured scope
  until it is removed. This is intentional: a trigger is a durable artifact
  owned by its creator, not a live re-evaluation of current access. It is
  documented here so it is a decision, not an accident. Revocation of a
  trigger's authority is done by disabling/removing the trigger, or — as
  fast-follow — by a re-validation step at fire time.
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

## 8. Testing

- **Unit:** `next_run_at` / scheduled-slot computation for cron / interval /
  once; cron expression validation rejects bad input at create;
  `TriggerSchedule` / `TriggerRunStatus` serde round-trip; deterministic
  `external_event_id` derivation is stable for a fixed `(trigger_id, slot)`.
- **Caller-level (required — the poller gates turn submission, a side effect):**
  drive `TriggerPollerWorker` against a real `InboundTurnService` plus an
  in-memory `TurnCoordinator`, and assert:
  1. each fire creates a new canonical thread;
  2. binding resolution receives the trusted scope;
  3. a re-run of the same scheduled slot (simulated crash-retry and simulated
     second poller) replays via the deterministic `external_event_id` rather
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
  method, and the host-internal `adapter_kind` representation (§5.4, H1).
- A new contract doc for the trigger system covering the `TriggerRecord` model,
  poller semantics, deterministic-slot idempotency, and scope rules.
- `docs/reborn/2026-04-25-current-architecture-map.md` — add `ironclaw_triggers`
  once the slice lands.

## 10. Build sequence (informative — full plan is a separate document)

**Level-0 gate (must ratify before implementation).** The trusted-ingress
contract extension to `conversation-binding.md` (§4) and the host-internal
`adapter_kind` representation (§5.4) must be written and ratified first. This
also depends on the Reborn turn-coordination wiring (Level-3 freeze-index item)
being far enough along to run an end-to-end turn.

1. Contract: `handle_inbound_turn_with_trusted_scope` + host-internal ingress
   representation in `conversation-binding.md`; ratify.
2. Implement `handle_inbound_turn_with_trusted_scope` in
   `ironclaw_conversations` with caller-level tests.
3. New crate `ironclaw_triggers`: `TriggerRecord`, `TriggerRepository` trait,
   domain enums, cron validation, in-memory implementation.
4. PostgreSQL + libSQL `TriggerRepository` implementations + parity tests.
5. `TriggerPollerWorker` + caller-level tests (including slot-replay).
6. `trigger_*` capabilities + registration on the Reborn capability surface.
7. Delivery wiring to the default notification channel — only if Reborn
   outbound is ready (§6); otherwise fast-follow.
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
