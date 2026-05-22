# Agent Map — ironclaw_interactions

## Start Here

- Read `CLAUDE.md` first for crate-local guardrails.
- Read `Cargo.toml` for actual dependency shape.
- Issue #3094 defines the contract: `ApprovalInteractionService`,
  `AuthInteractionService`, redaction rules, and acceptance tests.

## What This Crate Owns

- `ApprovalInteractionService` — list scoped pending approvals as redacted
  DTOs; route approve/deny through a typed `ApprovalDecisionPort`.
- `AuthInteractionService` — list scoped auth-required gates as redacted
  DTOs; route resume/cancel through a typed `AuthFlowManager` boundary.
- The `AuthFlowManager` trait + in-memory test fake.

## Do Not Move In Here

- Capability dispatch, runtime execution, OAuth flow concretes, raw secret
  handling, network policy, or filesystem access.
- Approval persistence (lives in `ironclaw_run_state`) or approval
  resolution-to-lease (lives in `ironclaw_approvals`).

## Validation

- `cargo test -p ironclaw_interactions`
- `cargo clippy -p ironclaw_interactions --tests --all-features`
