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

## Validation

```bash
cargo test -p ironclaw_notifications
cargo clippy -p ironclaw_notifications --all-targets -- -D warnings
```
