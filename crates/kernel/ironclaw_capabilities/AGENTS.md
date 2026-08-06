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
- `CapabilityHost` (`host`) and the invoke/resume/spawn flows/results: direct invoke/resume parameters, `CapabilityInvocationResult`, `CapabilitySpawnRequest`/`CapabilitySpawnResult` (`requests`); `CapabilityInvocationError`/`ResumeContextMismatchKind` (`error`).
- The obligation seam (`obligations`): `CapabilityObligationHandler`, `CapabilityObligationRequest`/`CapabilityObligationOutcome`, abort/completion requests, `CapabilityObligationPhase`/`CapabilityObligationFailureKind`/`CapabilityObligationError`.
- Capability-profile conformance evaluation (`conformance`): `CapabilityProfileClaim`/`CapabilityProfileClaimedOperation`, the conformance report/findings, and `evaluate_profile_conformance`.
- The host-private replay-payload store (`replay_payload`): `ReplayPayload`, the `ReplayPayloadStore` port, `ReplayPayloadStore`, and `ReplayPayloadStoreError`. Persists the raw replay payload a gate/auth resume re-dispatches from, keyed by `InvocationId`, behind a `ScopedFilesystem` CAS lane. Never model-visible (no `SafeSummary`) — see `CLAUDE.md`.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Where Code Goes Inside `host`

`src/host/` is a directory module split one file per **workflow**; the charter
table in `src/host/mod.rs` is the rule for which file an item belongs to. Read
it before adding to any of them. In short: `invoke`, `approval_resume`,
`auth_resume`, `spawn_resume` and `spawn` own one caller-facing workflow each
and nothing else; `authorize` owns the single authorization fold they all
funnel through; `resume_support` owns the tail the three resume workflows
share; `obligation_seams` and `error_mapping` own the seams around them, and
`mod.rs` owns the struct, the `CapabilityAuthorizer` seal and the
cross-workflow types.

The submodules are private and every workflow is an inherent method on
`CapabilityHost`, so callers still see exactly one path. Do not add a
`pub use` for a submodule, and do not let a workflow module grow a policy
branch of its own — that authority belongs to `authorize`.

Each file must stay under the 1,500-line ARCH-SPRAWL threshold; the whole point
of the split was retiring the `arch-exempt: large_file` waiver the fused file
carried, so re-fusing them now trips `scripts/pre-commit-safety.sh`.

## Do Not Move In Here

- parallel dispatch paths, process lifecycle/result APIs, and dispatch before authorization/obligations/approval gates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_capabilities`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
