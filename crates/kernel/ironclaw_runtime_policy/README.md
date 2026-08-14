# ironclaw_runtime_policy

The policy-planning stage of the kernel pipeline: pure, deterministic
resolution of `(DeploymentMode, RuntimeProfile, OrgPolicyConstraints)` into the
`EffectiveRuntimePolicy` every dispatch enforces, plus per-capability lane
planning. It is the kernel's only dependency-free stage — zero I/O, policy
math over `ironclaw_host_api` types — and keeping it a leaf means policy
resolution stays testable without the membrane's full service cone.

- **Family / layer:** `kernel/` / `kernel` · **Package:**
  `ironclaw_runtime_policy` · **Manifest:**
  `crates/kernel/ironclaw_runtime_policy/Cargo.toml`
- **Use this when:** deployment posture must resolve to backend kinds and
  enforcement posture, or a capability needs its execution lane planned.
- **Don't use this when:** you want per-invocation authorization (→
  `ironclaw_authorization` via the membrane), process startup or dispatch (→
  `ironclaw_host_runtime`), or approvals. The resolver picks lanes and modes;
  it never runs anything.

## Public surface

- `resolve(ResolveRequest) -> Result<EffectiveRuntimePolicy, ResolveError>`
  (`src/resolver.rs:129`) — the **only sanctioned producer** of
  `EffectiveRuntimePolicy`; a value constructed any other way is untrusted by
  contract. `EffectiveRuntimePolicy::was_reduced()` flags narrowing so audit
  can render "you asked for X, you got Y".
- `plan_capability` → `ExecutionPlan` / `PlannerError`
  (`src/planner.rs:123`) — per-capability backend/lane selection from a
  resolved policy.
- The policy *types* live in `ironclaw_host_api::runtime_policy`; this crate
  owns the math, not the vocabulary.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_host_api` only (plus `serde`/
  `thiserror`). Reproduce: `cargo metadata --no-deps` for this package.
- **Normal consumers (4):** `ironclaw_approvals` (the WS6 profile gate
  consumes `MinimalApprovalBypass`), `ironclaw_capabilities` (planning folded
  into `authorize()`'s reach), `ironclaw_host_runtime`,
  `ironclaw_composition`.

## Invariants

- **Monotone with respect to safety:** deployment mode and tenant/org policy
  may only *reduce* requested authority, never increase it.
- **Fail-closed:** invalid `(deployment, profile)` pairs are a `ResolveError`,
  not a silent downgrade (`resolver.rs:129`); `*Yolo*` profiles require the
  caller-supplied `yolo_disclosure_acknowledged` (`resolver.rs:61`), and
  `EnterpriseYoloDedicated` additionally requires
  `admin_approves_dedicated_yolo` (`resolver.rs:26`); a capability needing
  process effects against `ProcessBackendKind::None` is a `PlannerError`
  (`planner.rs:140`) — the planner half of the "no sandbox ⇒ no shell, never
  host shell" rule (`.claude/rules/safety-and-sandbox.md`).
- Deterministic and serializable: same inputs, same policy; round-trips
  through serde for audit.
- Boundary note (measured, not gate-pinned): this crate has **no**
  `BoundaryRule` of its own in `reborn_dependency_boundaries.rs` — its
  leaf-ness is held by the layer matrix plus the measured single-dependency
  manifest, and many other crates' rules forbid depending *on* it from below.
  Adding a second workspace dependency here should be treated as an
  architecture change, not a convenience.

## Tests

```bash
cargo test -p ironclaw_runtime_policy
cargo test -p ironclaw_architecture_tests   # after dependency/API changes
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules and guardrails.
- [`../AGENTS.md`](../AGENTS.md) — the kernel family.
- `docs/internal/reborn/contracts/runtime-profiles.md`,
  `docs/internal/reborn/contracts/runtime-selection.md`.
