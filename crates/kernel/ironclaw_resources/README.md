# ironclaw_resources

The reservation stage of the kernel pipeline: cost, quota, and scarce runtime
capacity governed through a reserve → execute → reconcile-or-release protocol,
reserved before work starts and reconciled after it completes. It is its own
crate because it is the kernel's only stage with multiple production
implementations behind one protocol (`ResourceGovernor` ×3), and its
platform-specific concerns must never leak into any other kernel crate's
build.

- **Family / layer:** `kernel/` / `kernel` · **Package:** `ironclaw_resources`
  · **Manifest:** `crates/kernel/ironclaw_resources/Cargo.toml`
- **Use this when:** costed or quota-limited work needs capacity decided,
  accounted, or reconciled — any budget dimension, any scope.
- **Don't use this when:** you want to execute the work (→
  `ironclaw_capabilities` / `ironclaw_host_runtime`) or decide whether it is
  authorized at all (→ `ironclaw_authorization`). Lanes never name this crate
  in production — they consume the narrow `RuntimeResourceBudget` port from
  `ironclaw_host_api::resource` (#7067).

## Public surface

- `ResourceGovernor` trait (`src/lib.rs`) with three impls:
  `InMemoryResourceGovernor`, `PersistentResourceGovernor`, and
  `FilesystemResourceGovernor` (`src/filesystem_governor.rs`, the one
  host-runtime wires); `ResourceGovernorStorePort` with
  `JsonFileResourceGovernorStore` / `ResourceGovernorStore`
  (`src/resource_store.rs`). Re-derive:
  `rg -n "pub (trait|struct) \w*Governor\w*" src/`.
- `GovernorRuntimeBudget` (`src/lib.rs:46`) — implements host_api's
  `RuntimeResourceBudget` (reserve/reconcile/release) over any governor; owns
  the subtractive `ResourceError` projection so lane callers get
  classification without the kernel's account/limit values.
- Vocabulary: `ResourceDimension` (`Usd`, `InputTokens`, `OutputTokens`,
  `WallClockMs`, `OutputBytes`, `NetworkEgressBytes`, `ProcessCount`,
  `ConcurrencySlots`), `ResourceAccount` (tenant/user/project/agent/mission/
  thread), `ResourceLimits`, `ResourceValue`, `ResourceTally`,
  `ResourceDenial`, `ResourceError`, `ResourceGovernorSnapshot`;
  `ResourceReservation` / `ResourceReceipt` re-exported from
  `ironclaw_host_api`.
- The `BudgetApprovalGate` pause-threshold machine — deliberately a distinct
  state machine from capability approval (unify only with an ADR, PROPOSAL
  §6.5.4).

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_host_api`, `ironclaw_filesystem` —
  the one edge that could surprise: the governor snapshot persists through
  `ScopedFilesystem`, sanctioned in the crate's `BoundaryRule` comment
  (`reborn_dependency_boundaries.rs`, `ironclaw_resources` entry).
- **Normal consumers (8):** `ironclaw_capabilities`, `ironclaw_host_runtime`,
  `ironclaw_processes` (kernel — all pinned same-layer edges),
  `ironclaw_composition`, `ironclaw_extension_host`,
  `ironclaw_extension_manager`, `ironclaw_loop_host`, `ironclaw_stress`.
  Lanes (`ironclaw_mcp`, `ironclaw_sandbox`, `ironclaw_wasm`) hold it
  **dev-only**, driving the host_api port over the real governor.

## Invariants

- No costed or quota-limited work executes without an active reservation; a
  reservation failure — storage failure included — is a denial, never a
  reason to proceed and true up later. Pinned by
  `tests/resource_governor_contract.rs`:
  `reserve_denies_when_usd_limit_would_be_exceeded`,
  `filesystem_resource_governor_fails_closed_then_recovers_after_delta_append_error`,
  `filesystem_resource_governor_store_fails_closed_on_byte_only_backend`,
  `filesystem_budget_gate_store_fails_closed_on_byte_only_backend`.
- Every reservation and receipt preserves tenant/user/project scope
  (`project_limit_denies_leaf_even_when_tenant_allows`).
- Dependency boundary: the `BoundaryRule` for `ironclaw_resources` forbids
  every other kernel stage and every lane — this crate is called by the
  membrane, lifecycle, and mediated execution, and never reaches into them.

## Tests

```bash
cargo test -p ironclaw_resources
cargo test -p ironclaw_architecture_tests   # after dependency/API changes
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules and guardrails.
- [`../AGENTS.md`](../AGENTS.md) — the kernel family.
- `docs/internal/reborn/contracts/resources.md`.
