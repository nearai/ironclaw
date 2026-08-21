---
description: Run the full Rust quality gate (fmt, clippy, tests) before shipping changes
allowed-tools: Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*)
---

Run the IronClaw shipping checklist. This is the mandatory quality gate before any change is considered done.

## Steps

1. **Format**: Run `cargo fmt` to normalize formatting.

2. **Lint**: Run `cargo clippy --all --benches --tests --examples --all-features -- -D warnings` and report any warnings or errors. ALL clippy warnings must be resolved before proceeding — CI denies warnings, and an unflagged run exits 0 with them still present, so `-D warnings` is not optional.

3. **Test**: Run `cargo test -- --nocapture` (unit + integration suites) and report the total pass/fail count. Postgres-backed legs self-provision via testcontainers and short-circuit to a passing `ok` (not `ignored`) when Docker is unavailable — that's expected, not a failure, but it means the pass count alone can't show whether those legs actually ran. `--nocapture` surfaces each leg's `eprintln!("skipping Postgres ...")` diagnostic even though the test passed (`cargo test` alone suppresses stdout/stderr for passing tests); count those lines in the output and report that count separately from the real pass count.

4. **Summary**: Report results for all three steps. If any step failed, list the specific errors and suggest fixes. Do NOT proceed past a failing step.

If `$ARGUMENTS` is provided, treat it as a specific test filter and run `cargo test -- $ARGUMENTS` instead of the full suite in step 3.

The expected outcome for a clean ship is:
- `cargo fmt` produces no changes
- `cargo clippy` has zero warnings
- All tests pass (Docker-less environments will show Postgres legs skipped, not failed)
