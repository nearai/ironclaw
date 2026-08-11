# ironclaw_turns

The turn admission kernel: the durable entry point where a unit of
conversational work becomes admitted work — one active run per thread,
idempotent submission — and the exit-claim boundary where a loop's reported
outcome is validated against host-minted evidence before it becomes durable
truth. "A `LoopExit` is a claim, not truth" lives here. It is a separate crate
because "is this turn allowed to keep running" is a fail-closed authority
question distinct from "is this one capability call allowed" — different
callers, different blast radii.

Turn and run state are **not a second durable store**: they are a typed
projection over the process journal `ironclaw_processes` owns, so a turn's
lifecycle and its underlying process's lifecycle can never give two answers to
"is this still running" (`src/process_projection/`, `TurnRunState` at
`src/status.rs:174`).

- **Family / layer:** `kernel/` / `kernel` · **Package:** `ironclaw_turns` ·
  **Manifest:** `crates/kernel/ironclaw_turns/Cargo.toml`
- **Use this when:** submitting/resuming/cancelling turns, enforcing
  admission capacity, validating a loop exit, or projecting agent-turn state
  over the journal.
- **Don't use this when:** you need turn *vocabulary* (`TurnId`, `TurnScope`,
  `TurnStatus`, refs — `ironclaw_host_api::turn` owns all of it; depend there
  if vocabulary is all you need), loop-tier contracts (`AgentLoopDriver`,
  `Loop*Port`, the `LoopExit` DTOs — `ironclaw_loop_contracts`), or dispatch
  of any kind.

## Public surface

- `TurnCoordinator` / `DefaultTurnCoordinator` (`src/coordinator.rs`) —
  accept/resume/cancel under the one-active-run lock, keyed by canonical
  scoped thread `(tenant_id, agent_id, project_id?, thread_id)`; scoped
  idempotency keys on every mutating API; `SubmitTurnRequest` /
  `ResumeTurnRequest` / `CancelRunRequest` (`src/request.rs`,
  `src/response.rs`).
- Admission control: limits, buckets, capacity denials (`src/admission.rs`).
- Loop-exit validation (`src/loop_exit.rs`): the evidence port, the
  `LoopExitApplier`, validation policy, and violation taxonomy that turn a
  driver's claim into a durable transition.
- Kernel state/errors: `TurnRunState` (`src/status.rs:174`), `TurnError`,
  admission rejections, the `is_recoverability_critical` durability boundary.
- Agent-turn projection (`src/process_projection/`): `AgentTurnRuntimePort`
  implemented by `AgentTurnProcessRuntime` — a coordination/query projection,
  not a persistence authority; checkpoint vocabulary +
  `ProcessLoopCheckpointStore` projection (`src/checkpoint_state.rs`).
- Lifecycle events + projection service (`src/events.rs`).
- Resident by exception: `src/host_managed_ports/` (two `Loop*Port` impls
  awaiting the `loop_host` re-charter) and `src/external_tool_catalog.rs` (a
  pure passenger whose §6.5.8 shed destination was refuted by measurement —
  the dated record is in [`AGENTS.md`](./AGENTS.md); do not re-litigate
  without re-measuring).

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_processes` (the journal the state
  projects over — pinned same-layer edge), `ironclaw_loop_contracts` (the
  claim DTOs it validates), `ironclaw_filesystem`, `ironclaw_observability`,
  `ironclaw_host_api`.
- **Normal consumers (8):** `ironclaw_capabilities`, `ironclaw_host_runtime`
  (kernel, pinned), `ironclaw_assistant`, `ironclaw_composition`,
  `ironclaw_extension_host`, `ironclaw_loop_host`, `ironclaw_turn_runner`,
  `ironclaw_stress`.

## Invariants

- One active run per scoped thread; blocked/resumable runs keep the lock
  until resume, cancel, fail, or complete; running cancellation is two-phase
  (`CancelRequested` → trusted-runner terminal `Cancelled`, releasing the
  lock exactly once).
- A loop cannot talk itself into a durable transition it was not granted:
  exit claims are validated against host-minted evidence in `loop_exit`
  before any state change.
- Stores hold lifecycle metadata and references only — no raw prompts,
  assistant content, tool input, secrets, or host paths; `Failed` events may
  carry a secret-scrubbed, model-visible `detail` (values withheld, cause
  kept).
- Dependency boundary: the `BoundaryRule` for `ironclaw_turns` forbids
  `ironclaw_approvals`, `ironclaw_authorization`, `ironclaw_capabilities`,
  `ironclaw_host_runtime`, `ironclaw_secrets`, `ironclaw_network`,
  `ironclaw_hooks`, `ironclaw_memory`, and the lanes — admission and exit
  validation never touch execution.

## Tests

```bash
cargo test -p ironclaw_turns
cargo test -p ironclaw_architecture_tests   # after dependency/API changes
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules, the vocabulary-ownership map
  (what is *not* owned here), guardrails.
- [`../AGENTS.md`](../AGENTS.md) — the kernel family.
- `docs/reborn/contracts/turns-agent-loop.md`,
  `docs/reborn/contracts/loop-exit.md`.
