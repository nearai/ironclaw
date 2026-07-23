# Reborn Contract — Heartbeat

**Status:** Contract freeze
**Date:** 2026-07-23
**Parent:** #6369
**Depends on:** [`triggers.md`](triggers.md), [`memory.md`](memory.md),
[`turns-agent-loop.md`](turns-agent-loop.md),
[`communication-delivery-resolution.md`](communication-delivery-resolution.md)

---

## 1. Purpose

Heartbeat is an opt-in, scoped automation that periodically evaluates the
owner's current `HEARTBEAT.md` checklist through the canonical Reborn turn
pipeline. It is not a process-liveness heartbeat, a runner lease heartbeat, or
a second scheduler.

This contract freezes the ownership, schedule, prompt, result, delivery,
restart, and failure semantics needed by #6570 and #6571. It does not enable a
heartbeat or change any runtime default.

## 2. Architecture decision

Heartbeat is a thin domain service backed by an ordinary durable recurring
`TriggerRecord`.

The service owns typed heartbeat configuration and reconciles that
configuration to one system-managed trigger per complete owner scope. The
existing trigger repository, `TriggerPollerWorker`, trusted trigger ingress,
canonical runner/driver/agent loop, active-fire claim, deterministic fire
identity, and triggered-run delivery path remain authoritative.

Heartbeat must not add:

- another timer loop or due-work query;
- an in-memory schedule or process-local dedupe ledger;
- direct construction of `TrustedInboundTurnRequest`;
- direct OS reads of `HEARTBEAT.md`;
- a heartbeat-only agent loop or capability bypass;
- direct product/channel sends from the trigger poller.

The system-managed heartbeat trigger is not exposed as a new trigger source
kind. It remains `TriggerSourceKind::Schedule`; a typed, host-owned heartbeat
marker distinguishes its materialization and completion policy from
user-authored scheduled prompts. User-facing trigger mutation surfaces must not
be able to manufacture that marker.

## 3. Ownership

| Owner | Responsibility | Forbidden responsibility |
| --- | --- | --- |
| `ironclaw_triggers` | Typed heartbeat config, validation, deterministic system-trigger identity, reconciliation request/result, durable schedule/failure metadata contracts | Reading memory, authorizing users, running turns, delivering replies |
| Memory service owner | Scoped `HEARTBEAT.md` document read at fire time | Scheduling, trusted ingress, outbound delivery |
| Reborn composition | Wire heartbeat reconciliation and a memory-backed prompt materializer into existing trigger ports; stamp automation origin | Domain persistence, alternate scheduler, trusted request construction |
| Trigger poller and repository | Due selection, atomic claim, persisted next slot/active fire, restart recovery, duplicate suppression | Heartbeat prompt policy, sentinel classification, outbound send |
| Canonical turn pipeline | Admission, authorization, runner/driver/loop, capability policy, terminal outcome | Special heartbeat execution bypass |
| Product-workflow triggered delivery | Classify the terminal heartbeat result and resolve/send an approved outbound target | Trigger identity, schedule state, direct provider credentials |

`HEARTBEAT.md` stays owned by the memory service described in
[`memory.md`](memory.md). The trigger record must not store a copy of the file
as its prompt because that copy becomes stale and bypasses scoped memory reads.

## 4. Scope and identity

A heartbeat is keyed by the complete captured scope:

```text
HeartbeatScope {
    tenant_id,
    creator_user_id,
    agent_id,
    project_id,
}
```

No field is a wildcard. `None` for `agent_id` or `project_id` is an exact
scope value, not permission to read all agents or projects.

For each scope, reconciliation produces at most one system-managed heartbeat
trigger. Its stable identity is derived with a versioned, domain-separated,
length-prefixed digest of all scope fields. Raw string concatenation and
display-label matching are forbidden. The trigger's normal fire identity still
uses the existing `(tenant_id, trigger_id, fire_slot)` derivation.

The trusted poller mints the turn actor and scope from the persisted trigger
record. It must re-check fire-time authorization and must never substitute an
ambient user, tenant, agent, or project. A heartbeat run keeps the generic
`TurnOriginKind::ScheduledTrigger` origin and additionally records trusted
automation provenance `RoutineId("heartbeat")`; adapters and untrusted product
code cannot mint that provenance. Model-initiated capability calls inside the
run remain `InvocationOrigin::LoopRun` and do not inherit scheduler authority.

## 5. Typed configuration

The domain configuration shape is:

```text
HeartbeatConfig {
    enabled: bool,
    interval: HeartbeatInterval,
    timezone: IanaTimezone,
    quiet_hours: Option<HeartbeatQuietHours>,
    delivery_target: Option<TriggerDeliveryTargetId>,
    failure_limit: NonZeroU32,
}

HeartbeatQuietHours {
    start: LocalTime,
    end: LocalTime,
}
```

Rules:

- heartbeat is disabled by default and must be explicitly enabled;
- `interval` is converted to a cron schedule accepted by the existing trigger
  contract and cannot be more frequent than once per minute;
- `timezone` is a valid IANA timezone and controls interval evaluation and
  quiet-hours interpretation; persisted due slots are UTC instants;
- equal quiet-hour endpoints mean a full-day quiet window, not an empty one;
- a window whose end precedes its start crosses local midnight;
- daylight-saving gaps or overlaps follow the trigger schedule's timezone
  rules and must not create two fires for one canonical UTC slot;
- `delivery_target` is an opaque, creator-scoped target resolved again at
  delivery time; it is not part of heartbeat or fire identity;
- `failure_limit` is bounded by the domain and controls when automatic
  scheduling is disabled after consecutive terminal failures.

The standalone binary promotes the dependency-neutral boot config into this
typed shape. A minimal enabled configuration is:

```toml
[trigger_poller]
enabled = true

[heartbeat]
enabled = true
interval_minutes = 30
timezone = "UTC"
failure_limit = 3

[heartbeat.quiet_hours]
start = "22:00"
end = "07:00"
```

An absent `[heartbeat]` section creates no managed trigger. A present section
defaults to `enabled = false`; enabling heartbeat while the trigger poller is
disabled fails boot rather than silently accepting a schedule that cannot run.

Configuration persistence and the managed trigger update are one logical
reconciliation operation. Implementations use the repository's transaction or
bounded CAS semantics so a crash cannot leave two active heartbeat triggers for
one scope. Reapplying identical configuration is idempotent.

Changing configuration updates the managed trigger without changing its stable
identity. Disabling pauses the managed trigger and prevents new claims; it does
not cancel a turn that was already accepted. Re-enabling computes the next
future eligible slot and does not replay every slot missed while disabled.

## 6. Due-slot and quiet-hours semantics

The existing trigger poller is the only component that polls for due work.

For an enabled heartbeat:

1. the normal repository due query returns its managed `TriggerRecord`;
2. the normal atomic claim permits at most one active fire;
3. the heartbeat schedule policy evaluates the due slot in the configured
   timezone;
4. a slot inside quiet hours advances to the first eligible future schedule
   slot without submitting a turn;
5. an eligible slot enters normal prompt materialization and trusted ingress.

Multiple poll ticks for the same slot reuse the same fire identity. They must
submit at most one turn. A restart recovers from durable trigger claim and
conversation idempotency state; it must not mint a replacement identity.

Quiet-hour suppression is not a failed run and does not increment the
consecutive-failure counter. It must be persisted by advancing `next_run_at`,
so restart does not reconsider the same suppressed slot.

## 7. `HEARTBEAT.md` materialization

The materializer reads `HEARTBEAT.md` at fire time through the scoped memory
document service. It never uses `std::fs`, a workspace path, or a copy embedded
in `TriggerRecord.prompt`.

Materialization outcomes are:

| Document state | Outcome |
| --- | --- |
| Missing | Suppress the slot without submitting a turn |
| Present but empty after Unicode whitespace trimming | Suppress the slot without submitting a turn |
| Present and non-empty | Build the bounded heartbeat instruction envelope and submit normally |
| Memory backend unavailable | Retryable materialization failure |
| Scope/policy denied or malformed/oversized content | Permanent materialization failure for that slot |

Missing and empty documents are normal no-work outcomes. They advance the
schedule, do not increment the failure counter, and produce no outbound
delivery. Prompt content remains subject to existing injection scanning,
maximum-size checks, redaction, and personal/group scope rules.

The heartbeat envelope identifies the run as a periodic checklist evaluation,
includes the current authorized document content, and asks for either an
actionable result or the exact successful sentinel. It must not weaken the
scheduled-trigger capability profile.

## 8. Completion and delivery

The exact successful no-action sentinel is:

```text
HEARTBEAT_OK
```

Classification happens only after the canonical turn has reached a successful
terminal outcome:

- output equal to `HEARTBEAT_OK` after trimming leading and trailing Unicode
  whitespace suppresses outbound delivery;
- any other successful non-empty output, including different case, added
  punctuation, Markdown, or surrounding prose, is actionable and follows the
  existing triggered-run delivery path;
- an empty successful result is treated as no actionable result and produces
  no outbound delivery;
- a failed, cancelled, recovery-required, or gate-expired turn is not success
  and must never emit a success delivery.

Near-sentinel output is deliberately not suppressed. For example,
`HEARTBEAT_OK.` and `Status: HEARTBEAT_OK` are actionable.

Delivery uses the trigger's typed delivery target when present, otherwise the
existing creator preference fallback. Target ownership, authorization,
approval, redaction, credential mediation, provider-issued evidence, and
read-back verification remain owned by the ordinary delivery-resolution path.
A missing, foreign, disconnected, or removed target fails closed.

Completion/delivery dedupe is keyed by the accepted trigger fire/run and
delivery evidence. Replayed completion notifications must not send a second
message.

## 9. Failure, backoff, and recovery

Heartbeat maintains a durable, bounded consecutive-failure count and the next
eligible retry instant. Only terminal execution failures and delivery failures
increment the counter. Successful sentinel suppression, successful actionable
delivery, and normal missing/empty/quiet suppression reset it to zero.

Backoff is exponential from the configured heartbeat interval, capped at 24
hours, with overflow-safe arithmetic:

```text
retry_delay = min(interval * 2^(consecutive_failures - 1), 24 hours)
```

The persisted next eligible instant is authoritative across restart. A retry
uses the original accepted fire/run identity when recovering an incomplete
delivery; it must not run the model again merely because delivery evidence was
not yet observed.

When `consecutive_failures >= failure_limit`, the managed trigger transitions
to `Paused`. Re-enabling or explicitly reconciling repaired configuration
resets the counter and schedules the next future eligible slot. Operator-facing
errors expose a stable class and redacted reason, never raw document content,
provider credentials, or unfiltered provider errors.

## 10. Implementation and test seams

### #6570 — durable scheduling

Primary owners: `ironclaw_triggers` and Reborn composition.

Required proofs:

- config validation, exact scope identity, quiet-hour/DST classification;
- reconcile creates or updates one durable managed trigger and is idempotent;
- due tick submits exactly one trusted turn with automation/heartbeat origin;
- duplicate polls, disabled state, quiet hours, restart, and cross-user scope;
- no product adapter can mint trusted trigger ingress or heartbeat origin.

The deterministic caller-level test belongs in
`tests/integration/group_heartbeat/` and must exercise the real trigger poller,
trusted ingress, scheduler, runner, and scripted model seam.

### #6571 — result delivery and recovery

Primary owner: product-workflow triggered delivery, with trigger-owned durable
settlement metadata.

Required proofs:

- missing/empty document produces no turn and no outbound send;
- exact sentinel produces zero outbound sends; near-sentinel output delivers;
- actionable output creates exactly one verified outbound result;
- failed turns never emit success delivery;
- duplicate completion/restart recovery does not duplicate delivery;
- failure counter, capped backoff, disable threshold, reset, and user isolation;
- a scripted whole-turn scenario covers schedule to delivery without a live
  provider.

Unit tests alone are insufficient for any rule that gates submission or
delivery. PostgreSQL and libSQL parity is required for new durable fields.

## 11. Rollout and compatibility

- The feature is opt-in and disabled by default.
- Existing scheduled triggers and their storage/wire representation retain
  current behavior.
- Existing runner lease heartbeat settings are unrelated and unchanged.
- Rollback disables heartbeat reconciliation and removes the new composition
  wiring; ordinary trigger polling and delivery continue unchanged.
- #6570 must land before #6571. Neither implementation may relax the security,
  authorization, persistence, redaction, or delivery evidence contracts named
  above.
