---
paths:
  - "crates/**/*.rs"
  - "tests/**"
  - "scripts/**"
  - ".github/**"
---
# Review and fix discipline

## Review the whole contract

Inspect implementation, callers, persistence, wire types, frontend consumers,
tests, and relevant Reborn contracts. Search for the bug pattern across
`crates/`, not only the reported instance. Verify negative claims with both
symbol and concept searches.

Every bug fix needs a regression test that would fail before the fix. Add a
`#[test]`, `#[tokio::test]`, contract test, or integration scenario that
reproduces the original failure. Prefer a caller-level or integration test when
wrappers, computed inputs, or side effects separate the helper from the
behavior. Documentation-only changes are exempt. If a regression test is
genuinely infeasible, document why in the PR and use the repository's explicit
regression-check exemption rather than silently omitting coverage.

## Mechanical review traps

- **Zero warnings:** changed Reborn crates must pass clippy with
  `--all-targets --all-features -- -D warnings`. Before committing, run the
  Reborn workspace-wide command below and fix every warning it surfaces,
  including pre-existing warnings outside the immediate files.
- **Feature matrix, not just `--all-features`:** `--all-features` cannot catch
  feature-gated dead code — a `#[cfg(feature = "x")]`-only caller makes its
  helper *live* under `--all-features` and *dead* (a `-D warnings` error)
  everywhere the feature is off. **PR CI runs only the slim `all-features` lane;
  the full `default` / `libsql-only` matrix runs post-merge**, so this class
  breaks `main` after a green PR. If you add or move a `#[cfg(feature = ...)]`
  gate, or touch a helper only reachable through one, run the full matrix
  locally before merging (see "Required checks"). When you gate the only
  caller(s) of a helper behind a feature, gate the helper's definition with the
  same `#[cfg]`.
- **UTF-8:** never byte-slice user or external strings with `&value[..n]`.
  Use `char_indices()`, `chars()`, or an `is_char_boundary()`-checked boundary.
  Search changed Rust files for suspicious `[..` slicing.
- **Case-insensitive external values:** normalize case-insensitive identifiers,
  media types, extensions, and platform-sensitive path comparisons at the
  boundary with `to_ascii_lowercase()` or `eq_ignore_ascii_case()`. Do not
  lowercase case-sensitive opaque values.
- **Decorator delegation:** when a trait method is added, enumerate every
  production implementation, decorator, adapter, and test double. For
  `LlmProvider`, start with `rg -n "impl LlmProvider for" crates` and test
  through the full wrapper chain.
- **Production panics:** search changed production files for `.unwrap()` and
  `.expect()`; they are prohibited outside tests. Propagate an explicit error
  instead.
- **Imports:** prefer `crate::` for cross-module imports. `super::` is acceptable
  inside tightly coupled submodules and tests.
- **Pattern fixes:** search all of `crates/` for sibling instances of the bug.

## Required checks

Run the narrowest crate tests and clippy first. Add:

```bash
cargo test -p ironclaw_architecture
cargo clippy -p OWNING_CRATE --all-targets --all-features -- -D warnings
scripts/pre-commit-safety.sh
```

Workspace-wide zero-warning clippy:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Full feature matrix — reproduces the post-merge `Code Style` gate that PR CI
skips (it only runs the `all-features` lane). Run all three whenever a change
adds, moves, or relies on a `#[cfg(feature = ...)]` gate:

```bash
cargo clippy --all --tests --examples -- -D warnings                              # default
cargo clippy --all --tests --examples --no-default-features --features libsql -- -D warnings  # libsql-only
cargo clippy --all --tests --examples --all-features -- -D warnings               # all-features
```

Run the Reborn integration or E2E harness when the change crosses turns,
capabilities, authorization, approvals, persistence, runtime lanes, networking,
secrets, product workflow, or user-visible transport.

## Scope discipline

The PR title and body must describe the full diff. If a change crosses several
layers, name that scope or split the PR. Move-only changes state that behavior
is unchanged, keep behavioral fixes separate, and record follow-up issues for
problems discovered during the move. After moving or renaming code, search
`.claude/`, `AGENTS.md`, `CLAUDE.md`, `crates/AGENTS.md`,
`docs/reborn/contracts/`, and other Markdown references for stale paths.

## Guardrails are code

Checks and hooks need regression tests, must handle multiline syntax, and must
run when their own files change. Never claim enforcement without executing the
enforcing command. Comments and docs that promise guarantees must match the
code and tests.
