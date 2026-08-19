# Agent Map — ironclaw_authorization

## Start Here

- Read `README.md` for orientation (charter, measured deps/consumers, gates).
  This file is the canonical working-rules home; `CLAUDE.md` is a pointer here.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/internal/reborn/contracts/capability-access.md`
- `docs/internal/reborn/contracts/kernel-boundary.md`
- `docs/internal/reborn/contracts/host-api.md`

## What This Crate Owns

- Grant matching, capability-lease state, and dispatch/spawn authorization decisions (default-deny), currently:
- Authorizer ports and implementations: `CapabilityDispatchAuthorizer` / `TrustAwareCapabilityDispatchAuthorizer` traits, `GrantAuthorizer`, `LeaseBackedAuthorizer`, and the `grant_exceeds_authority_ceiling` check.
- Capability-lease state: `CapabilityLease` (+ `CapabilityLeaseStatus`/`CapabilityLeaseError`), the `CapabilityLeaseStore` trait, and the one production `CapabilityLeaseStore<F>` backend (writes via bounded compare-and-swap — `CasExpectation::Version` with a retry budget — over versioned roots for cross-process safety, plus per-owner process-local mutation locks; only byte-only/`Unsupported` roots degrade to process-local serialization alone). Tests instantiate that same store over an in-memory backend through the `test-support` `in_memory_backed_capability_lease_store()` constructor — "in-memory" is a filesystem backend (`InMemoryBackend`), not a bespoke store (arch-simplification §4.3); the hand-written `InMemoryCapabilityLeaseStore` was deleted.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Guardrails

- Own grant matching, lease state, and dispatch/spawn authorization decisions.
- Do not execute capabilities, persist run-state, resolve approvals, reserve
  resources, prompt users, or import runtime/process/dispatcher/capability
  workflow crates.
- Authorization is default-deny and resource-owner/invocation scoped
  (tenant/user/agent/project/mission/thread plus invocation where applicable).
- Filesystem-backed leases must use async filesystem calls, not nested
  `block_on`.
- Only byte-only/`Unsupported` filesystem roots degrade the lease store to
  process-local serialization alone — those are not safe for real concurrent
  cross-process callers (the full store shape, CAS discipline included, is in
  "What This Crate Owns" above).
- Fingerprinted approval leases are resume-only authority and must not become
  ambient grants.

## Do Not Move In Here

- approval lease claiming, runtime dispatch, obligation execution, or stringly permission logic.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_authorization`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
