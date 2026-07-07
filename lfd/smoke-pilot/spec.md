# Spec: smoke-pilot

Run two text-only cases through the real Reborn integration harness using the
already-registered `smoke_builtin_tools` profile.

Non-goals:

- No lane-specific state queries.
- No live model calls.
- No product behavior changes.
- No holdout claims.

The pilot passes only if runner-emitted outcomes satisfy sealed contracts in
`harness/answers.dev.json`.
