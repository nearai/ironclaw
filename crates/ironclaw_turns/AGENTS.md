# Agent Map — ironclaw_turns

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/turns-agent-loop.md`
- `docs/reborn/contracts/turn-persistence.md`
- `docs/reborn/contracts/turn-runner.md`
- `docs/reborn/contracts/loop-exit.md`

## What This Crate Owns

- Host-layer turn coordination contracts (above the Reborn kernel facade), currently:
- Adapter-facing coordinator: `TurnCoordinator`/`DefaultTurnCoordinator`, `TurnAdmissionPolicy`, run-wake notifier ports (`coordinator`); request/response surface `SubmitTurnRequest`/`ResumeTurnRequest`/`CancelRunRequest`/`GetRunStateRequest` (`request`) and `SubmitTurnResponse`/`ResumeTurnResponse`/`CancelRunResponse`/`ThreadBusy` (`response`).
- Trusted runner transition ports (`runner`, kept out of the adapter prelude).
- Canonical typed IDs and references: `TurnId`, `TurnRunId`, `TurnRunnerId`, `RunProfileId`/`RunProfileVersion`, `IdempotencyKey`, `TurnLeaseToken`, gate/message/result/binding refs (`ids`); turn scope/actor (`scope`).
- Admission control: limits, buckets, capacity denials, providers (`admission`).
- Run-profile contracts: `AgentLoopDriver` + descriptors/run/resume requests, run-profile resolution/registry/resolver, prompt/context/model/capability profile ids, resource-budget tiers, scheduling/concurrency classes, redacted provenance (`run_profile`, which has its own `CLAUDE.md`).
- Loop-exit protocol: `LoopExit`/`LoopCompleted`/`LoopFailed`/`LoopBlocked`/`LoopCancelled`, evidence ports, applier, mapping, validation (`loop_exit`).
- Lifecycle events + projection: `TurnLifecycleEvent`, `TurnEventKind`, `TurnEventSink`, projection service/cursor/source (`events`).
- Status/error vocabulary: `TurnStatus`/`TurnRunState`/`TurnError`/`TurnErrorCategory`, admission rejections, sanitized failure/cancel reasons (`status`).
- Turn/checkpoint state stores: `TurnStateStore` + records (turn/run/checkpoint/idempotency/active-lock, persistence snapshot) (`store`); checkpoint + loop-checkpoint state stores (`checkpoint_state`); in-memory (`memory`) and filesystem (`filesystem_store`) backends.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- raw CapabilityHost/dispatcher/runtime handles, raw prompts/content/tool inputs/secrets/host paths, or channel identity parsing.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_turns`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
