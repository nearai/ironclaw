---
paths:
  - "src/**/*.rs"
  - "crates/**/*.rs"
---
# Error Handling

Existing rules forbid `.unwrap()` / `.expect()` in production. The footguns below are equally dangerous and equally banned on DB, IO, workspace, and settings reads.

## Silent-Failure Anti-Patterns

- `.unwrap_or_default()` on a `Result` — collapses errors into empty state. Masks DB outages, migration failures, schema drift. (#2526 `list_projects`, #2653 `.env` scan.)
- `.ok()?` on `Result` — drops the error entirely.
- `let Ok(x) = ... else { return None }` / `else { return }` — same shape, structured.
- `if let Err(e) = ... { warn!(...) }` followed by caching / inserting / continuing — poisons downstream state with a half-initialized value and hides the failure forever. (#2633 `seed_if_empty` cache.)

**Required pattern — fail loud by default:**

```rust
let projects = store.list_projects(&owner_id).await?;
```

**When fallback is genuinely acceptable** — must be justified inline and name the operation:

```rust
let rows = store.list_agent_jobs().await.unwrap_or_default(); // silent-ok: dashboard refresh, next poll retries
```

Review flag: added lines containing `unwrap_or_default()`, `.ok()?`, or `else { return` / `else { return None }` on a DB/IO/workspace call must carry a `// silent-ok: <reason>` comment or be rejected.

## Persist-Then-Reload Atomicity

A write that triggers a runtime rebuild (provider chain reload, settings reload, credential reinjection) is multi-step. The DB row may commit while the rebuild fails — do NOT leave split-brain state.

Two acceptable patterns:

- **Pre-validate** — attempt the rebuild on the new value *without persisting*; only persist on success.
- **Snapshot + rollback** — snapshot the old value, write, attempt rebuild; on rebuild failure, restore the snapshot and return the error.

Reference: PR #2673 `reload_llm_after_settings_change`.

## Error Boundaries at the Channel Edge

No internal identifier, traceback, or transport error may cross a channel boundary to the user. Map at the source:

- `LlmError::BadGateway` / raw HTTP 5xx → "provider temporarily unavailable"
- `LlmError::ContextOverflow` / HTTP 413 → "message too large — summarizing" (every direct-HTTP provider must detect 413)
- Filesystem / workspace errors → "can't access your workspace file" (never expose paths)
- Orchestrator worker tracebacks → "internal task failed" + opaque job id for correlation

Forbidden in user-facing output: raw 5xx codes, Python tracebacks, absolute paths (`/workspace/...`, `/home/...`), internal file names (`.system/`, `AGENTS.md`, `HEARTBEAT.md`, `BOOTSTRAP.md`), wire-format prefixes (`message{content:`, literal `\n`). References: #2546, #2407, #2408, #2489, #2584.
