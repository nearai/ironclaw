# ironclaw_capabilities

The membrane: the single caller-facing invocation service every privileged
effect must cross. Six workflows — invoke, resume, resume-after-auth,
decline-auth, resume-spawn, spawn — each run the same authorization fold
(trust ceiling → authorization → approval → obligations → reservation) before
any side effect, and the fold's output is the sealed `Authorized` witness only
this crate can mint. One crate, because the sealing invariant is enforced by a
dedicated boundary test, and one crate gives that test exactly one thing to
check.

- **Family / layer:** `kernel/` / `kernel` · **Package:**
  `ironclaw_capabilities` · **Manifest:**
  `crates/kernel/ironclaw_capabilities/Cargo.toml`
- **Use this when:** any caller — loop, extension, product surface — needs a
  privileged effect invoked, resumed, or spawned. There is no other door.
- **Don't use this when:** you want mediated execution mechanics (egress,
  secret staging, lane adapters → `ironclaw_host_runtime`), process lifecycle
  or results (→ `ironclaw_processes::ProcessHost`), or approval resolution (→
  `ironclaw_approvals`).

## Public surface

- `CapabilityHost` — the workflows live in `src/host/`, one private module per
  workflow (`invoke`, `approval_resume`, `auth_resume`, `spawn_resume`,
  `spawn`), with `authorize` owning the single fold they all funnel through;
  the charter table in `src/host/mod.rs` decides file placement. Every
  workflow is an inherent method — callers see exactly one path.
- The seal: `impl CapabilityAuthorizer for CapabilityHost`
  (`src/host/mod.rs:107`) — the sole implementor workspace-wide of the grant
  trait that can construct `AuthorizationGrant(())` and therefore seal an
  `Authorized` (`ironclaw_host_api/src/authorized.rs:60,:74,:101`).
- `RuntimeDispatcher` (`src/dispatch.rs:130`) — the sole production
  `CapabilityDispatcher` impl; routes a sealed witness to its bound lane and
  rejects any witness/binding lane mismatch.
- The obligation seam (`src/obligations.rs`): `CapabilityObligationHandler`,
  request/outcome/phase/failure types — implemented by `ironclaw_host_runtime`.
- `ReplayPayloadStore` (`src/replay_payload.rs`) — host-private raw replay
  payload for gate/auth resume, keyed by `InvocationId`, behind a
  `ScopedFilesystem` CAS lane; never model-visible (see `AGENTS.md`).

## Depends on / consumed by

- **Normal deps (measured, 14):** the five stage crates (`ironclaw_trust`,
  `ironclaw_authorization`, `ironclaw_approvals`, `ironclaw_resources`,
  `ironclaw_runtime_policy`) and `ironclaw_processes` + `ironclaw_turns` —
  all seven pinned in `reborn_same_layer_edge_inventory.rs` — plus
  `ironclaw_host_api`, `ironclaw_loop_contracts`, `ironclaw_event_log`,
  `ironclaw_filesystem`, `ironclaw_safety`, `ironclaw_extension_contracts`,
  `ironclaw_extension_registry` (descriptor/registry vocabulary in
  `src/registry.rs` and `src/trust.rs`).
  - The `ironclaw_turns` edge exists for the replay-payload store's field
    types (`CapabilityInputRef`, `AuthResumeApprovalIdentity`) — vocabulary,
    not the coordinator service. Note: `families/kernel.md` says this crate
    never depends on "the turn coordinator"; the crate edge is real and
    sanctioned — read that sentence as "never calls turn admission".
- **Normal consumers (4):** `ironclaw_host_runtime` (constructs and drives
  the membrane — the single construction site), `ironclaw_loop_host` (the
  loop-side caller), `ironclaw_extension_host`, `ironclaw_composition`.

## Invariants

- Only this crate mints the witness:
  `reborn_authorized_seal_ratchet.rs::capability_authorizer_is_implemented_only_by_the_kernel`
  (with self-tests that an evasion or unreadable source fails the scan rather
  than passing it).
- Authorization denial or an unsupported/failed obligation fails **before**
  runtime dispatch, process start, or approval-lease claim; approval resume
  validates and claims the matching fingerprinted lease before dispatch.
- Production wiring uses the trust-aware contract
  (`TrustAwareCapabilityDispatchAuthorizer`) — grant-only authorization that
  bypasses trust ceilings is forbidden (see `AGENTS.md`).
- Dependency boundary: the `BoundaryRule` for `ironclaw_capabilities` forbids
  `ironclaw_host_runtime`, `ironclaw_secrets`, `ironclaw_network`, and every
  lane crate — the membrane never reaches into mediated execution; the
  direction is strictly `host_runtime → capabilities`.

## Tests

```bash
cargo test -p ironclaw_capabilities
cargo test -p ironclaw_architecture_tests   # after dependency/API changes
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules, the `src/host/` module charter,
  guardrails.
- [`../AGENTS.md`](../AGENTS.md) — the kernel family; the witness in the
  sealed-mint table.
- `docs/internal/reborn/contracts/capability-access.md`,
  `docs/internal/reborn/contracts/capabilities.md`.
