# Capability Invocation Edge Writes

**Issue:** [#7598](https://github.com/nearai/ironclaw/issues/7598)

## Goal

Reduce each fresh, non-gated capability invocation from three process-journal commands to one without weakening gate resume, terminal side-effect evidence, lease fencing, or fail-closed crash recovery.

## Current behavior

`CapabilityHost::authorize` starts a `CapabilityInvocationState` process before policy checks. `ProcessInvocationStore::start` submits and claims that process, and the capability path later suspends, completes, or fails it. A normal non-gated invocation therefore writes queued, claimed/running, and terminal lifecycle state even though no production reader needs the intermediate durable record for recovery.

Durable invocation-state readers are limited to gate resume and resume preflight. Approval resume, auth resume, and spawn resume load a blocked record and validate its capability, actor, gate identifier, and status before claiming a lease or dispatching. Host runtime status and cancellation also list running invocation records, but they only report active inline work as visible or unsupported; they do not recover or dispatch it.

The parent turn writes a durable `BeforeSideEffect` checkpoint before capability dispatch. That checkpoint is the crash recovery boundary: lease recovery must fail closed rather than redispatch after a possible side effect. Invocation terminal state is separate durable audit evidence and remains mandatory.

## Decision

Buffer fresh invocation state inside `ProcessInvocationStore` and materialize it durably only at the first gate or terminal edge.

The process journal gains one atomic edge-submission command. Its request contains the same immutable process identity, scope, owner, process kind, metadata, and timestamps as normal submission plus one allowed initial edge:

- suspended for approval or authentication, including the existing suspension and checkpoint reference;
- completed; or
- failed, including sanitized failure evidence.

Applying this command creates the authoritative process snapshot directly in the requested edge state and emits one matching lifecycle journal entry. It does not synthesize queued, claimed, resumed, or running entries. Existing process identity, scope, replay, and active-process validation still apply.

`ProcessInvocationStore::start` validates and inserts a `Running` record into a worker-local pending map. It performs no durable I/O. The map is keyed by invocation identity and retains the complete `ProcessInvocationStart`, not a second partial DTO.

`block_approval`, `block_auth`, `complete`, and `fail` first inspect the pending map:

- For a pending record, they use the atomic edge-submission command. They remove the pending entry only after the durable command succeeds.
- For an already durable record, they use the current lease-fenced transition path. This preserves cross-worker resume behavior and prevents two workers from dispatching the same resumed invocation.

`get` and `records_for_scope` merge worker-local pending records with durable records, preferring the durable record for the same invocation. Same-worker runtime status and cancellation therefore retain their current visibility. Another worker cannot see a fresh running invocation before its first edge. This is intentional: fresh inline invocations have no redispatch path, and their parent turn checkpoint remains the recovery authority.

## Gate and resume flow

1. Worker A begins an invocation in local memory.
2. Authorization requires approval or authentication.
3. Worker A atomically writes a suspended invocation snapshot before returning the blocked outcome.
4. Worker B loads the suspended snapshot from the shared journal, validates the resume context, resumes and claims it with the existing lease path, and dispatches.
5. Worker B completes or fails it through the existing durable transition.

No gate outcome is returned before the suspended state is durable. Gate identifiers, approval identifiers, actor binding, error kind, and checkpoint references remain unchanged.

## Failure behavior

A durable edge write that fails leaves the pending entry available for the current worker to inspect or retry.

Pre-dispatch business errors keep their current precedence. If recording their failed or blocked state also fails, the original capability error is returned and the journal failure is logged.

After successful dispatch, terminal completion is still attempted. If that write fails, the capability result remains successful and the failure is logged; reporting the external side effect as failed would invite an unsafe retry.

A worker crash before the first edge loses only local invocation bookkeeping. A worker crash after `BeforeSideEffect` but before terminal invocation persistence leaves the parent checkpoint as durable evidence of a possibly attempted side effect; lease recovery fails closed. There is no half-run invocation redispatch path and this change does not add one.

## Reader impact

- Approval, authentication, and spawn resume continue to read only durable suspended records.
- Resume preflight continues to fail only a matching durable blocked record.
- Host runtime status and cancellation see pending invocations only on the worker that owns them. Cross-worker visibility of fresh running inline calls is deliberately removed; those records were never recoverable work.
- Process projections and journal observers receive only the edge entry for fresh invocations. Consumers must not infer that every terminal or suspended invocation has preceding queued and claimed entries.
- Terminal and suspended records remain available through `get` and scope listing after worker restart.

## Persistence compatibility and rollback

The stored command enum gains a new tagged variant. New binaries continue to read all existing journal data. An older binary cannot deserialize a journal containing the new variant and therefore fails closed rather than silently reconstructing the record as queued.

Rollback after the first edge-submission write requires a binary that retains the new command reader. The implementation must not hide the initial edge in metadata on the existing submit command because an older binary would ignore the metadata and silently materialize an unsafe queued process.

No database schema migration or row deletion is required.

## Test strategy

Tests are added before production changes.

1. Extend the process invocation store contract to assert that pending `Running` state is locally readable, a fresh completion persists one `Completed` journal entry, and a reconstructed store reads the terminal record.
2. Add command-level transition tests for direct suspended, completed, and failed materialization, replay behavior, scope isolation, and rejection of invalid initial states.
3. Extend the capability-host invocation-state contract so a non-gated dispatch leaves one terminal invocation journal entry and no queued/claimed entries.
4. Extend the existing approval/auth resume contracts with two `ProcessInvocationStore` instances sharing one journal: worker A blocks, worker B loads and resumes, and the terminal result remains durable.
5. Extend the existing turn-run lease recovery coverage to simulate worker loss after `BeforeSideEffect`; assert that recovery fails closed, does not redispatch the capability, and retains the checkpoint as possible-side-effect evidence.
6. Extend host-runtime status coverage to assert same-worker pending visibility and explicitly assert that a separate worker does not report pending state as recoverable durable work.

Run the narrow process, capability, host-runtime, turn-runner, and integration tests that own these contracts. Run `cargo test -p ironclaw_architecture_tests` because the process journal contract changes. Run clippy for every changed crate with warnings denied.

## Non-goals

- Removing terminal invocation records.
- Adding tool-idempotency persistence or redispatching half-run capability calls.
- Collapsing lease-fenced transitions after a durable gate resume.
- Changing approval, authentication, obligation, resource accounting, transcript, or event-log persistence.
- Making inline capability cancellation cross-worker capable.
- Deleting historical LLM or process data.
