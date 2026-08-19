# Agent Map — ironclaw_runtime_policy

## Start Here

- Read `README.md` for orientation (charter, measured deps/consumers, gates).
  This file is the canonical working-rules home; `CLAUDE.md` is a pointer here.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/internal/reborn/contracts/runtime-profiles.md`
- `docs/internal/reborn/contracts/runtime-selection.md`
- `docs/internal/reborn/contracts/runtime-workflows.md`

## What This Crate Owns

- Runtime profile resolver and runtime selection policy.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Guardrails

- Own the runtime policy resolver:
  `(DeploymentMode, RuntimeProfile, OrgPolicyConstraints) → EffectiveRuntimePolicy`.
- Depend only on `ironclaw_host_api` for vocabulary and `serde`/`thiserror`
  for plumbing. Do not pull in runtime crates, host runtime, capability host,
  secrets, network, or product workflow crates.
- Resolution must be **deterministic** and **monotonic with respect to
  safety**: deployment mode and tenant/org policy may *reduce* the requested
  profile's authority; they must never *increase* it.
- Fail-closed by default: invalid `(deployment, profile)` pairs are an error,
  not a silent downgrade. Yolo profiles require explicit caller-supplied
  disclosure. `EnterpriseYoloDedicated` requires both `EnterpriseDedicated`
  deployment and explicit org admin approval.
- The resolver is the only sanctioned producer of `EffectiveRuntimePolicy`.
  Treat values constructed elsewhere as untrusted.
- Output must be serializable for audit/debugging — `EffectiveRuntimePolicy`
  round-trips through serde and `was_reduced()` flags the narrowing case so
  audit can render "you asked for X, you got Y".
- Do not re-implement authorization/approvals/grants. The resolver picks
  backend kinds and policy modes; per-invocation authorization runs on top
  via `ironclaw_authorization` / `CapabilityHost`.
- No I/O in the resolver. It is a pure function over types from
  `ironclaw_host_api::runtime_policy`.

## Do Not Move In Here

- runtime process startup, action dispatch, or product strategy outside profile selection.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_runtime_policy`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
