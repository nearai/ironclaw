# Agent Map — ironclaw_trust

## Start Here

- Read `README.md` for orientation (charter, measured deps/consumers, gates).
  This file is the canonical working-rules home; `CLAUDE.md` is a pointer here.
- Read `CONTRACT.md` — the co-located cross-crate contract (evaluation matrix, requested-vs-effective split, mutation/invalidation orchestration).
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/kernel-boundary.md`
- `docs/reborn/contracts/extensions.md`
- `docs/reborn/contracts/host-api.md`

## What This Crate Owns

- Host-controlled trust evaluation, currently:
- Trust-decision vocabulary: `EffectiveTrustClass`, `TrustDecision`, `AuthorityCeiling`, `HostTrustAssignment`, `TrustProvenance` (`decision`). Privileged variants (FirstParty, System) are crate-internal to construct.
- Trust policy and layered sources: `TrustPolicy`, `HostTrustPolicy` (`policy`); `PolicySource`, `AdminConfig`/`AdminEntry`, `BundledRegistry`/`BundledEntry` (`sources`).
- **`TrustPolicyInput` is not owned here** — it is requested-trust *vocabulary* and lives in `ironclaw_host_api::trust`, beside `PackageIdentity` and `RequestedTrustClass`, every one of which is one of its fields. It moved there so a manifest-bearing package producer (`ironclaw_extension_registry`, layer `substrates`) can describe its trust request without depending on the kernel-layer engine that judges it (PROPOSAL §6.8.1, WS2). Import it from `ironclaw_host_api::trust`; do not re-export it from this crate.
- Synchronous fail-closed invalidation: `InvalidationBus`, `TrustChange`, `TrustChangeListener` (`invalidation`).
- The `Clock` abstraction (`clock`), `TrustError` (`error`), and test fixtures (`fixtures`).
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Guardrails

- Own host-controlled trust evaluation only: `EffectiveTrustClass`,
  `TrustPolicy`, layered `PolicySource`s, and the trust-change invalidation
  contract.
- Privileged variants of `EffectiveTrustClass` (FirstParty, System) MUST only
  be constructible from inside this crate. Public constructors expose Sandbox
  and UserTrusted only.
- Do not import any other `ironclaw_*` crate besides `ironclaw_host_api`. No
  dispatcher, capability host, runtimes, host runtime, approvals, run-state,
  processes, events, resources, or product workflow.
- Manifest input always flows through `TrustPolicyInput`,
  `RequestedTrustClass` and `PackageIdentity` — all three from `host_api`,
  which owns the requested-trust half of this boundary in full. Manifest
  deserialization paths must never construct an `EffectiveTrustClass`
  directly.
- Trust downgrade or revocation must publish on `InvalidationBus`
  synchronously, before any subsequent `evaluate()` returns the new lower
  decision — fail-closed. Runtime mutation must go through
  `HostTrustPolicy::mutate_with`; the per-source `upsert` / `remove` methods
  are `pub(crate)` precisely so this contract cannot be bypassed.
- `TrustClass` ceiling alone grants no capability authority. Authorization
  must consume both an `EffectiveTrustClass` *and* an explicit
  `CapabilityGrant`.
- Identity drift (`package_id`, `source`, `digest`, `signer`) or
  requested-authority growth invalidates retained grants; downstream
  revocation flows use the helpers exposed here.
- The cross-crate contract — evaluation matrix, requested-vs-effective split,
  `PackageIdentity` scope, mutation/invalidation orchestration, and built-in
  tool migration intent — lives in `crates/kernel/ironclaw_trust/CONTRACT.md`
  (co-located with the crate so doc + code review stays unified). Update it
  whenever `TrustPolicy::evaluate`, `default_decision`, source match keys,
  `mutate_with`, or `EffectiveTrustClass` semantics change. The Reborn-track
  docs at `docs/reborn/contracts/host-api.md` / `extensions.md` describe the
  broader vocabulary; reference them from CONTRACT.md, do not duplicate them
  here.

## Do Not Move In Here

- treating trust as a grant/bypass, package execution, extension storage, or capability dispatch.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_trust`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
