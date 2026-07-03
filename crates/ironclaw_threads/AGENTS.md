# Agent Map — ironclaw_threads

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for backend feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_turns/AGENTS.md`
  - `crates/ironclaw_conversations/AGENTS.md`
  - `crates/ironclaw_memory/AGENTS.md`

## What This Crate Owns

- Canonical Reborn `session_threads` and transcript service contracts.
- Thread identifiers, transcript message contracts, message ordering/status/redaction semantics, context-window reads.
- In-memory/fake stores and feature-gated durable contract stores.
- Stable turn/run references supplied by `TurnCoordinator`.

## Do Not Move In Here

- V1 `Agent`, V1 `SessionManager`, product/channel adapters, raw runtime dispatchers, provider clients, capability execution internals, or workspace/memory services.
- Turn/run lifecycle authority; this crate stores references, not lifecycle decisions.
- Raw secrets, host paths, raw runtime/tool payloads, or private backend diagnostics as ordinary transcript content.
- Product delivery policy or model/provider behavior.

## Validation

- Fast local check: `cargo test -p ironclaw_threads`
- Focused contract checks: `session_thread_contract`, `db_session_thread_contract`.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Preserve message identity and per-thread sequence across redaction/deletion.
- Use policy-filtered read APIs for model-visible context.
- Do not infer message status from nullable turn/run refs.
