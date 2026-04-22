---
paths:
  - "src/**"
  - "crates/**"
---
# Gateway Events — Single Source of Truth

Every `AppEvent` reaching the SSE/WS stream must come from a **typed
source log**, or be on a small **transport-only allowlist**. Direct
`sse.broadcast(...)` / `sse.broadcast_for_user(...)` calls from tools,
handlers, or extension managers are the root cause of the UI state
drift class tracked by #2792 — the stream and the replayable source
end up telling different stories.

This is the Phase 1 rule of the gateway state-convergence epic.

## Why

When `AppEvent` has producers outside the projection layer, those
producers become a second source of truth. On SSE reconnect, replay
from the engine event log can't reconstruct them (they were never
logged). On tab focus, reconciliation against a GET endpoint can't
confirm them (no persisted state backs them). Four recent bugs
(#2654, #2534, #2731, #2079) share this shape: broadcast emitted,
backend state unchanged, UI diverges.

## Source logs

Every `AppEvent` projects from exactly one of:

| Source log | Projection function | Typical variants |
|---|---|---|
| `ironclaw_engine::EventKind` | `src/bridge/router.rs::thread_event_to_app_events` | Turn progression, tool execution, gates, leases, child threads, skills |
| Sandbox `JobEvent` | `src/worker/job.rs` (currently inline; extract under #2792 Phase 1 PR 3) | `JobStarted`, `JobMessage`, `JobToolUse`, `JobToolResult`, `JobStatus`, `JobResult` |
| Channel-lifecycle logs | `src/channels/web/features/oauth/`, `features/pairing/`, `features/extensions/`, `extensions/manager.rs` | `OnboardingState`, `ExtensionStatus` |

## Transport-only allowlist

A small number of `AppEvent` variants don't project from anything
because they have no state backing them. These are documented
exceptions, not a loophole for new state:

- `Heartbeat` — SSE keepalive, no payload, no state
- `StreamChunk` — LLM token streaming, pre-step-completion by design; formalizing into `EventKind` would pollute the durable log with token-level noise

New `AppEvent` variants that claim "transport-only" status require
review sign-off and an entry in this table.

## The rule

**No call to `SseManager::broadcast` / `SseManager::broadcast_for_user`
is allowed outside:**

1. The projection dispatcher loop that consumes one of the three source
   logs above, **or**
2. A line annotated with `// projection-exempt: <category>, <detail>`.

## Annotation format

```rust
state.sse.broadcast_for_user(user_id, event); // projection-exempt: channel-lifecycle, extension activation
```

The `<category>` must name either:

- A source log — `bridge dispatcher`, `sandbox JobEvent`, `channel-lifecycle` — plus a short detail.
- A transport-only allowlist entry — `transport-only, heartbeat` or `transport-only, stream_chunk`.
- A scheduled migration — `migrate in #NNNN` where the issue tracks moving the emit into a source log.

An unnamed category (`// projection-exempt: legacy`) is not sufficient.
Either the site is legitimately exempt and the category explains why,
or it's a violation and should be migrated.

## Enforcement

Check #9 in `scripts/pre-commit-safety.sh` (label: `PROJECTION`) flags
added lines that call `SseManager::broadcast` or
`SseManager::broadcast_for_user` without a `// projection-exempt:
<category>, <detail>` annotation on the same line. The comma is
required — the check rejects bare `// projection-exempt: legacy`. Lines
in `#[cfg(test)] mod tests` blocks and under `tests/` are skipped via
the shared `strip_test_mod_lines` filter.

The matcher covers two call-site shapes:

1. Any-receiver `.broadcast_for_user(...)` — the method is defined
   only on `SseManager` (`src/channels/web/platform/sse.rs`), so
   matching the method name alone catches same-line receivers
   (`state.sse.broadcast_for_user(...)`), rustfmt wraps
   (`state\n    .sse\n    .broadcast_for_user(...)`), and any other
   receiver name (`manager.broadcast_for_user(...)`) without
   false-positive risk.
2. `<word-boundary>sse.broadcast(...)` — the single-name `broadcast`
   is shared with the `Channel` trait, so this arm is deliberately
   narrower and only fires when the receiver is literally named
   `sse`. The boundary uses `(^|[^[:alnum:]_])` rather than `\b` so
   the check is portable across GNU and BSD `grep -E`.

## Not covered by this rule

- **`Channel::broadcast` on the `Channel` trait.** Different method,
  different trait, different semantics (delivery to a specific channel
  endpoint like Telegram, not to SSE subscribers). The `Channel` trait
  has its own invariants in `src/channels/`.
- **Non-SSE `broadcast` methods.** If you're broadcasting on a
  `tokio::sync::broadcast::Sender` directly, you're below the
  `AppEvent` abstraction; the rule doesn't apply.

## References

- Epic: #2792 — Gateway state convergence
- Coverage: #2654 — Engine→AppEvent bridge gaps
- Incidents: #2079 (SSE ordering), #2534 (stale approval), #2731 (Telegram thread split)
- Rule cluster: `.claude/rules/types.md` for wire-stable enums; `.claude/rules/tools.md` for the parallel "everything goes through tools" rule this mirrors
