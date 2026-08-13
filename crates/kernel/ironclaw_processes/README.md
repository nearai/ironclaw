# ironclaw_processes

**The single durable lifecycle authority** for every piece of host-tracked
work, foreground turn or background invocation alike: a row-native process
journal recording identity, lineage, and status, and the supervisor that
claims, leases, heartbeats, and recovers registered work. One journal
answering "what is this work doing right now" for every kind of work is the
entire reason this stage is a single authority rather than several.

Get the direction of truth right — it is the fact most often gotten wrong:
**the journal is the store; everything else is a projection over it.**
`ironclaw_turns`' turn/run state (`TurnRunState`) is a typed projection over
this journal, not a second durable store (PROPOSAL §6.5.8, landed by #6696);
`ProcessRecord`/`ProcessInvocationRecord` are lifecycle and invocation
projections produced from journal rows (`src/journal_store/state.rs`,
`src/invocation_state.rs`); and the turn-runner's await-edge store was
re-measured on 2026-08-05 as a pure projection over this crate's
`ProcessDependencyPort` — one `Arc<dyn ProcessDependencyPort>` field, zero
persistence of its own (PROPOSAL §12.13 D-S). Do not reintroduce a
domain-owned dependency store, recovery roster, or parallel run-state table.

- **Family / layer:** `kernel/` / `kernel` · **Package:** `ironclaw_processes`
  · **Manifest:** `crates/kernel/ironclaw_processes/Cargo.toml`
- **Use this when:** work must be durably submitted, claimed, heartbeat,
  suspended, recovered, cancelled, or queried; child/parent process
  relationships must be recorded; a new *kind* of durable work needs a
  registered executor.
- **Don't use this when:** you want to decide whether spawning is *allowed*
  (→ the membrane; this crate has no opinion about authorization) or how a
  lane executes (→ `ironclaw_host_runtime` / `crates/lanes/`).

## Public surface

- The journal: `ProcessJournalStore` (`src/journal_store.rs` +
  `src/journal_store/{command,rows,state,validation,observer,migration}`),
  `SubmitProcessRequest` (child submission opens its dependency edge
  atomically — `dependency: Option<ProcessDependencySubmission>`),
  checkpoint payloads as journal rows, immutable process input.
- `ProcessDependencyPort` (`src/journal.rs`) — open/settle/consume/abandon
  with idempotent replays and descendant-reservation release; dependency
  records carry loop-tier data as **opaque metadata** — the journal stays
  vocabulary-neutral, and the kernel never auto-settles or resumes dependents
  (measured, D-S).
- `ProcessSupervisor` (`src/supervisor.rs:219`) — claim/lease/heartbeat/
  recovery/panic containment/shutdown; process kinds registered by the crate
  owning the work (`ironclaw_turn_runner` → agent-turn,
  `ironclaw_host_runtime` → capability invocation).
- `ProcessExecutor` trait + `ProcessExecutionRequest`/`Result`, lifecycle
  types `ProcessRecord`/`ProcessStatus`/`ProcessStart`/`ProcessExit`
  (`src/types.rs`).
- `ProcessHost` (`src/host.rs:19`) + `ProcessSubscription`;
  `ProcessInvocationRecord`/`ProcessInvocationStatus`/
  `ProcessInvocationStatePort`/`ProcessInvocationStore`
  (`src/invocation_state.rs`); `ProcessResultStore` (`src/result_store.rs`);
  cancellation registry/token (`src/cancellation.rs`).

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_resources` (tree capacity
  reservation — the pinned same-layer edge that was once a
  `LAYER_MATRIX_EXCEPTION`, dissolved when this crate re-layered to kernel),
  `ironclaw_event_log`, `ironclaw_filesystem`, `ironclaw_host_api`.
- **Normal consumers (10 — the family's widest):** `ironclaw_turns`,
  `ironclaw_capabilities`, `ironclaw_host_runtime` (kernel, pinned edges),
  `ironclaw_assistant`, `ironclaw_composition`, `ironclaw_extension_host`,
  `ironclaw_extension_manager`, `ironclaw_loop_host`, `ironclaw_turn_runner`,
  `ironclaw_stress`.

## Invariants

- A terminal status is written once and never overwritten by a late
  completion (`src/journal_store/state.rs:603-661` terminal guards).
- Result store first, lifecycle terminal status second — observing a terminal
  process means its result is already available.
- Child creation + tree reservation + dependency open are **one** journal
  command; consume/abandon + reservation release are **one** journal command —
  never compensating dual writes.
- Request paths use exact reads and bounded, partition-leading keyset queries
  only; collection enumeration is confined to explicit offline migration —
  `reborn_process_storage_scan_gate.rs::process_and_thread_request_storage_paths_do_not_enumerate_collections`.
- Dependency boundary: the `BoundaryRule` for `ironclaw_processes` forbids
  `ironclaw_authorization`, `ironclaw_approvals`, `ironclaw_capabilities`,
  `ironclaw_host_runtime`, `ironclaw_secrets`, `ironclaw_network`, and the
  lanes — nothing above or beside reaches back down except through the ports
  this crate defines.

## Tests

```bash
cargo test -p ironclaw_processes
cargo test -p ironclaw_architecture_tests   # after dependency/API changes
# The projection claims are pinned at the integration tier:
#   tests/integration/subagent_await_edge.rs (runner_await_edge_is_a_projection_over_process_dependencies)
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules and guardrails.
- [`../AGENTS.md`](../AGENTS.md) — the kernel family.
- `docs/internal/reborn/contracts/processes.md`; PROPOSAL §6.5.7 and §12.13 D-S.
