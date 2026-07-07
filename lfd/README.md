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

## Worked Example

The `smoke-pilot` package is the smallest useful example. It answers the
question: "Can a scripted model reply travel through the real Reborn harness,
get extracted as an outcome, and satisfy a sealed contract?"

One visible case describes the input and scripted model behavior:

```json
{
  "case_id": "smoke_dev_reply",
  "profile": "smoke_builtin_tools",
  "title": "scripted text reply is persisted and extracted",
  "llm_script": [
    {"turn": 1, "steps": [{"text": "pilot smoke reply complete"}]}
  ],
  "inbound": [
    {"channel": "pilot", "payload": {"text": "please produce the pilot smoke reply"}}
  ]
}
```

The sealed answer in `harness/answers.dev.json` scores the emitted outcome
without exposing matcher details to the optimizer:

```json
{
  "case_id": "smoke_dev_reply",
  "required": [
    {"id": "reply", "type": "reply_contains", "substrings_any": ["pilot smoke reply complete"]}
  ],
  "forbidden": [{"type": "leak"}]
}
```

A developer can run the package locally, inspect only the outcome artifacts,
and get a stable score:

```bash
rm -rf /tmp/lfd-smoke-out /tmp/lfd-smoke-state
mkdir -p /tmp/lfd-smoke-out /tmp/lfd-smoke-state

CARGO_TARGET_DIR=target-lfd \
LFD_CASES=lfd/smoke-pilot/eval/dev/cases \
LFD_OUT=/tmp/lfd-smoke-out \
cargo test --test reborn_lfd_runner -- --nocapture

lfd/smoke-pilot/harness/score.sh \
  --repo-root . \
  --state-root /tmp/lfd-smoke-state \
  --outcomes /tmp/lfd-smoke-out
```

Expected output:

```text
score: 1.0000
cases: 2
worst:
  smoke_dev_reply PASS
  smoke_dev_second_reply PASS
```

For a real feature, replace the smoke profile with a feature profile, replace
the visible cases with representative product flows, and encode the contract
that should stay true while implementation changes.

## DevX Value

LFD packages give feature teams and coding agents a repeatable development
loop:

- Turn a feature expectation into a runnable integration case, not just prose.
- Exercise the real Reborn harness while keeping feature-specific setup isolated
  in one profile file.
- Score behavior objectively after each implementation attempt.
- Catch both missing behavior and unsafe extra behavior, including leaks.
- Generate probe variants to detect memorization or overfitting.
- Hand agents a concrete optimization target while preserving sealed answers
  and holdout data.
- Keep PR review small: land infrastructure once, then review feature packages
  one at a time.

In practice, this turns "make the feature work" into "improve the score without
violating caps, leaking secrets, or editing the pinned harness." That is easier
to hand off, easier to reproduce locally, and easier to compare across attempts.

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
