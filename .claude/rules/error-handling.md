---
paths:
  - "crates/**/*.rs"
---
# Error Handling

Existing rules forbid `.unwrap()` / `.expect()` in production. The footguns below are equally dangerous and equally banned on DB, IO, workspace, and settings reads.

## Silent-Failure Anti-Patterns

- `.unwrap_or_default()` on a `Result` — collapses errors into empty state and
  masks mount, backend, migration, or schema failures.
- `.ok()?` on `Result` — drops the error entirely.
- `let Ok(x) = ... else { return None }` / `else { return }` — same shape, structured.
- `if let Err(e) = ... { warn!(...) }` followed by caching, inserting, or
  continuing — poisons downstream state with a half-initialized value.
- `.map_err(|_| OtherError)` — discards the cause. A sanitized
  `RebornServicesError` may hide details from the client, but the server-side
  chain must retain/log the source. Use a cause-preserving constructor such as
  `RebornServicesError::internal_from`, or log the bound source before mapping.

**Required pattern — fail loud by default:**

```rust
let projects = store.list_projects(&owner_id).await?;
```

**When fallback is genuinely acceptable** — must be justified inline and name the operation:

```rust
let cached = optional_cache.refresh().await.unwrap_or_default(); // silent-ok: optional cache refresh, authoritative read follows
```

Review flag: added lines containing `unwrap_or_default()`, `.ok()?`, or `else { return` / `else { return None }` on a DB/IO/workspace/boundary call must carry a `// silent-ok: <reason>` comment or be rejected.

A `map_err(|_| …)` (a closure discarding the error binding) is **not** `silent-ok`-exemptible — a comment does not make the dropped cause reappear. Fix it by carrying the cause (`.map_err(ErrorType::constructor)` / `RebornServicesError::internal_from`) or by logging the bound error before mapping. Reject the line otherwise.

## Persist-Then-Reload Atomicity

A write that triggers a runtime rebuild (provider chain reload, settings reload, credential reinjection) is multi-step. The DB row may commit while the rebuild fails — do NOT leave split-brain state.

Two acceptable patterns:

- **Pre-validate** — attempt the rebuild on the new value *without persisting*; only persist on success.
- **Snapshot + rollback** — snapshot the old value, write, attempt rebuild; on rebuild failure, restore the snapshot and return the error.

Test both the failed rebuild and the rollback/pre-validation path through the
product-facing settings caller.

## Error boundaries at product and transport edges

No internal identifier, traceback, or transport error may cross a channel boundary to the user. Map at the source:

- `LlmError::BadGateway` / raw HTTP 5xx → "provider temporarily unavailable"
- `LlmError::ContextOverflow` / HTTP 413 → "message too large — summarizing" (every direct-HTTP provider must detect 413)
- Filesystem / workspace errors → "can't access your workspace file" (never expose paths)
- Runtime adapter/process failures → a stable sanitized category plus opaque
  invocation ID for correlation.

Forbidden in user-facing output: raw provider bodies, stack traces, absolute host
paths, mount internals, credential material, raw runtime output, debug wire
formats, or literal escaped framing. Product errors expose stable codes/messages;
logs and durable events still pass through their redaction contracts.
