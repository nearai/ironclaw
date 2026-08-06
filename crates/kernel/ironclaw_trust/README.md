# ironclaw_trust

The trust-ceiling stage of the kernel pipeline: resolves a package's
*requested* trust into the host-validated *effective* ceiling every
authorization decision consumes. It is its own crate because the seal is a
property of crate-scoped visibility — only this crate's policy evaluation can
produce a privileged ceiling, and that is true because the privileged
constructors and per-source mutators are unreachable from any other crate, not
because reviewers promise not to call them.

- **Family / layer:** `kernel/` / `kernel` · **Package:** `ironclaw_trust` ·
  **Manifest:** `crates/kernel/ironclaw_trust/Cargo.toml`
- **Use this when:** you need to evaluate, mutate, or subscribe to what a
  package is *allowed to be trusted with* (ceiling, provenance, invalidation).
- **Don't use this when:** you want to know what a caller may *do right now* —
  that is grant matching (`ironclaw_authorization`) and the membrane
  (`ironclaw_capabilities`). Requested-trust vocabulary
  (`TrustPolicyInput`, `RequestedTrustClass`, `PackageIdentity`) lives in
  `ironclaw_host_api::trust`, not here.

## Public surface

- `EffectiveTrustClass` (`src/decision.rs:31`) — the sealed ceiling. Public
  constructors exist only for `sandbox()` / `user_trusted()`; `FirstParty` /
  `System` values come only from `TrustPolicy::evaluate`. `Serialize` for
  audit, deliberately no `Deserialize` (`decision.rs:24-28`).
- `HostTrustAssignment` (`src/decision.rs:112`) — host-wiring *seed* for
  policy-source entries; convertible to an effective class only via the
  `pub(crate)` `into_effective` (`:145`), i.e. only through evaluation.
- `TrustDecision`, `AuthorityCeiling`, `TrustProvenance` (`src/decision.rs`).
- `TrustPolicy` / `HostTrustPolicy` (`src/policy.rs`); runtime mutation only
  through `HostTrustPolicy::mutate_with` (`policy.rs:269`) — the per-source
  `upsert`/`remove` are `pub(crate)` (`sources.rs:144-570`) so the
  invalidation contract cannot be bypassed.
- Layered sources: `PolicySource`, `AdminConfig`/`AdminEntry`,
  `BundledRegistry`/`BundledEntry` (`src/sources.rs`).
- `InvalidationBus`, `TrustChange`, `TrustChangeListener`
  (`src/invalidation.rs`) — synchronous, fail-closed invalidation.
- `Clock` (`src/clock.rs`), `TrustError` (`src/error.rs`), fixtures
  (`src/fixtures.rs`).

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_host_api` only — the leanest possible
  stage. Reproduce: `cargo metadata --no-deps` filtered to this package.
- **Normal consumers (7):** `ironclaw_approvals`, `ironclaw_authorization`,
  `ironclaw_capabilities`, `ironclaw_host_runtime` (kernel siblings, pinned in
  `reborn_same_layer_edge_inventory.rs`), `ironclaw_composition`,
  `ironclaw_extension_host`, `ironclaw_extension_manager`.

## Invariants

- A privileged `EffectiveTrustClass` cannot be constructed, deserialized, or
  defaulted outside this crate — compiler-enforced visibility
  (`decision.rs:18-33`), with host_api's `#[serde(skip_deserializing)]`
  guarding the wire half.
- A ceiling grants nothing by itself: authorization must consume both an
  `EffectiveTrustClass` *and* an explicit grant (see the family fail-closed
  table and `ironclaw_authorization`).
- Trust downgrade/revocation publishes on `InvalidationBus` synchronously,
  before any subsequent `evaluate()` returns the lower decision.
- Dependency boundary: the `BoundaryRule` for `ironclaw_trust` in
  `reborn_dependency_boundaries.rs` forbids every kernel sibling and every
  substrate — this crate stays a leaf over `host_api`.

## Tests

```bash
cargo test -p ironclaw_trust
cargo test -p ironclaw_architecture_tests   # after dependency/API changes
```

## See also

- [`AGENTS.md`](./AGENTS.md) — working rules and guardrails for this crate.
- [`CONTRACT.md`](./CONTRACT.md) — the co-located cross-crate contract
  (evaluation matrix, requested-vs-effective split, mutation/invalidation
  orchestration).
- [`../AGENTS.md`](../AGENTS.md) — the kernel family: pipeline, sealed mints,
  armed gates.
