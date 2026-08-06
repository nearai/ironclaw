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

- Host-layer turn coordination contracts (above the Reborn kernel service), currently:
- Adapter-facing coordinator: `TurnCoordinator`/`DefaultTurnCoordinator`, `TurnAdmissionPolicy`, run-wake notifier ports (`coordinator`); request/response surface `SubmitTurnRequest`/`ResumeTurnRequest`/`CancelRunRequest`/`GetRunStateRequest` (`request`) and `SubmitTurnResponse`/`ResumeTurnResponse`/`CancelRunResponse`/`ThreadBusy` (`response`).
- Runner-facing claim and outcome projection types (`runner`); lifecycle
  mutation ports are owned by `ironclaw_processes`.
- **Not** the turn vocabulary. `TurnId`/`TurnRunId`/`TurnRunnerId`/`RunProfileId`/`RunProfileVersion`/`IdempotencyKey`/`TurnLeaseToken`, the gate/message/result/binding refs, `TurnScope`/`TurnActor`, `TurnStatus`+`GateKind`+`BlockedReason`, `EventCursor`, and `RunOriginAdapter` are all owned by `ironclaw_host_api::turn`. This crate re-exports them from its prelude for its own consumers' convenience — adding a *new* turn type means adding it there, not here.
- Admission control: limits, buckets, capacity denials, providers (`admission`).
- **Not** the loop-tier contract. `AgentLoopDriver` + descriptors/run/resume requests, run-profile resolution/registry/resolver, prompt/context/model/capability profile ids, resource-budget tiers, scheduling/concurrency classes, redacted provenance, every `Loop*Port`, and the `LoopExit`/`LoopCompleted`/`LoopFailed`/`LoopBlocked`/`LoopCancelled` claim DTOs are owned by `ironclaw_loop_contracts` (WS1.2). This crate depends on it and never re-exports it.
- Loop-exit **validation**: the evidence port, the applier, the validation policy, the mapping, and the violation taxonomy that turn a driver's claim into a durable transition (`loop_exit`). "A `LoopExit` is a claim, not truth" is enforced here.
- Two resident host-port implementations (`host_managed_ports`): `HostManagedLoopModelPort` and `HostManagedLoopPromptPort`, awaiting the WS4 `loop_host` re-charter. Nothing new belongs there.
- Lifecycle events + projection: `TurnLifecycleEvent`, `TurnEventKind`, `TurnEventSink`, projection service/cursor/source (`events`).
- Kernel state and error surface: `TurnRunState`, `TurnError`/`TurnErrorCategory`/`TurnCapacityResource`, `TurnRunProfile`, admission rejections, and the `is_recoverability_critical` write-behind durability boundary (`status`). The `TurnStatus` values those types carry come from `ironclaw_host_api::turn`.
- Agent-turn projection over process submission, journal, control, and tree
  ports (`process_projection`); `AgentTurnRuntimePort` is a coordination/query
  projection implemented by `AgentTurnProcessRuntime`, not a persistence
  authority.
- Agent-loop checkpoint vocabulary and the `ProcessLoopCheckpointStore`
  projection; metadata and bounded opaque payload persistence are owned by
  `ironclaw_processes`.
- Crate-local public API, tests, and fixtures needed to prove that ownership.
- `external_tool_catalog` — the per-run catalog of client-supplied ("external")
  tools. PROPOSAL §6.5.8 lists it as a shed, *"→ product, its self-described
  owner"*; **that destination is refuted, measured 2026-08-05 (WS5 `product`
  narrows), and the module stays here for now.** Recorded so the next agent does
  not re-litigate it:
  - **`ironclaw_assistant` is not a consumer — it names zero of the six exported
    symbols.** The production readers are exactly two: `ironclaw_loop_host`
    (`src/external_tool_capability.rs`, which imports `ExternalToolCatalog`,
    `PendingExternalCall`, `ExternalToolSpec` and `ExternalToolCatalogError`
    outside any `cfg(test)`) and `ironclaw_composition`, which wires it. So the
    move would place the module where nothing reads it.
  - **And `loops → products` is matrix-illegal.** `ironclaw_loop_host` is
    `layer = "loops"`, `ironclaw_assistant` is `layer = "products"`, and
    `layer_allows_dependency` admits nothing above `loops` for a `loops` crate —
    the move needs a `LAYER_MATRIX_EXCEPTION`, which the WS0 ratchet forbids.
    Same refutation shape as `ironclaw_common`'s `provider_transcript`.
  - **`ironclaw_openai_compat` is not the product-side consumer the §6.5.8
    sentence implies.** It declares its own `OpenAiCompatExternalToolSpec` /
    `OpenAiCompatExternalToolStore` port and names `ironclaw_turns` only as a
    *dev*-dependency; composition adapts between the two. Its `BoundaryRule`
    also forbids `ironclaw_loop_host` outright, so "put it beside its reader"
    is closed off in that direction too.
  - **The shed itself is still sound**: nothing in `ironclaw_turns` uses the
    module — it is a pure passenger, re-exported and never called from a product-tier crate (loop_host and composition are its production readers). A legal home
    must sit at or below `loops` and be reachable from `products` and `app`;
    `ironclaw_loop_contracts` is the candidate, and choosing it is a design call
    (it would put an in-memory store impl in a contracts crate), not this row's
    mechanical move.

## Do Not Move In Here

- raw CapabilityHost/dispatcher/runtime handles, raw prompts/content/tool inputs/secrets/host paths, or channel identity parsing.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_turns`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
