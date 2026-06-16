# Agent Map — ironclaw_capabilities

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/capability-access.md`
- `docs/reborn/contracts/capabilities.md`
- `docs/reborn/contracts/approvals.md`
- `docs/reborn/contracts/run-state.md`

## What This Crate Owns

- The single caller-facing `CapabilityHost` authority path, currently:
- `CapabilityHost` (`host`) and the invoke/resume/spawn requests/results: `CapabilityInvocationRequest`/`CapabilityInvocationResult`, `CapabilityResumeRequest`, `CapabilitySpawnRequest`/`CapabilitySpawnResult` (`requests`); `CapabilityInvocationError`/`ResumeContextMismatchKind` (`error`).
- The obligation seam (`obligations`): `CapabilityObligationHandler`, `CapabilityObligationRequest`/`CapabilityObligationOutcome`, abort/completion requests, `CapabilityObligationPhase`/`CapabilityObligationFailureKind`/`CapabilityObligationError`.
- Capability-profile conformance evaluation (`conformance`): `CapabilityProfileClaim`/`CapabilityProfileClaimedOperation`, the conformance report/findings, and `evaluate_profile_conformance`.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- parallel dispatch paths, process lifecycle/result APIs, and dispatch before authorization/obligations/approval gates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_capabilities`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
