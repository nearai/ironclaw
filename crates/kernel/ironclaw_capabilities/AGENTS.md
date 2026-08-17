# Agent Map — ironclaw_capabilities

## Start Here

- Read `README.md` for orientation (charter, measured deps/consumers, gates).
  This file is the canonical working-rules home; `CLAUDE.md` is a pointer here.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/internal/reborn/contracts/capability-access.md`
- `docs/internal/reborn/contracts/capabilities.md`
- `docs/internal/reborn/contracts/approvals.md`
- `docs/internal/reborn/contracts/run-state.md`

## What This Crate Owns

- The single caller-facing `CapabilityHost` authority path, currently:
- `CapabilityHost` (`host`) and the invoke/resume/spawn flows/results: direct invoke/resume parameters, `CapabilitySpawnRequest`/`CapabilitySpawnResult` (`requests`); `CapabilityDispatchResult` re-exported at the crate root from `ironclaw_host_api::dispatch`; `CapabilityInvocationError`/`ResumeContextMismatchKind` (`error`).
- The obligation seam (`obligations`): `CapabilityObligationHandler`, `CapabilityObligationRequest`/`CapabilityObligationOutcome`, abort/completion requests, `CapabilityObligationPhase`/`CapabilityObligationFailureKind`/`CapabilityObligationError`.
- The host-private replay-payload store (`replay_payload`): `ReplayPayload`, the `ReplayPayloadStore` port, `ReplayPayloadStore`, and `ReplayPayloadStoreError`. Persists the raw replay payload a gate/auth resume re-dispatches from, keyed by `InvocationId`, behind a `ScopedFilesystem` CAS lane. Never model-visible (no `SafeSummary`) — see Guardrails below.
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

## Guardrails

- `CapabilityHost` is the single caller-facing authority path for
  invoke/resume/spawn: host-runtime adapters, built-ins, custom packages, and
  external runtimes must enter through this workflow rather than adding
  parallel authorization/approval dispatch paths. `authorize` decides; a
  workflow module only maps the verdict.
- Use the neutral `CapabilityDispatcher` port; do not add a normal dependency
  on concrete runtime crates.
- Host authorization must use the trust-aware contract
  (`TrustAwareCapabilityDispatchAuthorizer`) with a policy-derived
  `TrustDecision`; do not wire production `CapabilityHost` with grant-only
  authorization that bypasses trust ceilings.
- Do not absorb process lifecycle/result APIs; those belong in
  `ironclaw_processes::ProcessHost`.
- Approval resume must validate and claim the matching fingerprinted lease
  before dispatch.
- Authorization denial or unsupported/failed obligations must fail before
  runtime dispatch, process start, or approval lease claim.
- Keep obligation handling behind the seam; the built-in obligation
  implementations live in
  `crates/kernel/ironclaw_host_runtime/src/obligations/`, never here.
- The `ReplayPayloadStore` (`replay_payload`) persists the **host-private**
  raw replay payload (tool `input`, `estimate`, prior-approval identity,
  input ref, correlation id) a gate/auth resume re-dispatches from, keyed by
  `InvocationId`. It is the opposite of a model-visible `GateRecord`: it
  carries no `SafeSummary` and must never reach the model, an event, an
  error, a snapshot, or a log — the record exists only for host-side
  re-dispatch. It lives here because capabilities owns the invoke/resume
  workflow this payload serves; approval and turn contracts forbid raw replay
  input. The `ironclaw_filesystem` / `ironclaw_turns` dependencies exist for
  this store: it persists behind a `ScopedFilesystem` over the shared
  `cas_update` lane (fail-closed on non-CAS backends) and embeds the
  resume-payload field types owned by `ironclaw_turns` (`CapabilityInputRef`,
  `AuthResumeApprovalIdentity`) rather than re-typing them. Write-once; no
  removal method until an explicit retention contract adds one.

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
