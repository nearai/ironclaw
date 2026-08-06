---
paths:
  - "crates/**/*.rs"
  - "tests/**"
---
# Reborn testing rules

## Caller-first coverage

New or changed production-wired behavior needs a caller-level test at the
nearest meaningful seam: route response, mediated egress, durable store
reopen, event/projection output, capability evidence, approval state, or
runtime result. Waiting only for a completed status is not sufficient.

Crate-tier tests are appropriate for local invariants and public contract
conformance. Use `tests/integration/` when behavior crosses crates, turns,
runtime lanes, persistence, or external-service seams. Do not add integration
wiring for a route or local contract that its crate-level caller test can
cover. Never add test-only wiring for behavior production does not wire.

Read `.claude/skills/ironclaw-reborn-testing/SKILL.md` and
`tests/integration/CLAUDE.md` before adding a cross-layer scenario.

## Test tiers

1. **Unit/contract:** pure logic and local public contracts —
   `cargo test -p OWNING_CRATE`.
2. **In-process Reborn integration:** whole deterministic turns with the real
   product orchestration, runner, loop, decorator chain, and in-memory filesystem —
   `cargo test --test reborn_integration_SCENARIO`.
3. **Architecture:** dependency and composition boundaries —
   `cargo test -p ironclaw_architecture_tests`.
4. **Backend/runtime integration:** DB-, Docker-, or runtime-shaped behavior —
   use the owning crate's feature-gated suite
   (`cargo test -p <owning-crate> --features integration`, e.g.
   `-p ironclaw_hooks` for the Postgres/libSQL hooks parity matrix) when its
   guide requires it. The workspace-root `integration` feature is empty with no
   consumers, so a bare root `cargo test --features integration` adds nothing
   over `cargo test`.
5. **Recorded model behavior:** hermetic fixtures for tool choice/request shape;
   validate with `scripts/ci/check-reborn-qa-fixtures.sh`.
6. **Browser/E2E:** user-visible WebUI flows under `tests/e2e/`.
7. **Live canary:** ignored, credentialed drift checks; supplemental only.

Use `bash scripts/reborn-e2e-rust.sh` when a Reborn contract or whole-path
behavior changes. Choose the smallest tier that reaches the changed contract;
a green PR does not prove that unselected tiers ran.

## Test through the caller

A helper-only unit test is insufficient when all of these are true:

1. A predicate, classifier, or transform gates a side effect.
2. A wrapper or computed input sits between the helper and the effect.
3. The helper has multiple inputs or the caller derives an input from context.

Drive the public caller or inline the helper into its sole caller. Runtime and
browser doubles capture every argument the production call supplies.

For provider decorators, runtime adapters, and capability wrappers, the test
must exercise the complete production chain. A direct leaf call does not prove
delegation, redaction, retry, or policy behavior survives every wrapper.

## Required properties

- Every bug fix includes a regression test that fails before the fix. The
  commit-msg hook and `.github/workflows/regression-test-check.yml` enforce the
  repository convention for marked fix/high-risk changes. `[skip-regression-check]`
  is an explicit, review-visible escape hatch for genuinely infeasible cases;
  never use it to avoid a reproducible caller-path test.
- Extend an existing suite when it owns the same seam.
- Use `tempfile` for test files and directories. Never hardcode system
  temporary-directory paths.
- Avoid ignored or TODO-pinned tests; landed tests run in CI.
- Prefer real in-memory implementations, deterministic fakes, or recording
  adapters over mocks that duplicate internal behavior.
- External services are hermetic. Live canaries supplement deterministic tests.
- Test denial, cancellation, restart, conflict, redaction, scope isolation, and
  partial failure where the contract exposes them.
- Recorded model fixtures contain no secrets or PII and pass the repository
  fixture validator.
- Reborn integration coverage follows the committed ratchet in
  `tests/integration/coverage-floor.toml`; when coverage is intentionally added,
  follow that file's same-PR recapture/floor-update instructions.

## Validation

Run targeted crate tests first. Add architecture tests when dependency edges or
ownership change, and the Reborn integration/E2E harness when behavior crosses
turns, runtime lanes, authorization, approvals, networking, secrets, product
workflow, or capability dispatch. Re-derive exact commands from
`crates/AGENTS.md` and the owning crate guide rather than copying stale commands.

Reborn dependency/composition boundary enforcement is
`cargo test -p ironclaw_architecture_tests`.
