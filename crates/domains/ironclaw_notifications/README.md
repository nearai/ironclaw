# ironclaw_notifications

The durable, metadata-only user notification inbox domain. It owns notification
identity, recipient scoping, lifecycle timestamps, pagination, idempotent
publication, and the filesystem-backed store contract.

- **Family / layer:** `domains` / `substrates`
- **Use this when:** publishing or mutating a user-visible Inbox record.
- **Do not use this when:** sending an external message or recording a delivery
  attempt; those remain in `ironclaw_outbound`. Product-specific projection and
  read policy remain in `ironclaw_assistant`.

Run outcome records are produced by `ironclaw_assistant` from committed process
journal transitions. Successful scheduled runs require an exact durable
finalized-reply lookup; external delivery failures remain separate from the run
outcome and come from the outbound delivery caller. The domain store does not
infer either fact.

The store persists through `ScopedFilesystem`, so composition selects the real
backend and provides tenant/user-rewritten mounts. Records contain metadata and
typed references only; prompts, replies, tool payloads, secrets, and backend
diagnostics are forbidden.

## Producer safety contract

Every producer must derive a stable id from the authoritative workflow event,
use `NotificationKind::stable_key()` for the kind segment, and preserve the
same recipient and source across retries. Kind keys are persistence vocabulary:
changing one requires a migration because otherwise a replay would create a
second record. Publication must remain best-effort with respect to the
originating workflow, while failures stay observable through sanitized logs or
the owning durable observer's retry mechanism.

Actionable records use one id for one active lifecycle and resolve that record
after verified recovery. Idempotent publication never reopens a resolved
record; only authoritative workflow reconciliation may explicitly reopen the
same actionable lifecycle, and doing so preserves its read and archive state.
Producers must not publish retry progress, transient polling states, subagent
noise, prompts, tool payloads, credentials, backend diagnostics, or any other
unbounded content. New notification kinds, source fields, actions, API shapes,
and WebUI behavior are separate contract changes; producer onboarding alone
must reuse the existing grammar.

Actions are typed metadata, not arbitrary URLs. `open_thread` is valid only
when its thread matches the record source. A separately reviewed contract
change may represent terminal facts created before a canonical thread exists
as non-actionable, but its persisted form must remain readable and mutable by
the schema-v1 rollback reader. Its legacy projection is archived so the
rollback ProductSurface never exposes the compatibility placeholder as a dead
thread link; the additive lifecycle metadata restores the real visibility for
the current reader. The schema-v1 source/action placeholder also carries a
stable distinction between a synthetic compatibility archive and a real user
archive, so a rollback rewrite cannot permanently hide or make an unarchived
record eligible for capacity eviction. A rollback reader must never receive an
actionable link for either representation.

Authentication-required sources carry the credential-authority providers from
the committed suspension. Provider-scoped recovery may settle only records
whose persisted provider set contains that provider; legacy records without
the metadata remain open until an authoritative run transition resolves them.

## Validation

```bash
cargo test -p ironclaw_notifications
cargo clippy -p ironclaw_notifications --all-targets -- -D warnings
```
