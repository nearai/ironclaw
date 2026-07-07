# IronClaw LFD Infrastructure

This directory contains reusable loss-function-development infrastructure for
IronClaw features. It is intentionally not tied to a roadmap lane: the shared
schema, scorer, templates, and smoke pilot are the base that future feature LFDs
copy or extend.

## What Is Included

- `lfd/_shared/SCHEMA.md`: JSON contracts between visible cases, runner
  outcomes, and sealed scorer answers.
- `lfd/_shared/scorer/`: deterministic scoring, lint, probe, and status
  helpers, with a stdlib-only selftest.
- `lfd/_shared/templates/`: shell wrappers and starter cap/log templates for a
  new feature package.
- `tests/integration/lfd/`: the data-driven Reborn integration runner.
- `tests/integration/lfd/profiles/smoke_builtin_tools.rs`: a minimal example
  profile that wires the real Reborn harness for builtin tool scenarios.
- `lfd/smoke-pilot/`: a tiny end-to-end package used to prove the runner and
  scorer work together.

## Feature Package Shape

A feature LFD should add one package and one profile:

```text
lfd/<feature>/
  goal.md
  spec.md
  LOG.md
  eval/dev/cases/*.json
  harness/answers.dev.json
  harness/caps.json
  harness/lint.sh
  harness/pins.json
  harness/probe.sh
  harness/score.sh
  harness/status.sh

tests/integration/lfd/profiles/<feature>.rs
```

Visible case inputs live under `eval/dev/cases`. Dev answers are sealed by
policy and linted for canary/literal leakage. Holdout answers should live
outside the repository under `$LFD_STATE_ROOT/holdout/<feature>/`.

## Runner Flow

Run a package by pointing the integration test at cases and an output
directory:

```bash
CARGO_TARGET_DIR=target-lfd \
LFD_CASES=lfd/smoke-pilot/eval/dev/cases \
LFD_OUT=/tmp/lfd-smoke-out \
cargo test --test reborn_lfd_runner -- --nocapture
```

Then score the emitted outcomes:

```bash
lfd/smoke-pilot/harness/score.sh \
  --repo-root . \
  --state-root /tmp/lfd-smoke-state \
  --outcomes /tmp/lfd-smoke-out
```

The scorer runs lint first. A lint violation prints only
`VOID: constraint violation`; detailed reports go under
`$LFD_STATE_ROOT/lint-reports/` so the optimizer cannot mine sealed answers.

## Adding A New Feature LFD

1. Copy `lfd/_shared/templates/` into `lfd/<feature>/harness/`.
2. Add visible cases under `lfd/<feature>/eval/dev/cases/`.
3. Add sealed dev contracts in `lfd/<feature>/harness/answers.dev.json`.
4. Define capacity caps and base ref in `harness/caps.json`.
5. Add a profile under `tests/integration/lfd/profiles/` and register it in
   `profiles/mod.rs`.
6. Generate `harness/pins.json` after the profile and runner surface are final.
7. Verify with `lint.sh`, `probe.sh`, `status.sh`, the runner, and `score.sh`.

The smoke pilot is the reference for a complete but small package.
