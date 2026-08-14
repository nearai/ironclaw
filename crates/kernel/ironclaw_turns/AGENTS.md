# Agent Map — ironclaw_turns

## Start Here

- Read `README.md` for orientation (charter, measured deps/consumers, gates).
  This file is the canonical working-rules home; `CLAUDE.md` is a pointer here.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/internal/reborn/contracts/turns-agent-loop.md`
- `docs/internal/reborn/contracts/turn-persistence.md`
- `docs/internal/reborn/contracts/turn-runner.md`
- `docs/internal/reborn/contracts/loop-exit.md`

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
    (`crates/loop/ironclaw_loop_host/src/external_tool_capability.rs`, which
    imports `ExternalToolCatalog`,
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

## Guardrails

- Own host-layer turn coordination contracts only: adapter-safe coordinator
  APIs, admission, runner transition ports, store traits, and redacted
  lifecycle events.
- Do **not** own the turn vocabulary. Scope/actor, turn+run+checkpoint+lease
  ids, the bounded refs, `TurnStatus` and its `GateKind`/`BlockedReason`
  correspondence, `EventCursor`, and `RunOriginAdapter` live in
  `ironclaw_host_api::turn`. The prelude re-export in `lib.rs` exists so
  crates that already depend on this one keep a single import; it is not an
  ownership claim, and a crate that needs *only* vocabulary must depend on
  `ironclaw_host_api` directly instead of taking a dependency here. Never
  re-alias a host_api turn type to a second name (the deleted `ids.rs` did
  exactly that with `GateRef`, colliding with the unrelated
  `ironclaw_host_api::ids::GateRef`).
- Stay above the Reborn kernel service. Do not depend on or re-export raw
  `CapabilityHost`, dispatcher, process host, runtime-lane adapters, raw
  filesystem, network, secrets, MCP, script, or WASM handles.
- Product adapters use `TurnCoordinator` methods only. Trusted workers may
  import `ironclaw_turns::runner` explicitly; do not add runner transition
  APIs to the public prelude.
- Mutating adapter-facing APIs must take scoped idempotency keys.
  `submit_turn` accepts requested run-profile hints and `received_at`;
  responses/state expose resolved profile id+version, not lower runtime
  handles.
- Consume canonical binding/session refs from upstream services. Do not parse
  Slack/Telegram/Web/CLI identity, channel conversation IDs, or raw
  transcript content in this crate.
- Active-run exclusivity is keyed by canonical scoped thread
  `(tenant_id, agent_id, project_id?, thread_id)` and must not include
  channel IDs or user IDs.
- Blocked/resumable runs keep the same-thread active lock until resume,
  cancel, fail, or complete. Running cancellation is two-phase: public cancel
  requests move to `CancelRequested`, and a trusted runner cancellation
  completion moves to terminal `Cancelled` and releases the lock exactly
  once.
- Store lifecycle metadata and references only. Do not persist raw prompts,
  assistant content, tool input, secrets, or host paths in turn state or
  events. Failure events MAY carry a secret-scrubbed, model-visible `detail`
  (`TurnLifecycleEvent.detail`, only on `Failed`) describing the real cause
  so the model/explainer can retry or explain — only secret *values* are
  withheld (scrubbed by the value-level redactors), not the descriptive
  cause. Raw, unscrubbed backend error strings still stay behind the host
  adapters.
- Keep concrete PostgreSQL/libSQL adapters and product projection/egress
  wiring out of the core contract unless a scoped follow-up explicitly adds
  them with parity tests.
- **Model-call idle boundary:** `host_managed_ports/model.rs` wraps the
  primary model call with `PRIMARY_MODEL_CALL_IDLE_TIMEOUT` (75 s). Each
  model text update resets this watchdog, so healthy long streams can exceed
  75 seconds while a stalled gateway still fails before the 90-second runner
  lease can reclaim the run. An elapsed idle timeout maps to retryable
  `AgentLoopHostErrorKind::Unavailable`; provider-specific semantic
  continuation must not manufacture a successful response from partial
  output.
- **Loop-framework contracts are not ours either.** `LoopFailureKind`,
  `AgentLoopDriver`, `AgentLoopDriverHost`, every `Loop*Port`, the
  run-profile descriptors and refs, prompt-bundle and checkpoint contracts,
  progress events, cancellation signals, and the `LoopExit` claim DTOs live
  in `ironclaw_loop_contracts` (WS1.2). This crate depends on that crate; the
  dependency never runs the other way. What stays here is the *authority*
  half: admission, the coordinator, the state projection, and `LoopExit`
  **validation** — the exit applier, the validation policy, and the violation
  taxonomy that turn a driver's claim into a durable transition.
- Implementations of those contracts live elsewhere: host adapters in
  `ironclaw_loop_host`, driver-side integration in `ironclaw_turn_runner`,
  and reusable loop mechanics in `ironclaw_agent_loop`. Two are still
  resident here under `host_managed_ports/` — `HostManagedLoopModelPort` and
  `HostManagedLoopPromptPort` — because PROPOSAL §6.1.4 forbids a contracts
  crate from implementing its own ports and §6.7.2 assigns them to
  `ironclaw_loop_host`. They move with the WS4 `loop_host` re-charter.
  Nothing new belongs in that module.
- Add a new `.rs` file before widening an existing contract file with an
  unrelated responsibility. Do not create broad `common`, `misc`, or
  `helpers` modules.

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
