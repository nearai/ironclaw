# IronClaw Local Harness

The IronClaw local harness should be bootable per git worktree so agents can validate changes without shared mutable state.

This document describes the target shape. It does not mean every command exists today.

## What already exists

Current branch-local harness assets include:

- Rust unit/integration tests;
- `crates/ironclaw_architecture/tests/ironclaw_dependency_boundaries.rs`;
- E2E scenarios under `tests/e2e/scenarios/`;
- replay fixtures under `tests/fixtures/llm_traces/`;
- the recorded-fixture gate in `.github/workflows/ironclaw-tests.yml`
  (`IronClaw QA recorded fixtures` + `scripts/ci/check-ironclaw-qa-fixtures.sh`),
  which replaced the v1 `replay-gate.yml`.

## Goals

- Let agents run IronClaw from an isolated worktree.
- Avoid shared mutable state between concurrent agent tasks.
- Prefer deterministic fake providers and fixtures over live external services.
- Produce redacted artifacts that a reviewer or follow-up agent can inspect.
- Make failures reproducible with a short command and a debug bundle.

## Target command surface

Future tooling may provide:

```bash
scripts/ironclaw-dev up
scripts/ironclaw-dev down
scripts/ironclaw-dev reset
scripts/ironclaw-dev status
scripts/ironclaw-dev logs
scripts/ironclaw-dev seed
scripts/ironclaw-dev doctor
```

## Per-worktree state

Local harness state should live under:

```text
.pi/ironclaw-dev/
  db/
  logs/
  events/
  traces/
  artifacts/
  screenshots/
  config.toml
  tokens.json
```

Rules:

- `.pi/ironclaw-dev/` is local-only state and must not be committed.
- Local tokens must be fake, test-only, or redacted.
- The harness must not require production secrets.
- `reset` should delete only `.pi/ironclaw-dev/`, not user data outside the harness directory.

## Expected local services

A complete IronClaw local harness should be able to start or simulate:

- IronClaw host/runtime composition;
- web gateway;
- fake or trace LLM provider;
- fake embedding provider;
- deterministic MCP fixture;
- deterministic WASM capability fixture;
- deterministic script runtime fixture;
- fake OAuth provider;
- fake channel/webhook adapters;
- local libSQL and, where needed, PostgreSQL test backend;
- JSONL event/audit/log sinks.

## Doctor bundle

`doctor` should create a redacted bundle:

```text
.pi/ironclaw-dev/artifacts/ironclaw-debug/<timestamp>/
  config-redacted.json
  logs.jsonl
  events.jsonl
  audit.jsonl
  process-tree.json
  failed-invocations.json
  screenshots/
  replay-command.txt
```

The bundle should let an agent answer:

- What command was run?
- Which tenant/user/project/agent/thread/run/invocation IDs were involved?
- Which capability or runtime failed?
- Was the failure authorization, approval, resource, network, secret, process, or runtime related?
- Is there a replay command?

## Safety requirements

- Never write raw secrets to logs, events, snapshots, screenshots, or bundles.
- Never call live external services unless the user explicitly opts in.
- Prefer fake/local providers by default.
- Use per-worktree ports, paths, and database names.
- Fail closed when a required fake provider or fixture is missing.
