# Web Debug Inspector Contract

The Web Debug Inspector is an opt-in, operator-only view of the host evidence
for one conversation run. It explains what the host actually sent to the
model, the ordered model/tool lifecycle, and aggregate run statistics without
putting verbose diagnostic content into the normal chat event stream.

## Activation and authorization

- The browser enables the panel only when the chat URL contains
  `?debug=true`. Ordinary chat routes and layouts remain unchanged otherwise.
- Every inspector HTTP and SSE route requires both an authenticated caller
  with operator configuration authority and a deployment that exposes the
  operator configuration surface. Either missing gate returns `403` before a
  product query is dispatched.
- The product service derives tenant and user from `ProductSurfaceCaller`; the
  URL supplies only thread, run, and activity identifiers. Reads are keyed by
  `(tenant_id, user_id, thread_id, run_id)` and tool detail adds the exact
  `activity_id`. A mismatched component returns no diagnostic record.
- Diagnostic state is process-local and best effort. It is not persisted and
  disappears on restart or bounded eviction.

## Read surfaces

| Surface | Route | Content |
| --- | --- | --- |
| Snapshot | `GET /api/webchat/v2/operator/inspector/threads/{thread_id}/runs/{run_id}` | Bounded prompt metadata, ordered activity, model/tool summaries, statistics |
| Prompt | `GET …/{run_id}/prompt` | Bounded host-resolved prompt components and reconstructed prompt |
| Tool detail | `GET …/{run_id}/tools/{activity_id}` | Bounded, sanitized arguments and result for one exact activity |
| Updates | `GET …/{run_id}/events` | SSE lifecycle/stat updates with resumable cursors |

The update stream never includes prompt component bodies, tool arguments, or
tool results. A tool update may include stable identifiers, capability name,
status, duration, output byte count, and whether its retained result was
truncated. The browser requests verbose tool data only after the operator
expands that activity.

## Bounds and sanitation

- Prompt component content: 64 KiB each, 256 KiB total, at most 128
  components; reconstructed prompt: 256 KiB.
- Tool arguments: 64 KiB; tool result: 50 KiB. Original byte length and
  truncation are explicit.
- Per run: 128 retained model calls, 16 retained tool executions, 1,000
  activity entries, and 1,024 retained updates.
- Process defaults: eight caller sessions and two runs per session, all with
  deterministic bounded eviction.
- Prompt, model, tool, failure, and summary text crosses control-character
  sanitation and secret scanning before retention. Diagnostic text types
  validate UTF-8-safe bounds and reject inconsistent size metadata.
- Capture types that may contain raw provider input or output do not implement
  `Debug`. Diagnostic capture is best effort and must not alter capability or
  conversation success.

## Ordering and reconnect behavior

Each retained run has a generated stream id and monotonically increasing
sequence. The SSE `id` is `{stream_id}:{sequence}`. A reconnect supplies
`Last-Event-ID`; the server either continues after that cursor or emits one
`diagnostic_rebase` event when the cursor is stale, from another stream
generation, or older than retained history. The browser then refetches the
snapshot and rejects duplicate or out-of-order cursors.

Every connection first receives a data-free, id-free `diagnostic_connected`
handshake so an idle or fully caught-up run is reported as live without
waiting for the next lifecycle update. The handshake cannot advance the resume
cursor.

The activity view orders authoritative events by sequence. It can show local
product lifecycle hints while waiting for host diagnostics, but replaces those
hints when the matching authoritative event arrives. A started model or tool
activity remains pending until its correlated terminal event arrives.

Turn navigation is session-local. The browser retains at most 32 observed run
ids per thread in `sessionStorage`, selects the active/latest run by default,
and lets the operator move to the previous, next, or latest observed turn. It
does not create a durable run index.

The Stats view also reports browser-observed stream state, reconnect attempts,
accepted diagnostic-update count, and the last accepted update time. These
values are tab-session diagnostics, not server accounting. They are bounded in
`sessionStorage`, retain cursors for at most 32 observed runs, and reject
duplicate or backwards cursors when a run is revisited or the page reloads.

## Failure behavior

Inspector failures do not disable chat. The panel renders explicit loading,
forbidden, unavailable, reconnecting, disconnected, truncated, and evicted
detail states. Concurrent diagnostic streams share the normal per-caller SSE
capacity gate and return `429` when the configured limit is exhausted. Each
browser tab uses a bounded connection id and monotonic generation so a reload
or run change supersedes its prior inspector stream instead of consuming an
additional slot.

## Verification

- Contract and store tests cover caller/operator gates, missing caller
  context, cross-scope reads, redaction, bounds, sequence/rebase behavior, and
  the absence of verbose tool data in updates.
- Frontend tests cover activation, responsive layouts, preferences, bounded
  reducers, pending states, cursor deduplication, and dedicated detail lookup.
- Playwright scenarios cover ordinary-chat isolation, desktop/tablet/mobile
  behavior, prompt/activity/stat rendering, multi-turn navigation, reload
  reconnect, tool detail expansion, and the 50 KiB result limit.
