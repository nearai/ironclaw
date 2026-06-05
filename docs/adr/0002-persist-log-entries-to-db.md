# Persist log entries (info+) to DB; remove in-memory ring buffer

The `/logs` page previously replayed a 500-entry in-memory ring buffer on
open, which was wiped on every process restart. We need durable log history
so users can review what happened before a restart.

**Decision:** persist `info`, `warn`, and `error` tracing log entries to the
database (both PostgreSQL and libSQL backends) via an async background writer
that subscribes to the `LogBroadcaster` broadcast channel. On page open,
`GET /api/logs/events` loads a fixed recent batch from DB, then hands off to
the live SSE stream. The in-memory ring buffer (`HISTORY_CAP = 500`) is
removed — DB is the sole source of history.

**Why info+ only:** `debug` logs include LLM request/response bodies and
internal diagnostics — high volume, rarely needed as history. They remain
visible in the live stream while the user is on the page.

**Considered alternatives:**

- *Keep both ring buffer and DB:* adds two code paths for the same data and
  requires a process-start timestamp boundary to avoid duplicates on page
  open. Removed in favour of a single source of truth.
- *Persist debug logs too:* table grows much faster with no clear benefit
  for after-the-fact review. Deferred; can be revisited if a specific audit
  use case arises.
