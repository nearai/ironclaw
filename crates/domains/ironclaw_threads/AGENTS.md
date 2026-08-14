# ironclaw_threads — working rules

Orientation (what this crate is, surface, deps, tests) lives in
[`README.md`](./README.md); the family boundary in
[`../AGENTS.md`](../AGENTS.md). This file is the canonical crate-local rules —
consolidated 2026-08-05 from the former `CLAUDE.md` guardrails (now a pointer)
per `docs/internal/reborn/guidance-conventions.md` rule 1.

## Invariants

- Own canonical Reborn `session_threads`, transcript message contracts,
  message ordering/status/redaction semantics, context-window reads, and the
  in-memory plus filesystem-backed durable contract stores — nothing else.
- Keep turn/run lifecycle authority out of this crate; store only stable
  turn/run references supplied by `TurnCoordinator`. Do not infer message
  status from nullable turn/run refs.
- Preserve message identity and per-thread sequence across
  redaction/deletion.
- Use policy-filtered read APIs for model-visible context; never expose raw
  secrets, host paths, raw runtime/tool payloads, or private backend
  diagnostics as ordinary transcript content.
- Serve thread lists from the declared scope/activity/thread-id projection
  with a bounded keyset cursor. Do not list the source directory, replay all
  thread rows, offset-walk the projection, or build a process-wide
  thread-list cache on requests or normal startup. Projection backfill is
  explicit migration work.
- Message and summary projections lead with `thread_id`; sequence and status
  reads bind that partition before ordering. Existing rows are repaired only
  through `migrate_transcript_indexes_for_scope`, never through a read
  fallback.
- Do not depend on product/channel adapters, raw runtime dispatchers,
  provider clients, capability execution internals, or workspace/memory
  services — the `BoundaryRule { crate_name: "ironclaw_threads" }` in
  `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`
  enforces the list. `ironclaw_safety` is deliberately permitted (validating
  provider-originated replay metadata before persistence).
- Never declare a type name `ironclaw_conversations` also declares —
  `conversations_and_threads_declare_no_name_in_common`
  (`reborn_conversations_threads_attachments.rs`) fails on the first
  collision.

## Validation

- Fast local check: `cargo test -p ironclaw_threads`
- Focused contract suites: `session_thread_contract`,
  `filesystem_session_thread_contract`, `filesystem_message_range_contract`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`

## Neighbors to read before changing behavior

- `crates/kernel/ironclaw_turns/AGENTS.md` (turn/run reference semantics)
- `crates/domains/ironclaw_conversations/AGENTS.md` (binding vs transcript —
  they own the binding, this crate owns the content)
- `crates/domains/ironclaw_memory/AGENTS.md`
