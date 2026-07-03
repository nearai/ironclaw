# Reborn Local Harness

The Reborn local harness should be bootable per git worktree so agents can validate changes without shared mutable state.

This document describes the target shape. It does not mean every command exists today.

## What already exists

Current branch-local harness assets include:

- Rust unit/integration tests;
- `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`;
- E2E scenarios under `tests/e2e/scenarios/`;
- replay fixtures under `tests/fixtures/llm_traces/`;
- live/replay test guide at `tests/support/LIVE_TESTING.md`;
- replay workflow at `.github/workflows/replay-gate.yml`.

## Goals

- Let agents run Reborn from an isolated worktree.
- Avoid shared mutable state between concurrent agent tasks.
- Prefer deterministic fake providers and fixtures over live external services.
- Produce redacted artifacts that a reviewer or follow-up agent can inspect.
- Make failures reproducible with a short command and a debug bundle.

## Target command surface

Future tooling may provide:

```bash
scripts/reborn-dev up
scripts/reborn-dev down
scripts/reborn-dev reset
scripts/reborn-dev status
scripts/reborn-dev logs
scripts/reborn-dev seed
scripts/reborn-dev doctor
```

## Per-worktree state

Local harness state should live under:

```text
.pi/reborn-dev/
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

- `.pi/reborn-dev/` is local-only state and must not be committed.
- Local tokens must be fake, test-only, or redacted.
- The harness must not require production secrets.
- `reset` should delete only `.pi/reborn-dev/`, not user data outside the harness directory.

## Expected local services

A complete Reborn local harness should be able to start or simulate:

- Reborn host/runtime composition;
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
.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/
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
