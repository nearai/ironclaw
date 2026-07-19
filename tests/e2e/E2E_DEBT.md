# E2E Debt

## Open Gap

The old full-path Emulate tests were not Reborn tests: their fixtures built and
started `ironclaw-legacy` and used the retired gateway API. They were removed
with v1.

A replacement should start canonical `ironclaw serve`, install and configure a
first-party extension through current product APIs, drive a turn through the
Reborn runner, and read the resulting provider mutation back from Emulate.
Provider-contract tests remain in
`test_emulate_reborn_provider_contracts.py`, but they intentionally do not claim
runtime wiring coverage.

## Policy

- Runtime skips are acceptable only for prerequisites outside the hermetic
  harness, with a specific reason.
- Browser lifecycle tests should mock external registries unless that external
  contract is the behavior under test.
- Placeholder skipped tests belong in an issue or this debt file, not as empty
  test functions.
- Do not revive v1 fixtures to recover coverage; port the behavior through the
  canonical Reborn harness.
