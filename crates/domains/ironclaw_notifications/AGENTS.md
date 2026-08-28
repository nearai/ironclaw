# ironclaw_notifications — working rules

- Own durable user Inbox record grammar and lifecycle invariants only.
- Store metadata and typed references only; never persist message bodies,
  prompts, tool inputs/outputs, secrets, host paths, or backend diagnostics.
- Keep read, resolved, and archived timestamps orthogonal.
- Producers publish terminal facts initially resolved; actionable records are
  initially open and the originating workflow resolves them when they settle.
- Idempotent publication never reopens a resolved record. Authoritative
  workflow reconciliation must explicitly reopen the same actionable lifecycle;
  that recipient-scoped mutation preserves read and archive state.
- Publication is idempotent by stable notification id for every record the
  snapshot still holds; conflicting reuse of a held id fails closed. That window
  is finite, not unbounded — see the bound below — so a producer must not treat
  an arbitrarily delayed retry as guaranteed deduplication.
- Recipient scope is mandatory on every read and mutation.
- Retention is explicit: never delete unread or unarchived records to make room.
- The record bound is configuration the constructing caller states rather than a
  constant compiled into the store: production states
  `NOTIFICATION_INBOX_MAX_RECORDS`, and contract tests state a small one so the
  behaviour below is provable without re-encoding a full snapshot thousands of
  times.
- That bound is 1,000 records in production, and it *is* the idempotency
  window: a publish at the bound reclaims records that are both resolved and
  archived, oldest first, and a later retry for a reclaimed id is admitted as a
  new record rather than recognised as a duplicate. When nothing is closed the
  publish fails instead of evicting live state, so a full inbox of open gates is
  never silently thinned. Widening the window means retaining durable
  deduplication state for reclaimed ids, which is a persisted-schema change and
  needs its own rollback review.
- A publish drains to the active bound, it does not shed one record per call, so
  a bound lowered under an existing snapshot converges instead of staying over
  it forever. Reads never reject an over-bound snapshot: validation guards
  against corruption with the absolute product ceiling, deliberately not with
  the configured bound, because rejecting there would turn lowering the
  configuration into a locked-out recipient.
- Persistence uses `ScopedFilesystem` plus bounded CAS; backend selection stays
  in composition.
- Notification production and product read policy belong to the originating
  workflow in `ironclaw_assistant`; external delivery belongs to
  `ironclaw_outbound`.

## Validation

- `cargo test -p ironclaw_notifications`
- `cargo clippy -p ironclaw_notifications --all-targets -- -D warnings`
- `cargo test -p ironclaw_architecture_tests`
