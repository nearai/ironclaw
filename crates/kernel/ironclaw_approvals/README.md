# ironclaw_approvals

The consent stage of the kernel pipeline: resolves a require-approval verdict
into a scoped, fingerprinted capability lease or a durable denial — the crate
that turns a human or policy decision into something the membrane can act on.
It also owns the durable approval-request and model-visible gate records
(absorbed from the retired `ironclaw_run_state` by #6696) and the
persistent-approval / auto-approve / permission-override policy stores. It is
a separate crate because consent resolution has its own durability and
ordering guarantees; folding it into authorization would blur "does this grant
apply" with "did a human agree to this".

- **Family / layer:** `kernel/` / `kernel` · **Package:** `ironclaw_approvals`
  · **Manifest:** `crates/kernel/ironclaw_approvals/Cargo.toml`
- **Use this when:** a pending approval must become a lease or a denial; you
  need approval-request / gate-record persistence or scope-bounded
  "always allow" policy.
- **Don't use this when:** you want grant matching or lease storage (→
  `ironclaw_authorization`), user prompting or notification delivery (→
  product tier, which calls into this crate), or dispatch (→
  `ironclaw_capabilities`).

## Public surface

- `ApprovalResolver` (`src/lib.rs:86`) — the fail-closed resolver — and
  `ApprovalResolutionError`; outcomes `LeaseApproval` (scoped lease issued)
  and `DenyApproval` (no lease).
- Durable stores: `ApprovalRequestStore` and `GateRecordStore`
  (`src/approval_store.rs`), filesystem-backed, with their contract suites.
- Policy stores: auto-approve (`src/auto_approve.rs`), capability permission
  overrides (`src/capability_permission.rs` —
  `CapabilityPermissionOverrideStorePort`; the blanket-impl
  `ToolPermissionOverrideStorePort` alias was deleted, WS8 2026-08-05).
- The deployment-profile approval gate (`src/profile_gate.rs`,
  `src/profile_gate_policy.rs`) — evicted from the composition root in WS6;
  the reason this crate names `ironclaw_runtime_policy`
  (`MinimalApprovalBypass`) and `ironclaw_trust` (`TrustDecision` in the
  `TrustAwareCapabilityDispatchAuthorizer` signature it implements).

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_authorization` (the lease it issues
  into), `ironclaw_trust`, `ironclaw_runtime_policy` (all three pinned in
  `reborn_same_layer_edge_inventory.rs` with their deciding workstreams),
  `ironclaw_event_log` (metadata-only audit), `ironclaw_filesystem` (stores),
  `ironclaw_host_api`.
- **Normal consumers (8):** `ironclaw_capabilities`, `ironclaw_host_runtime`
  (kernel), `ironclaw_assistant`, `ironclaw_composition`,
  `ironclaw_extension_host`, `ironclaw_extension_manager`,
  `ironclaw_loop_host`, `ironclaw_turn_runner`.

## Invariants

- Fail-closed ordering: the `approve` authority record is persisted **before**
  the lease is issued (`src/lib.rs:240`). If the lease store fails after the
  approval is persisted, the request stays `Approved` and the caller surfaces
  the lease error — no rollback to `Pending`; re-issuance against an
  already-decided request is recoverable.
- A denial is durable and issues no lease; a caller raises a new request
  rather than retrying a denied one.
- Audit emission is metadata-only and best-effort — it never alters a
  resolution outcome.
- Which capabilities may *bypass* this stage is frozen data:
  `reborn_origin_gate_matrix_ratchet.rs` pins the reviewed ungated seed and
  requires a well-formed `origin_gate_matrix` on every declared capability.
- Dependency boundary: the `BoundaryRule` for `ironclaw_approvals` in
  `reborn_dependency_boundaries.rs` forbids `ironclaw_capabilities`,
  `ironclaw_host_runtime`, `ironclaw_processes`, `ironclaw_resources`,
  `ironclaw_secrets`, `ironclaw_network`, and the lane crates — the membrane
  depends on this crate, never the reverse.

## Tests

```bash
cargo test -p ironclaw_approvals
cargo test -p ironclaw_architecture_tests   # after dependency/API changes
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules and guardrails.
- [`../AGENTS.md`](../AGENTS.md) — the kernel family; the lease mint's charter
  seal (the issuing port is public — the restriction is boundary rules, not a
  type seal).
- `docs/internal/reborn/contracts/approvals.md`, `docs/internal/reborn/contracts/run-state.md`.
