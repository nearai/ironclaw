# ironclaw_threads guardrails

- Own canonical Reborn `session_threads`, transcript message contracts, message ordering/status/redaction semantics, context-window reads, and in-memory/fake plus feature-gated durable contract stores.
- Do not depend on v1 `Agent`, v1 `SessionManager`, product/channel adapters, raw runtime dispatchers, raw provider clients, capability execution internals, or workspace/memory services.
- Keep turn/run lifecycle authority out of this crate; store only stable turn/run references supplied by `TurnCoordinator`.
- Preserve message identity and per-thread sequence across redaction/deletion; do not infer status from nullable turn/run refs.
- Use policy-filtered read APIs for model-visible context; never expose raw secrets, host paths, raw runtime/tool payloads, or private backend diagnostics as ordinary transcript content.
- Serve thread lists from the declared scope/activity/thread-id projection with
  a bounded keyset cursor. Do not list the source directory, replay all thread
  rows, offset-walk the projection, or build a process-wide thread-list cache
  on requests or normal startup. Projection backfill is explicit migration
  work.
- Message and summary projections lead with `thread_id`; sequence and status
  reads bind that partition before ordering. Existing rows are repaired only
  through `migrate_transcript_indexes_for_scope`, never through a read
  fallback.
- `thread.json` remains authoritative for thread discovery migrations. The
  `thread-index-v2` migration must physically re-encode pre-v2 index rows so
  backend-maintained ordered projections receive `scope_key`, `activity_sort`,
  and `thread_id`; a decoded-record equality no-op is not a completed backfill.
  Do not reuse an older completion marker after changing projection metadata.
- `transcript-index-v2` first materializes the bounded rc1 `message_appends`
  log into missing per-message records, retaining the log for rollback and
  letting an existing message record win so updates/redactions are never
  reversed. Only then may message, summary, and lookup projections rebuild and
  the v2 completion marker become durable.
