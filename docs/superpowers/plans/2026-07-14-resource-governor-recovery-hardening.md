# Resource governor recovery hardening implementation plan

> Approved continuation of the resource-governor recovery work on PR #6089.

## 1. Pin lifecycle and acknowledgement regressions

**Files:**

- `crates/ironclaw_resources/src/filesystem_governor.rs`
- `crates/ironclaw_resources/tests/resource_governor_contract.rs`

Add failing tests for restart failure leaving the poisoned authority installed
and for an acknowledged operation remaining successful after a later operation
invalidates the generation. Then publish an explicit `Recovering` state before
releasing the authority lock, restart the journal outside the critical section,
clear the authority only on successful restart, preserve the primary error,
and remove the post-ack availability check.

## 2. Bound and observe journal contention retries

**Files:**

- `crates/ironclaw_resources/src/filesystem_governor/journal.rs`
- `docs/reborn/contracts/resources.md`

Add a deadline-exhaustion regression around the atomic batch retry helper. Keep
retry eligibility limited to `BackendBusy`, stop starting retries after the
bounded retry window, and log attempts/exhaustion with sanitized attempt,
elapsed, and batch-size fields. Document the no-ambiguous-commit requirement.

## 3. Make default budget seeding fail closed

**Files:**

- `crates/ironclaw_loop_host/src/budget_accountant.rs`

Change seeding helpers to return errors. Update the transient failure test so
the first pre-model call returns `BudgetAccountingFailed` without creating a
reservation, while a second call retries seeding and succeeds.

## 4. Retain and retry the correct post-call action

**Files:**

- `crates/ironclaw_loop_host/src/budget_accountant.rs`
- `crates/ironclaw_turns/src/run_profile/model.rs`
- `crates/ironclaw_turns/tests/agent_loop_host_contract.rs`

Represent the pending disposition as release or reconcile-with-actual-usage.
Retry storage failures without removing the in-flight record. Disarm the model
port guard only after post-call accounting succeeds; on failure its drop path
retries the retained disposition. Add caller-level tests for the guard and
accountant tests proving actual usage is not replaced by release.

## 5. Complete backend parity review feedback

**Files:**

- `crates/ironclaw_filesystem/src/db.rs`
- `docs/reborn/contracts/resources.md`

Map Postgres SQLSTATE `40001`, `40P01`, and `55P03` to `BackendBusy`, retain
generic mapping for all other errors, and keep focused classification coverage.
Document mandatory SQLite/libSQL and Postgres behavior.

## 6. Rebase and verify the exact final head

Rebase onto `origin/main`, resolve by preserving main's current canary/testing
approach, then run:

```bash
cargo test -p ironclaw_filesystem --all-features
cargo test -p ironclaw_resources --all-features
cargo test -p ironclaw_loop_host --all-features
cargo test -p ironclaw_turns --all-features --test agent_loop_host_contract
cargo test -p ironclaw_agent_loop -p ironclaw_runner --all-features
cargo test -p ironclaw_reborn_composition --all-features --lib
cargo test -p ironclaw_reborn_composition --all-features --test resource_governor_libsql_contract
cargo test -p ironclaw_architecture
bash scripts/reborn-e2e-rust.sh
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build -p ironclaw_reborn_cli --all-features
scripts/pre-commit-safety.sh
```

Inspect the final diff for production `unwrap`/`expect`, schema or compatibility
changes, unrelated churn, and stale docs. Commit, push, resolve review threads,
and monitor required checks plus new review feedback until GitHub reports the
exact pushed head mergeable with all required checks successful.

## 7. Close final review gaps without weakening atomicity

**Files:**

- `crates/ironclaw_loop_host/src/budget_accountant.rs`
- `crates/ironclaw_resources/src/filesystem_governor.rs`
- `crates/ironclaw_resources/src/filesystem_governor/journal.rs`

1. Add a failing accountant test in which a failed provider call selects
   `Release`, the first storage-backed release fails, the retained action is
   retried, and no reservation remains. Then keep the model-visible error
   sanitized while binding the underlying `ResourceError` into the internal
   accounting-failure log.
2. Add a test-only recovery barrier and a failing concurrency test that holds
   journal restart after the authority becomes `Recovering`. While the
   authority mutex is released, verify that both reload and stale enqueue fail
   closed; release the barrier and verify the next operation reloads durable
   state.
3. Add a failing journal contract test proving one queued delta calls only
   `append` and two queued deltas call only `append_batch`. Restore the previous
   singleton branch inside the existing bounded retry loop; do not retry any
   error other than `BackendBusy` and do not split multi-delta batches.
4. Run the focused red/green tests after each change, followed by the full
   `ironclaw_resources`, `ironclaw_loop_host`, caller-level turn contract,
   workspace clippy, build, architecture, Reborn E2E, and pre-commit commands
   listed above. Push the rebased head, reply in each inline review thread, and
   resolve only after the final implementation is visible on GitHub.
