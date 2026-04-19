---
description: Run the full Rust quality gate (fmt, clippy, tests) before shipping changes
allowed-tools: Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*)
---

Run the IronClaw shipping checklist. This is the mandatory quality gate before any change is considered done.

## Steps

1. **Format**: Run `cargo fmt` to normalize formatting.

2. **Lint**: Run `cargo clippy --all --benches --tests --examples --all-features` and report any warnings or errors. ALL clippy warnings must be resolved before proceeding.

3. **Test**: Run `cargo test --lib` to execute the full library test suite. Report the total pass/fail count.

4. **Summary**: Report results for all three steps. If any step failed, list the specific errors and suggest fixes. Do NOT proceed past a failing step.

If `$ARGUMENTS` is provided, treat it as a specific test filter and run `cargo test --lib -- $ARGUMENTS` instead of the full suite in step 3.

The expected outcome for a clean ship is:
- `cargo fmt` produces no changes
- `cargo clippy` has zero warnings
- All tests pass

Note: Integration tests (`--test workspace_integration`) require a PostgreSQL database and are expected to fail locally. Only report `--lib` test failures as blocking.
