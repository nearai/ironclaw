# Task 2 Report: Dispatch One Exact Auth Resolution

## Status

DONE_WITH_CONCERNS. The requested Task 2 implementation is complete and its
owning production libraries and turns tests are green. Product-workflow
all-target test compilation is blocked by the intentionally stale
`ironclaw_channel_host` auth continuation consumer that Task 3 will migrate.

## Implementation

- Replaced the split product auth continuation API with the exported
  `ProductAuthTurnGateResumeDispatcher::dispatch_auth_resolved(AuthResolved)`
  and the minimal `AuthResolutionDispatchOutcome::{Resumed, Canceled, Ignored}`.
- The dispatcher reads the run first and only mutates an exact
  `BlockedAuth`/gate-ref match. Missing, terminal, differently blocked, and
  newer-gate runs return `Ignored`; the final coordinator compare-and-mutate
  precondition still closes the race after the read.
- `Authorized` resumes without a disposition. `ProviderDenied` resumes with
  `Denied`; `Expired` and `Failed` resume with the new `Error` disposition.
  `UserAborted` cancels with the exact `BlockedAuthGate` precondition.
- Explicit product `Deny` now resolves the durable flow through `cancel_flow`,
  observes the terminal winner on races, constructs the same `AuthResolved`
  contract, dispatches through the same concrete dispatcher, and marks the
  durable resolution delivered. The prior reservation/finalize/rollback path
  is gone.
- The pending product view exposes only canonical `Open` and `Processing`
  statuses; terminal auth resolutions are not pending views.
- `CancelRunPrecondition` is narrowed to auth only. Generic resource and
  dependent-run declines retain their caller-side state validation and use an
  unconditional cancellation request.

## Files

- `crates/ironclaw_product_workflow/src/auth_continuation.rs`
- `crates/ironclaw_product_workflow/src/auth_interaction/service.rs`
- `crates/ironclaw_product_workflow/src/auth_interaction/types.rs`
- `crates/ironclaw_product_workflow/src/lib.rs`
- `crates/ironclaw_product_workflow/src/reborn_services.rs`
- `crates/ironclaw_product_workflow/tests/auth_interaction_contract.rs`
- `crates/ironclaw_product_workflow/tests/product_workflow_contract.rs`
- `crates/ironclaw_product_workflow/tests/reborn_services_contract.rs`
- `crates/ironclaw_turns/src/request.rs`
- `crates/ironclaw_turns/tests/turn_coordinator_contract.rs`

## TDD Evidence

### RED

The new turns request tests were added before the production enum change:

```text
cargo test -p ironclaw_turns request::tests::
error[E0599]: no variant or associated item named `Error` found for enum
`GateResumeDisposition`
```

Caller-level dispatcher contract tests were then added for all terminal
outcomes, exact cancel, stale/missing/newer gates, duplicate delivery, and the
final cancel race. Product-workflow compilation reached the Task 1 migration
breaks before those tests could run, including missing
`AuthContinuationEvent`, old `AuthFlowStatus`, and removed
reservation/finalize/rollback methods.

### GREEN

```text
cargo test -p ironclaw_turns
all crate unit and integration test binaries passed; 0 failed

cargo check -p ironclaw_product_workflow
Finished successfully

cargo clippy -p ironclaw_product_workflow --lib --all-features -- -D warnings
Finished successfully with zero Rust warnings

cargo fmt --all -- --check
Passed

cargo clippy -p ironclaw_turns --all-targets --all-features -- -D warnings
Finished successfully with zero Rust warnings

git diff --check
Passed
```

Cargo printed the repository's existing `unused config key net.retries`
notice; clippy emitted no Rust warning.

## Known Downstream Blocker

Fresh verification:

```text
cargo test -p ironclaw_product_workflow --no-run
error[E0432]: unresolved import `ironclaw_auth::AuthContinuationEvent`
  --> crates/ironclaw_channel_host/src/auth_continuation.rs:10:21
error: could not compile `ironclaw_channel_host` (lib) due to 1 previous error
```

`ironclaw_channel_host` is outside Task 2 and is explicitly assigned to Task 3,
so this task does not patch around the removed event API.

## Self-Review

- No production `.unwrap()` or `.expect()` was added.
- `ResumeTurnRequest` still carries no mirrored credential identifier;
  authorized account evidence remains in the durable auth resolution.
- Exact state and gate checks precede mutation, while the coordinator request
  preserves the atomic exact-gate precondition for the final race.
- Explicit denial has one durable terminal winner and no reservation,
  finalization, rollback, or process-local coordination path.
- Searches found no old auth continuation event/status/reservation APIs in the
  changed product-workflow source.
- Broad resource/dependent-run cancellation preconditions were removed only
  from cancellation; their resume preconditions remain intact.
- No channel-host, composition, generated, secret, schema, or PII changes were
  made.

## Concerns

Only the known downstream Task 3 compilation blocker above. No additional
concern was found in Task 2 scope.
