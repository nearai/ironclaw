# Agent Map — ironclaw_safety

## Start Here

- No crate-local `CLAUDE.md` exists yet; use this map plus the security rules below.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these sources of truth before changing safety behavior:
- `.claude/rules/safety-and-sandbox.md`
- `src/NETWORK_SECURITY.md`
- `crates/ironclaw_safety/fuzz/README.md`

## What This Crate Owns

- Prompt-injection detection, input validation, sanitization, safety policy evaluation, sensitive path helpers, manual credential detection, and secret leak scanning.
- Crate-local public API, tests, benches, fuzz targets, and fixtures needed to prove that ownership.

## Do Not Move In Here

- Sandbox execution, credential storage/injection, network allowlists, tool dispatch, agent loops, or UI decisions.
- Any code path that logs or returns raw secret values while reporting a safety finding.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_safety`
- Fuzz-target check after parser/pattern changes: follow `crates/ironclaw_safety/fuzz/README.md`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- New ingress surfaces outside this crate must scan before storage or LLM injection; keep APIs easy for callers to use correctly.
- Prefer bounded, linear-time pattern matching; do not add backtracking regex behavior on untrusted input.
- Add regression tests for both blocked and allowed near-miss cases when tuning detectors.
