# Agent Map — ironclaw_engine

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these design docs/contracts as the source of truth before changing behavior:
- `docs/plans/2026-03-20-engine-v2-architecture.md`
- `docs/reborn/contracts/agent-loop-protocol.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/kernel-boundary.md`

## What This Crate Owns

- Unified thread/capability/CodeAct execution model: thread and step types, capability leases and policy, execution gates, memory docs, mission runtime, and host-facing execution traits.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- Concrete LLM providers, database backends, tool registries, channel transports, web/TUI UI, raw secrets, or host-specific filesystem/network process execution.
- Safety scanning itself; adapters around `EffectExecutor` apply safety at the boundary.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_engine`
- Strict check after behavior changes: `cargo clippy -p ironclaw_engine --all-targets -- -D warnings`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Keep prompt templates in files, not inline Rust strings.
- Keep Monty/CodeAct host functions resource-bounded and panic-safe.
- Prefer caller-level tests when a helper gates leases, dispatch, approvals, memory, events, or process side effects.
