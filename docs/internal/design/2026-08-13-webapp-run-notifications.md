# Web-app run-completion notifications

**Date:** 2026-08-13
**Status:** Approved design
**Grounded in:** `main` @ `93522b714`
**Extends:** [2026-08-08-web-push-notifications.md](2026-08-08-web-push-notifications.md)
**Independent of:** `docs/agent-execution-design` and PR #7562

## Summary

When a user-visible, top-level agent run completes successfully and its final
assistant reply is durable in thread `T`, IronClaw publishes one metadata-only,
owner-scoped completion fact. A typed user-completion logical stream delivers
that fact over one app-wide, read-only session WebSocket per authenticated page,
regardless of the SPA route. The same socket multiplexes the existing thread
projection streams; it does not carry product commands. Browser clients offer
short-lived presentation intents over authenticated HTTP based on their actual
focus, route, visibility, reply-render evidence, notification permission, and
enrollment state. A durable server coordinator grants exactly one responding
browser profile the highest-priority presentation or, when no browser can
present, falls back to an authorized Web Push through the existing `web-app`
`ChannelDelivery` surface.

This is a hybrid design:

- the **runtime** owns the durable completion fact;
- the **product notification coordinator** owns replay, arbitration, read state,
  and the decision to attempt Web Push;
- the **product event interface** owns typed logical stream selectors and
  independent resume cursors;
- the **WebUI session event gateway** multiplexes those streams over one
  read-only WebSocket and renders the browser-safe wire schema;
- the **service worker** owns same-browser multi-tab inspection and the final
  local choice between suppressing, an in-app surface, and an OS surface;
- the **outbound policy and web-app extension** own authorization, delivery
  attempts, enrollment resolution, VAPID-mediated egress, and endpoint pruning.

The coordinator never treats stream subscription count as push authority.
Subscriber access and outbound push selection remain separate, as required by
the event/projection contract. The design prefers an occasional duplicate over
a missed completion: presentation is at-least-once, while stable notice IDs,
CAS transitions, a service-worker ledger, per-thread notification tags, and
read evidence suppress ordinary replay and race duplicates.

Thread creation, turn submission, cancellation, retry, gate resolution,
notification intent, presentation acknowledgement, and read evidence all stay
on authenticated HTTP routes backed by `ProductSurface::invoke`. The WebSocket
accepts only bounded subscription-control frames. It is an event transport, not
a command bus.

## 1. Product behavior

### 1.1 Included completions

A completion notice is eligible only when all of the following are true:

1. the process is a top-level, thread-backed, user-visible agent turn;
2. its terminal state is successful `Completed`;
3. the thread has a finalized assistant message for that exact run;
4. the owner user can still view the thread through the ordinary product
   access policy; and
5. the notice has not already been marked read from reply-render evidence.

Subagent processes, detached executions, system inference, failed runs,
cancelled runs, and internal maintenance processes do not create completion
notices. A completion may originate from WebUI input, an automation, or another
conversation ingress as long as the resulting top-level thread is visible to
the owner in the web app. “Web-app only” describes the notification surface,
not the run's ingress source.

### 1.2 Required presentation

For user `U`, run `R`, and thread `T`:

1. If any responding browser profile has a focused, visible tab rendering `T`
   and that tab confirms `R`'s finalized reply rendered, show nothing.
2. Otherwise, if a responding browser profile has a focused, visible tab,
   show one in-app toast/badge in one deterministic tab. Clicking navigates to
   `/chat/T`.
3. Otherwise, if a responding browser profile has open tabs, the web-app
   notification target is selected, and the selected profile has granted
   permission and an enrolled push registration, show one browser/OS
   notification through `ServiceWorkerRegistration.showNotification`.
4. Otherwise, if no browser can present before the arbitration deadline and
   the web-app target is selected with at least one enrollment, attempt Web
   Push. A notification click focuses an existing tab if one exists and opens a
   new app window only when none exists.
5. If OS/push is unavailable, retain the unread completion and show it in-app
   on the next authenticated app session. Never request permission
   automatically.

Focused-on-`T` is not by itself read evidence. If the completion fact outruns
the thread stream, the coordinator gives that tab the arbitration window to
confirm the final reply rendered. If it cannot, IronClaw presents a notice
rather than silently treating unseen content as seen.

### 1.3 Delivery guarantees

- One immutable completion fact exists per eligible run.
- One browser-profile presentation winner is granted at a time.
- Same-profile tabs do not independently present.
- A browser effect and its server acknowledgement cannot be atomic. If a
  browser presents and then loses its acknowledgement, a retry may duplicate;
  local notice IDs and OS tags collapse the normal case.
- Cross-device presentation is best-effort at-most-one within the bounded
  arbitration window. A sleeping device may receive an already-enqueued push
  after another device reports the reply read; the next stream/push wake clears
  it. Globally atomic focus/read state would require long-lived distributed
  presence leases and is deliberately not part of this design.
- A push service's 2xx response proves transport acceptance, not display or
  user receipt.

## 2. What already ships

The 2026-08-08 design is historical prior art, not an accurate inventory of
current `main`. PR #7398 and the channel normalization in PR #7477 shipped most
of its transport foundation, then renamed the product from `web-push` to
`web-app`:

- `crates/extensions/packages/web-app/` is a bundled first-party extension.
  Its manifest declares authenticated-session ingress, host-owned stream
  replies, push delivery, enrollment, VAPID credential mediation, and exact
  push-service egress hosts.
- `ironclaw_web_app` owns subscription grammar, RFC 8291 payload encryption,
  VAPID request construction, and the host-owned enrollment store.
- The SPA registers `/sw.js` at boot. The worker already handles `push` and
  `notificationclick`, validates same-origin paths, focuses/navigates an
  existing client, and calls `openWindow` only when no client exists.
- Permission and enrollment are already explicit user actions. The app never
  requests notification permission at load.
- The adapter prunes registrations after 404/410 push responses. Host-side
  VAPID injection keeps credentials out of the extension.
- The web-app target is a notification target but not a final-reply target:
  `final_replies = false`, `notifications = true`.

Important gaps remain:

- completion notices are intentionally disabled; the background-run notifier
  sends gate/auth/failure notices, not results;
- the adapter renders arbitrary `OutboundPart::Text` into the OS body, uses the
  hard-coded `/automations` URL, and sends no notification tag;
- the existing notification center polls approval state every ten seconds;
- the current SSE route is thread-scoped and its client disconnects while the
  page is hidden;
- the generic `ProductSurfaceStreamRequest.stream_id` is an optional string
  that the assistant facade currently interprets as exactly one thread ID;
- product stream envelopes are erased to `serde_json::Value` and decoded back
  into the same typed envelope at the WebUI seam;
- SSE and the dormant per-thread WebSocket duplicate subscription/drain logic,
  while the WebSocket serializes raw `ProductOutboundEnvelope` routing metadata
  instead of the browser-safe `WebChatV2EventFrame` required by the WebUI
  contract; and
- the dormant per-thread `progress` push plan is not a production completion
  path and must not be revived for this feature.

The shipped stack therefore removes the need to design cryptography,
enrollment, endpoint storage, restricted egress, or basic click handling. The
new work is a durable completion projection, presentation arbitration, safe
payload rendering, precise scoping, and a deeper product-event transport seam.

### 2.1 Current thread streaming pipeline and assessment

The current chat stream has a strong execution core:

1. the SPA creates a thread and submits a turn through authenticated HTTP;
2. the inbound product surface applies ownership, idempotency, attachment
   bounds, and turn admission before `TurnCoordinator` schedules execution;
3. providers emit text deltas into `ProviderStreamSink`, which accumulates and
   sanitizes the complete text-so-far rather than forwarding raw tokens;
4. `ModelTextDelta` milestones feed a 16 ms coalesced, process-local live
   projection; partial text is deliberately not durable transcript state;
5. `EventStreamManager` supplies authorization, snapshot/replay, rebase,
   redaction validation, bounded buffering, and lag behavior;
6. the WebUI serves the thread projection over
   `/api/webchat/v2/threads/{thread_id}/events`; and
7. the SPA replaces the active assistant phase with cumulative text, then
   upgrades it when the completed-turn projection loads the exact durable
   finalized assistant message.

That split between ephemeral presentation text and a durable finalized reply is
correct and remains unchanged. The weakness is above `EventStreamManager`:
thread runtime, live, and turn-lifecycle sources are manually combined behind a
shallow string selector; the WebUI erases and restores types; transport loops
are duplicated; and React owns connection, retry, route, history, and several
ordering races. This proposal deepens that seam rather than replacing the
runner, model gateway, transcript, or projection machinery beneath it.

## 3. Design principles

1. **A completion is a fact, not a transport event.** The process/turn journal
   is authoritative. Session events, in-app UI, OS notifications, and Web Push
   only project or present it.
2. **Browser state is sampled, not persisted as standing presence.** Tabs
   report focus/route/visibility only as short-lived intents for a specific
   notice. The server does not retain a general browsing-history or heartbeat
   table.
3. **Display is not read.** A toast or OS notification may be presented while
   unread. Read requires exact reply-render evidence or a subsequent focused
   visit to the thread after the final reply is present.
4. **Streams do not authorize pushes.** A missing session-event subscriber
   neither enables nor selects Web Push. The durable arbitration state and
   configured web-app target do.
5. **Web-app-only is structural.** A new target capability selects only the
   `web-app` delivery target. The generic coordinator never branches on Slack,
   Telegram, or extension IDs.
6. **Lock-screen content is public presentation.** Encryption protects the
   payload in transit, not after the browser renders it.
7. **Recovery is replay plus CAS, not polling.** One boot reconciliation and
   timer-driven due work recover pending notices. The client never polls for
   completion.
8. **Commands stay on HTTP.** A WebSocket client may subscribe, unsubscribe,
   resume, or exchange transport heartbeats. It may not create threads, submit
   turns, cancel/retry runs, resolve gates, submit notification intents, mark
   notices read, or invoke any other product mutation.
9. **One transport does not imply one ordering.** The session WebSocket
   multiplexes independent logical streams. Each selector retains its own
   authorization, snapshot, replay, lag, and cursor contract; thread events and
   user-completion events never share a synthetic global cursor.
10. **Preserve types until browser rendering.** The product interface returns
    typed event envelopes. One WebUI stream driver maps them to a redacted
    browser frame and is reused by the session WebSocket and the temporary
    legacy SSE adapter.

## 4. Presence primitives and their limits

No single primitive answers “where is the user?” The decision uses the
intersection below.

| Primitive | Scope | What it proves | What it does not prove |
|---|---|---|---|
| `document.visibilityState` | One tab | Whether that document is visible or hidden | Whether it owns focus or which route another tab renders |
| `document.hasFocus()` plus `focus`/`blur` | One tab | Whether the document currently owns focus | Durable presence; the value can change immediately after sampling |
| SPA router state | One tab | Which route has committed and which thread the UI intends to render | That the final reply has rendered |
| `reply_observed(run_id)` from the thread view | One tab | The exact finalized reply is present in the rendered transcript | That the user cognitively read it |
| `clients.matchAll({type: "window", includeUncontrolled: true})` | One service-worker registration | Which same-origin windows exist, plus each client's URL, visibility, and focus hints | Tabs in another browser profile or device; exact DOM render state |
| `Notification.permission` | Browser profile | Whether the browser may show an OS notification | Whether a usable server enrollment exists |
| Push registration/enrollment | User plus browser profile | Whether the host can address that browser through Web Push | That the browser is online, the app is closed, or a notification will display |
| Session event connection | One authenticated page | A transport is currently attached and may carry several logical streams | Notification permission, view state, or authorization to push |

Focus and visibility are inherently racy. The design narrows the race by
putting the final local check in the service worker immediately before a
browser effect and by attaching a monotonically increasing browser state
revision to every intent and grant. It does not claim a stronger guarantee
than the platform provides.

## 5. Recommended architecture: bounded intent/grant hybrid

### 5.1 Components and ownership

| Component | Owner | Responsibility |
|---|---|---|
| Completion observation adapter | `ironclaw_assistant` | Converts an authoritative successful top-level turn commit plus finalized reply into one idempotent notice |
| Run-completion notice store | `ironclaw_assistant` | Persists product notification, arbitration, presentation, push-ownership, and read state on `ScopedFilesystem` |
| User completion projection | event projection/stream boundary | Produces authorized, redacted snapshots and replay/live updates from the notice store |
| Notification coordinator | `ironclaw_assistant` | Opens the intent window, ranks intents, grants one presenter, recovers expired grants, and requests push fallback |
| Product event interface | `ironclaw_product_contracts` | Defines typed logical stream selectors, typed envelopes, independent opaque cursors, and the frozen `ProductSurface::stream_events` operation |
| Product operations | frozen `ProductSurface` methods | Streams typed events; accepts intents, acknowledgements, and thread-read evidence over HTTP; queries bounded unread state |
| Session event gateway | `ironclaw_webui` | Authenticates one app-wide WebSocket, drives logical subscriptions, enforces connection/subscription budgets, and renders browser-safe frames through one shared codec |
| Session event client | SPA root | Owns one socket per authenticated page, reconnects it, tracks a cursor per logical subscription, and exposes transport-independent event subscriptions to route hooks |
| Tab state reporter | SPA | Reports route, focus, visibility, and exact reply-render state to the service worker |
| Presentation arbiter | service worker | Inspects all same-origin clients, dedupes a notice, proposes one profile-level intent, applies grants, and clears OS notifications |
| Push policy and attempts | `ironclaw_event_streams` plus `ironclaw_outbound` | Re-authorizes projection access and target, creates a stable attempt, and preserves crash semantics |
| Web Push renderer/transport | `web-app` `ChannelDelivery` | Maps a typed completion notice to a fixed safe payload and sends it to host-resolved registrations |

The composition root wires these owners. It contains no completion policy.

### 5.2 Durable source and observer

Current `main` already has a replaying `ProcessJournalCommitObserver` mechanism
with a durable observer cursor. Add a product-owned `RunCompletionObserver`
through that port rather than adding another run-state poller.

For every committed process batch, the observer:

1. filters to terminal `Completed` top-level agent-turn processes;
2. requires an owner user and a WebUI-visible thread scope;
3. resolves the finalized assistant message by the exact run ID;
4. derives a stable, purpose-separated `RunCompletionNoticeId` from the
   owner scope, run ID, and `web-app-run-completion/v1` label;
5. writes the notice idempotently; and
6. returns success only after the notice write is durable, allowing the
   process observer cursor to advance.

A transcript-store backend error is a retryable observer failure. The current
turn contract finalizes the assistant message before `Completed`; if a
successful commit nevertheless resolves authoritatively to no final message,
the observer records a sanitized anomaly metric, advances, and creates no
notice. One malformed historical run must not wedge the shared durable observer
cursor forever. Duplicate journal delivery rewrites nothing and wakes the
existing notice.

The observer stores no reply text. It stores only typed identities, timestamps,
the terminal projection reference, and state-machine fields.

### 5.3 Durable notice state

The store is product-notification state owned by `ironclaw_assistant`, not
push-transport state owned by `ironclaw_web_app`: in-app notices exist even
when Web Push is disabled. It uses new versioned records under a dedicated
per-user notification mount. This is additive filesystem data, with additive
ordered indexes created through `ensure_index`; it requires no relational
migration, no rewrite of existing enrollment records, and no backend branch in
the store wrapper. Composition selects the backend and mount; the web-app
domain remains responsible only for enrollment and Web Push mechanics.

```rust
struct RunCompletionNotice {
    notice_id: RunCompletionNoticeId,
    scope: TurnScope,
    owner_user_id: UserId,
    run_id: TurnRunId,
    terminal_projection_ref: ProjectionUpdateRef,
    completed_at: Timestamp,
    delivery: CompletionDeliveryState,
    read: CompletionReadState,
    updated_at: Timestamp,
}

enum CompletionDeliveryState {
    PendingArbitration { closes_at: Timestamp },
    Granted {
        grant_id: CompletionGrantId,
        browser_instance_id: BrowserInstanceId,
        surface: CompletionSurface,
        state_revision: u64,
        expires_at: Timestamp,
    },
    Presented { surface: CompletionSurface, presented_at: Timestamp },
    PushOwned { delivery_id: OutboundDeliveryId, claimed_at: Timestamp },
    NoExternalTarget { settled_at: Timestamp },
}

enum CompletionSurface {
    NoSurfaceWatchingThread,
    InApp,
    LocalOs,
    WebPush,
}

enum CompletionReadState {
    Unread,
    Read { read_at: Timestamp, evidence: CompletionReadEvidence },
}

enum CompletionReadEvidence {
    ReplyRendered { browser_instance_id: BrowserInstanceId },
    FocusedThreadVisit { browser_instance_id: BrowserInstanceId },
}
```

Delivery and read are orthogonal. `Presented(InApp)` remains unread until the
thread is viewed. A read transition may settle a pending or granted notice and
prevents future presentation, but it does not delete or rewrite the immutable
completion fact.

Every transition is a bounded CAS update. The critical ownership race is:

```text
PendingArbitration
  |-- read evidence ------------------------------> Read / settled
  |-- best intent at deadline --------------------> Granted
  |-- no eligible intent + push target available -> PushOwned
  `-- no eligible intent + no target -------------> NoExternalTarget

Granted
  |-- matching effect acknowledgement -----------> Presented
  |-- matching reply-render acknowledgement ------> Read / settled
  `-- grant expiry -------------------------------> PendingArbitration
```

Only a `PendingArbitration` record can become `PushOwned`; only one replica can
win that CAS. Once push ownership is acquired, ordinary outbound attempt
semantics decide whether egress is prepared, sending, delivered, failed, or
unknown. A late browser intent cannot recall possible provider egress.

Records are retained with timestamps. Bounded in-memory indexes, timer queues,
and browser caches may evict entries; durable completion and state records are
never deleted as “cleanup.”

### 5.4 Arbitration timing and recovery

P0 uses host constants, not user-facing knobs:

- intent collection window: **1 second**;
- presentation-grant acknowledgement timeout: **2 seconds**;
- one grant-expiry re-arbitration before push fallback; and
- maximum unread snapshot returned to one client: **250 notices**, grouped by
  thread in the UI.

All adversary-controlled collections are bounded at their admission seam:

- at most **32 browser-profile intents** per notice, with a newer revision
  replacing the prior intent for the same profile;
- at most **128 recent observed run IDs** in one tab-state message;
- the shipped maximum of **20 delivery registrations** per user/channel; and
- bounded opaque identifier and serialized input sizes in the product command
  schemas.

The service-worker notice ledger and UI cache retain at most 250 active notice
IDs; eviction affects only cache acceleration because the durable projection
remains authoritative.

The one-second window is long enough for same-origin tabs and ordinary nearby
devices to respond without making a closed-app push feel delayed. These values
may move into `DeploymentConfig` after telemetry justifies tuning; they do not
become Cargo features or environment-only behavior.

At startup, the coordinator performs one bounded reconciliation over pending,
expired-grant, and push-owned records, then schedules the next `closes_at` or
`expires_at` with timers. New notice writes wake the due queue. There is no
steady interval scan and no client polling. If the queue overflows, the writer
fails retryably before acknowledging the process observer cursor.

### 5.5 Browser-profile coordination

Each tab creates an opaque random `tab_id`. Each enrolled browser profile has
an opaque `browser_instance_id` correlated host-side with its delivery
registration; neither identifier contains account or route data.

Every authenticated page owns one app-root `SessionEventClient` and therefore
at most one physical session WebSocket. A page may subscribe to its active
thread, the owner-scoped run-completion stream, and other read-only logical
streams without opening another connection. Tabs do not elect a shared socket
leader: sharing one credentialed connection across tabs would add a second
request-routing protocol and make chat delivery depend on a hidden leader.

Duplicate run-completion events from several tabs are expected. Tabs coordinate
notification presentation through `BroadcastChannel` and the service worker,
and the worker's IndexedDB test-and-set ledger collapses them by `notice_id`.
The server accepts at most one current intent per notice and
`browser_instance_id`, so duplicate page connections do not become duplicate
profile candidates. Pages beyond the per-caller connection budget degrade to
the durable unread snapshot and Web Push fallback; they do not weaken
arbitration correctness.

On route commit, focus, blur, visibility change, reply render, and page exit,
each tab posts a state update to the worker:

```ts
type TabNotificationState = {
  tabId: string;
  stateRevision: number;
  route: { kind: "thread"; threadId: string } | { kind: "other" };
  visibility: "visible" | "hidden";
  focused: boolean;
  observedRunIds: string[]; // at most 128 recent IDs
};
```

The worker combines these reports with a fresh `clients.matchAll` snapshot.
It creates at most one intent per notice and browser profile. An authenticated
page submits that intent through HTTP; the worker never receives session
credentials and the socket never carries the mutation.

### 5.6 Intent ranking and grant application

Intents are notice-specific, short-lived, and server-stored only within the
notice record's arbitration history. They are not a reusable presence API.

Priority, highest first:

1. `reply_observed` — a focused, visible tab on `T` confirms `R` rendered;
2. `watching_thread` — a focused, visible tab on `T` expects render before the
   window closes;
3. `in_app` — a focused, visible tab not demonstrably showing `R`;
4. `local_os` — clients exist but none is focused, and permission,
   registration, and current web-app target configuration permit OS display;
5. `unavailable` — this profile cannot present now.

For conflicting focus claims, a tab on `T` outranks a tab elsewhere. Among
equal candidates, choose the greatest state revision, then the newest focus
epoch, then lexicographically smallest opaque browser/tab IDs. The stable final
tie-break makes replay deterministic without encoding product meaning in IDs.

The server waits for the intent window before issuing a grant so a slower
`reply_observed` intent can suppress a lower-priority notification on another
device. A grant names exactly one `browser_instance_id`, surface, state
revision, and expiry. Every run-completion logical subscription may receive the
grant; only the named worker applies it, and only if its current state revision
is not newer and incompatible.

Immediately before applying a grant, the worker re-runs `clients.matchAll`:

- an `InApp` grant is posted to one focused tab;
- a `LocalOs` grant calls `registration.showNotification` with the typed
  payload and thread tag;
- a `NoSurfaceWatchingThread` grant waits for/validates exact reply render and
  sends read evidence; and
- stale grants are rejected with `stale_state`, causing one re-arbitration.

The authenticated page acknowledges over HTTP only after the worker reports
that the effect succeeded. This avoids the failure mode where a server
suppresses fallback for a surface that never appeared. Lost acknowledgements
may duplicate but cannot silently lose the notice.

## 6. Definitive state, signal, and decision matrix

### 6.1 Core matrix

`Selected` means the user's effective `outbound.notification_channels_set`
contains a live target whose `run_completions` capability is true. `Enrolled`
means the chosen browser profile has a usable host-owned delivery registration.

| State at arbitration | Browser signals | Permission / configuration | Profile intent | Server decision | User-visible result |
|---|---|---|---|---|---|
| Exact reply already rendered in focused `T` | focused + visible + route `T` + `observedRunIds` contains `R` | Any | `reply_observed` | Mark read; settle without presentation | Nothing; clear existing surfaces for `T` |
| Focused and on `T`, reply arrives inside window | focused + visible + route `T`; later exact render ACK | Any | `watching_thread`, then read ACK | Grant no-surface lease; settle on ACK | Nothing |
| Focused and on `T`, reply does not render | focused + visible + route `T`; no render ACK before expiry | Any | `watching_thread` expires | Re-arbitrate; focused tab becomes `in_app` safety winner | One in-app notice; the reply was not demonstrably seen |
| One focused tab elsewhere | focused + visible + route not `T` | Any | `in_app` | Grant that profile/tab | One toast and unread badge |
| Several tabs; one focused on `T` with reply rendered | worker sees all clients; exact tab supplies render evidence | Any | `reply_observed` outranks all | Settle read | No notification anywhere in that browser; winning intent suppresses other responding profiles too |
| Several tabs; none on `T`, one focused | one focus winner, routes differ | Any | one profile-level `in_app` | Stable tab tie-break | Exactly one in-app notice |
| Browser reports two focused tabs, one on `T` | inconsistent focus hints; one exact route/render match | Any | route/render match wins | Settle read or grant no-surface lease | No duplicate |
| Browser reports two focused tabs, neither on `T` | inconsistent focus hints | Any | `in_app` with revision/focus epoch | Stable tie-break | One in-app notice |
| Tabs exist, none focused | `clients.matchAll` nonempty; all focus hints false | `granted` + Selected + Enrolled | `local_os` | Grant one profile | One OS notification through `showNotification` |
| Tabs exist, none focused | same | permission `default` | `unavailable` | Do not request permission; if another profile cannot present, settle no external target or push elsewhere | Unread in-app item on next focus |
| Tabs exist, none focused | same | permission `denied` or unsupported | `unavailable` | Never re-prompt; no local OS grant | Unread in-app item on next focus |
| Tabs exist, none focused | same | not Selected or not Enrolled | `unavailable` | No OS grant and no web-app push | Unread in-app item on next focus |
| No tab/browser responds by deadline | no intent | Selected + at least one enrollment | none | CAS to `PushOwned`; authorize and attempt web-app delivery | Web Push; worker opens or focuses `T` on click |
| No tab/browser responds by deadline | no intent | not Selected, no enrollment, or permission unavailable everywhere | none | `NoExternalTarget` | No immediate surface; replay unread on next app open |
| Focus/route changes before grant | intent revision older than current worker state | Any | prior intent becomes stale | Reject grant, re-arbitrate once | Result follows current state; no client improvisation |
| Chosen tab closes before in-app effect | missing client at final worker check | Any | `stale_state` | Re-arbitrate once, then fallback | Another eligible surface or push |
| Local OS effect succeeds but ACK is lost | worker ledger and OS tag contain notice | Granted | duplicate intent deduped locally | Grant may expire and fallback may race | At most a collapsed duplicate in ordinary same-profile operation |
| Completion was marked read on another responding device | server record read before grant/push CAS | Any | later intents ignored | Settle | No new presentation; other devices clear on next wake |
| Access to `T` is revoked before push planning | product access policy denies projection target | Any | irrelevant | No push candidate; sanitized failed/denied outcome | No content leak; unread entry becomes unavailable on next authorized query |
| Coordinator/store unavailable | no safe arbitration transition | Any | client may retain local event | Retry durable observer/coordinator work; never infer seen | Delayed notification; no false success |

### 6.2 Component decision split

| Question | Decider | Why |
|---|---|---|
| Did a qualifying run complete? | Durable completion observer | Runtime fact, replayable after restart |
| May `U` see `T`? | Product projection access policy | Authority cannot come from a browser payload |
| Which same-profile tab is focused/on `T`? | Service worker from fresh client/tab signals | Browser has the freshest local truth |
| Which responding profile wins? | Durable notification coordinator | Prevents profiles/devices from independently presenting |
| May browser/OS presentation be attempted? | Browser permission plus selected/enrolled server state | Both local platform permission and host configuration are required |
| May Web Push be sent? | Push-candidate authorization plus `OutboundPolicyService` | Stream liveness is not outbound authority |
| Has the completion been read? | Server, only from exact render/focused-visit evidence | Presentation and route beacons alone are insufficient |
| Which window opens on click? | Service worker | It can inspect existing same-origin clients at click time |

## 7. Event transport and wire contracts

### 7.1 Transport decision

Do not add `/notifications/run-completions/events`, another route-specific SSE
hook, or another string convention inside `stream_id`. Add one authenticated,
app-wide transport:

```text
POST /api/webchat/v2/session/websocket-ticket
GET  /api/webchat/v2/session/websocket?ticket=<single-use-ticket>
```

The POST is a WebUI-owned transport-auth operation, not a product command. It
uses the normal bearer-authenticated HTTP stack to mint a bounded, single-use,
15-second ticket bound to the exact `ProductSurfaceCaller`. The opaque ticket
is random and contains no identity or bearer material. The WebSocket upgrade
enforces the existing same-origin policy and consumes the ticket with CAS before
accepting the socket. The long-lived session bearer never appears in the
WebSocket URL, browser history, or proxy access log; a logged ticket is already
consumed or expires within 15 seconds.

`ironclaw_webui` host-auth owns the `WebSocketTicketStore` interface.
Standalone composition may use its bounded in-memory adapter. A multi-replica
deployment must wire a shared CAS adapter so mint and consume may land on
different replicas and replay still fails closed. If that adapter is absent,
the session bootstrap does not advertise WebSocket support and the SPA keeps
using compatibility SSE. Tickets are transport-auth nonces, not product or
conversation records, and may be removed after consumption/expiry.

The socket is server-to-client for product data. Client-to-server frames are
limited to `subscribe`, `unsubscribe`, and `ping`. Notification intents,
presentation acknowledgements, thread-read evidence, thread creation, turn
submission, cancellation, retries, gates, settings, and every other product
mutation continue through authenticated HTTP. The WebSocket handler never
calls `ProductSurface::invoke` or dispatches an operation ID.

### 7.2 Typed product stream interface

Keep the frozen three-method `ProductSurface` interface. Deepen the existing
`stream_events` method by replacing the ambiguous optional string selector and
JSON-erased response with typed contracts owned by
`ironclaw_product_contracts`:

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProductStreamSelector {
    Thread { thread_id: ThreadId },
    RunCompletions,
}

struct ProductSurfaceStreamRequest {
    selector: ProductStreamSelector,
    after_cursor: Option<String>,
}

struct ProductStreamEventEnvelope {
    cursor: ProjectionCursor,
    event: ProductStreamEvent,
}

#[serde(tag = "kind", rename_all = "snake_case")]
enum ProductStreamEvent {
    Thread(ProductOutboundPayload),
    RunCompletion(RunCompletionStreamEvent),
}

struct ProductSurfaceStreamResponse {
    events: Vec<ProductStreamEventEnvelope>,
    next_cursor: Option<String>,
    subscription: Option<ProductSurfaceEventSubscription>,
}
```

`ProductOutboundEnvelope` remains the extension-delivery envelope; its adapter,
installation, target, and delivery-attempt metadata do not cross the browser
product-event seam. The WebUI receives typed product stream events and maps
them exactly once into a redacted browser schema. There is no
`serde_json::Value` encode/decode round trip.

Adding another event family means adding a closed selector/event variant and
its authorization/projection adapter, not a route or a magic string. The
request carries no caller-selectable tenant or user identity. `Thread` resolves
ordinary thread access; `RunCompletions` binds directly to the authenticated
caller's owner scope.

The user-completion source is the durable notice store rather than the runtime
event-log partition. This is necessary because the current runtime stream key
is `(tenant, user, agent)` and cannot represent one caller-wide cursor across
all agents and threads. It also gives arbitration/read transitions one ordered
user-scoped sequence without repurposing a thread cursor.

### 7.3 Session WebSocket protocol

One physical socket multiplexes independent logical subscriptions. A page
subscribes explicitly:

```json
{
  "type": "subscribe",
  "subscription_id": "chat-active",
  "selector": { "kind": "thread", "thread_id": "typed-thread-id" },
  "after_cursor": "opaque-thread-cursor-or-null"
}
```

The app-root notification client independently subscribes to completions:

```json
{
  "type": "subscribe",
  "subscription_id": "run-completions",
  "selector": { "kind": "run_completions" },
  "after_cursor": "opaque-user-completion-cursor-or-null"
}
```

The server acknowledges admission and sends browser-safe event frames:

```json
{
  "schema": "webui.session_event.v1",
  "type": "subscribed",
  "subscription_id": "chat-active",
  "generation": 7,
  "cursor": "opaque-thread-cursor-or-null"
}
```

```json
{
  "schema": "webui.session_event.v1",
  "type": "event",
  "subscription_id": "chat-active",
  "generation": 7,
  "cursor": "opaque-thread-cursor",
  "event": {
    "kind": "thread",
    "payload": { "type": "projection_update", "state": {} }
  }
}
```

`subscription_id` is a bounded client correlation key, never authority or a
resume token. The selector determines authorization and the event's cursor
domain. Each accepted subscription receives a connection-scoped monotonically
increasing `generation`; every event, error, and unsubscribe carries it. The
writer drops frames from cancelled generations, and the client ignores a stale
generation. Reusing an active ID authorizes the replacement first, then
atomically swaps generations and cancels the old subscription. If authorization
fails, the attempted replacement is rejected and the existing authorized
subscription continues unchanged.

There is no session-wide event cursor. The client retains one cursor per
logical subscription:

- thread cursors keep their existing composite runtime/live/turn semantics and
  process-local live epoch;
- the run-completion cursor is one durable owner-scoped notice sequence; and
- reconnect resubscribes each selector from its own last delivered cursor.

On lag, an old cursor, or a live-epoch mismatch, only the affected logical
subscription rebases. A thread rebase returns durable state plus compacted
current live state. A completion rebase returns at most 250 unread/unsettled
notices. One slow thread must not reset notification delivery or another
thread.

### 7.4 Session event gateway and failure isolation

`ironclaw_webui` adds one deep session-event module with two internal seams:

- a `ProductStreamDriver` opens, resumes, drains, and cancels typed
  `ProductSurface::stream_events` subscriptions; and
- a `WebUiSessionEventCodec` maps typed product events into bounded,
  redacted browser frames.

The WebSocket adapter and temporary SSE adapter both use those same modules.
Transport framing, ping/pong, socket backpressure, and connection closure stay
outside product projection code. Selector authorization, replay, rebase,
redaction, and lag stay behind the product stream interface.

Initial hard bounds are host constants:

- at most 3 session event connections per authenticated caller, preserving the
  shipped caller connection budget;
- at most 16 active logical subscriptions per socket;
- at most 64 bytes per `subscription_id`;
- at most 8 KiB per client control frame; and
- 16 queued event batches per logical subscription and 256 across one socket,
  drained with round-robin fairness;
- at most 1,024 outstanding ticket nonces per host-auth store, plus a bounded
  mint rate of 12 tickets per authenticated caller per minute; and
- a 5-minute maximum socket lifetime, matching the shipped stream lifetime,
  after which the client must reauthenticate over HTTP and resume each logical
  selector independently.

Rename the transport-leaking `SseCapacity` module to event-connection capacity
when both transports share the new driver. A lagged subscription receives a
typed terminal `subscription_error` with its last safe cursor and may rebase;
its generation is cancelled without closing the whole socket. Per-subscription
overflow therefore cannot starve notification delivery. Authentication expiry,
malformed framing, or aggregate socket backpressure closes the connection and
releases all subscriptions. Normal lifetime expiry emits a reconnect hint and
closes the socket; the client mints a fresh ticket, which re-evaluates bearer
validity and caller identity. Reconnect uses jittered bounded backoff and does
not resend any product mutation.

### 7.5 Thread-stream migration and compatibility

The session socket initially carries the exact existing thread projection
vocabulary. It does not change provider streaming, cumulative text phases,
16 ms coalescing, final-reply durability, or the thread projection's ordering
rules. The SPA moves connection/reconnect ownership from route-local `useSSE`
to an app-root `SessionEventClient`; `useChatEvents` consumes the same typed
thread payloads through a transport-independent subscription.

During rollout:

1. `/api/webchat/v2/session` advertises session-event protocol support;
2. capable clients use `/session/websocket`;
3. `/threads/{thread_id}/events` remains as a compatibility adapter over the
   shared driver and codec;
4. the existing dormant `/threads/{thread_id}/ws` route is removed rather than
   promoted; and
5. the per-thread SSE route is retired only after production parity and
   rollback confidence.

This migration prevents two implementations from defining event shape or
resume behavior. The compatibility SSE adapter differs only in HTTP framing
and `Last-Event-ID` handling.

### 7.6 User-scoped completion stream

`ProductStreamSelector::RunCompletions` extends the transport-neutral event
stream vocabulary with a user-completion view/target and envelope.
Authorization requires the authenticated caller's tenant/user to equal the
owner scope. The target carries no caller-selectable user ID. The projection
applies the same admission, bounded buffering, access-first ordering, redaction
validation, lag handling, and cursor/rebase rules as thread streams.

On lag or an invalid/old cursor, the stream emits a bounded rebase snapshot of
unread and unsettled notices, then resumes from the snapshot cursor. Read old
records remain durable and queryable but do not resurrect a toast. Slow clients
rebase instead of blocking completion commits.

The typed logical-stream payload is closed vocabulary:

```rust
#[serde(tag = "type", rename_all = "snake_case")]
enum RunCompletionStreamEvent {
    Notice(RunCompletionNoticeEvent),
    Grant(RunCompletionGrantEvent),
    Clear(RunCompletionClearEvent),
}
```

`Notice` opens or replays arbitration state. `Grant` names the one selected
browser profile and surface. `Clear` names a notice/thread whose durable read
transition requires every connected page and worker ledger to dismiss local
surfaces. None of these variants is a product mutation.

### 7.7 Completion event

```json
{
  "schema": "webui.run_completion.v1",
  "type": "notice",
  "sequence": "opaque-cursor-member",
  "notice_id": "opaque-notice-id",
  "run_id": "typed-run-id",
  "thread_id": "typed-thread-id",
  "completed_at": "RFC3339 timestamp",
  "read": false,
  "unread_count_for_thread": 2
}
```

The stream contains no final reply, prompt, thread title, actor name, project
name, tool name, failure reason, or arbitrary URL. The SPA constructs its route
from the validated thread ID. It may show a thread title only by joining with
thread data the authenticated app already fetched through ordinary access
checks.

### 7.8 Intent, grant, acknowledgement, and read operations

All writes use authenticated HTTP handlers that adapt to
`ProductSurface::invoke` operation IDs, not new trait methods and never
WebSocket frames:

```text
POST /api/webchat/v2/notifications/run-completions/intents
POST /api/webchat/v2/notifications/run-completions/acknowledgements
POST /api/webchat/v2/notifications/run-completions/thread-read
```

The product operation IDs remain:

```text
webui.run_completion.intent.v1
webui.run_completion.acknowledge.v1
webui.run_completion.thread_read.v1
```

Intent input:

```json
{
  "notice_id": "opaque-notice-id",
  "browser_instance_id": "opaque-browser-id",
  "tab_id": "opaque-tab-id",
  "state_revision": 41,
  "focus_epoch": 9,
  "intent": "reply_observed | watching_thread | in_app | local_os | unavailable"
}
```

The server derives user/tenant authority from the bound caller and rejects a
foreign notice as `NotFound`. It validates `local_os` against the caller's live
web-app target selection and the browser instance's host-owned enrollment;
client claims cannot mint permission or a target.

Grant event:

```json
{
  "schema": "webui.run_completion_grant.v1",
  "type": "grant",
  "notice_id": "opaque-notice-id",
  "grant_id": "opaque-grant-id",
  "browser_instance_id": "opaque-browser-id",
  "state_revision": 41,
  "surface": "no_surface_watching_thread | in_app | local_os",
  "expires_at": "RFC3339 timestamp"
}
```

Clear event:

```json
{
  "schema": "webui.run_completion_clear.v1",
  "type": "clear",
  "notice_id": "opaque-notice-id",
  "thread_id": "typed-thread-id",
  "read_at": "RFC3339 timestamp"
}
```

Acknowledgement input:

```json
{
  "notice_id": "opaque-notice-id",
  "grant_id": "opaque-grant-id",
  "state_revision": 41,
  "outcome": "reply_rendered | presented | stale_state | effect_failed"
}
```

Thread-read input carries `thread_id`, the greatest completion sequence whose
reply the focused view has rendered, and the browser instance ID. The server
advances only through notices that exist, belong to the caller, target that
thread, and have finalized replies at or below the supplied sequence.

The bounded unread/rebase view uses `ProductSurface::query` with view ID:

```text
webui.run-completions.unread.v1
```

### 7.9 Typed outbound completion part

Do not encode this feature as `FinalReply`, set `web-app.final_replies = true`,
or send the answer as `OutboundPart::Text`. Add distinct typed vocabulary:

```rust
enum RunNotificationEventKind {
    // existing variants...
    RunCompleted,
}

enum OutboundPushKind {
    // existing variants...
    RunCompletion,
}

struct DeliveryTargetCapabilities {
    // existing fields...
    #[serde(default)]
    run_completions: bool,
}

struct RunCompletionNoticeView {
    notice_id: RunCompletionNoticeId,
    thread_id: ThreadId,
    opaque_thread_tag: String,
    unread_count_for_thread: u16,
}

enum OutboundPart {
    // existing variants...
    RunCompletion(Box<RunCompletionNoticeView>),
}
```

Existing target providers default `run_completions` to false. Only the
`web-app` provider advertises true. Resolving the user's effective notification
set and filtering by this capability structurally yields zero or one completion
target even when Slack and Telegram are also configured. Generic fanout code
contains no extension-name condition.

The coordinator then calls `EventStreamManager::push_candidates_for_update`
for the authorized thread projection and resolved target. That method remains
separate from subscription enumeration and delegates to the outbound store.
The `RunCompletion` kind treats the already capability-filtered explicit
reply-target binding as its candidate. `OutboundPolicyService` revalidates the
binding and records the stable attempt before the delivery coordinator calls
`ChannelDelivery::deliver`.

The typed `RunCompletion` part lets the web-app adapter build the safe URL and
fixed copy without interpreting arbitrary text. Other adapters either never
receive the part because their capability is false or report it unsupported in
contract tests.

### 7.10 Encrypted Web Push payload

```json
{
  "schema": "web_app_notification.v2",
  "kind": "run_completion",
  "notice_id": "opaque-notice-id",
  "thread_id": "typed-thread-id",
  "url": "/chat/<percent-encoded-thread-id>",
  "tag": "opaque-purpose-separated-thread-tag",
  "unread_count": 2,
  "title": "IronClaw",
  "body": "An agent run finished."
}
```

The adapter constructs `url` from the typed thread ID; no caller supplies an
arbitrary path. The tag is a bounded, purpose-separated digest of owner scope
and thread ID, not a display string or authorization token. Hash input declares
the purpose `web-app-run-completion-collapse/v1`. The unread count is capped
and may be omitted by older clients.

## 8. Flows by notification form

### 8.1 Focused on `T`: no notification

```mermaid
sequenceDiagram
    participant RT as "Turn/process journal"
    participant O as "Completion observer"
    participant C as "Notification coordinator"
    participant S as "Run-completion logical stream"
    participant P as "Authenticated page"
    participant SW as "Service worker"
    participant T as "Focused tab on T"

    RT->>O: "Completed(R, T, U) committed"
    O->>O: "Verify finalized reply; persist notice"
    O->>C: "Wake pending arbitration"
    C->>S: "run_completion notice"
    S->>P: "Session event"
    P->>SW: "Forward notice"
    SW->>T: "Inspect route/focus and ask render state"
    T-->>SW: "R reply rendered"
    SW-->>P: "reply_observed intent"
    P->>C: "HTTP intent"
    C->>C: "CAS read; settle notice"
    C->>S: "clear T"
    S-->>P: "Session event"
    P-->>SW: "Forward clear"
    SW->>SW: "Close tagged OS notices; clear badges"
```

### 8.2 Focused elsewhere: in-app notification

```mermaid
sequenceDiagram
    participant C as "Notification coordinator"
    participant S as "Run-completion logical stream"
    participant P as "Authenticated page"
    participant SW as "Service worker"
    participant A as "Focused tab on another route"

    C->>S: "run_completion notice"
    S->>P: "Session event"
    P->>SW: "Forward notice"
    SW->>SW: "clients.matchAll; choose focused tab"
    SW-->>P: "in_app intent + state revision"
    P->>C: "HTTP intent"
    C->>C: "Wait intent window; choose winner"
    C->>S: "InApp grant for browser/profile"
    S->>P: "Session event"
    P->>SW: "Forward grant"
    SW->>SW: "Recheck clients and revision"
    SW->>A: "Show one toast and increment grouped badge"
    A-->>P: "Effect succeeded"
    P->>C: "HTTP presented acknowledgement"
    A->>A: "Click navigates SPA to /chat/T"
```

### 8.3 Open but unfocused: local browser/OS notification

```mermaid
sequenceDiagram
    participant C as "Notification coordinator"
    participant S as "Run-completion logical stream"
    participant P as "Authenticated hidden page"
    participant SW as "Service worker"
    participant B as "Background tabs"

    C->>S: "run_completion notice"
    S->>P: "Session event when scheduled"
    P->>SW: "Forward notice"
    SW->>B: "clients.matchAll finds windows, none focused"
    SW-->>P: "local_os intent"
    P->>C: "HTTP intent"
    C->>C: "Validate selected target and enrollment"
    C->>S: "LocalOs grant for browser/profile"
    S->>P: "Session event"
    P->>SW: "Forward grant"
    SW->>SW: "Recheck clients; showNotification with T tag"
    SW-->>P: "Effect succeeded"
    P->>C: "HTTP presented acknowledgement"
    Note over SW,B: "Click focuses/navigates one existing client; never opens a duplicate"
```

Background scheduling is not assumed reliable. If every hidden page is
throttled and misses the one-second intent window, the next flow supplies the
same OS-visible result through Web Push.

### 8.4 No responding tab: Web Push

```mermaid
sequenceDiagram
    participant C as "Notification coordinator"
    participant ES as "EventStreamManager"
    participant OP as "OutboundPolicyService"
    participant D as "Delivery coordinator"
    participant WA as "web-app ChannelDelivery"
    participant PS as "Push service"
    participant SW as "Service worker"

    C->>C: "Intent window closes with no eligible response"
    C->>C: "Resolve effective targets; keep run_completions capability"
    C->>ES: "push_candidates_for_update(thread projection, web-app target)"
    ES->>ES: "Authorize actor/scope/target"
    ES->>OP: "Prepare stable RunCompletion attempt"
    OP->>OP: "Revalidate binding; persist Prepared"
    OP->>D: "Authorized candidate"
    D->>WA: "Typed RunCompletion envelope + registrations"
    WA->>PS: "Encrypted, VAPID-authorized push"
    PS-->>WA: "Transport acceptance or classified failure"
    D->>OP: "Record Delivered / Failed / Unknown"
    PS-->>SW: "push event"
    SW->>SW: "Dedupe and show generic tagged notification"
    Note over SW: "Click focuses/navigates an existing client; openWindow only if none"
```

## 9. Click-through, clearing, dedupe, and stacking

### 9.1 Click selection

The service worker's `notificationclick` handler:

1. closes the clicked notification;
2. validates and derives the same-origin `/chat/T` path from the typed payload;
3. calls `clients.matchAll({ type: "window", includeUncontrolled: true })`;
4. chooses an existing client already on `T`, otherwise the most recently
   focused/visible client known to the worker, otherwise any same-origin
   client in stable ID order;
5. calls `navigate(url)` when needed, then `focus()`; and
6. calls `clients.openWindow(url)` only when no same-origin window exists.

The SPA router already accepts `/chat/:threadId`. On session expiry, the app
routes through login with a validated same-origin return path. If the thread
was removed or access was revoked, the app shows a generic unavailable state,
clears the local surface, and returns no ownership detail.

### 9.2 Exact dedupe and presentation collapse

- Exact event/presentation key: one `notice_id` per run.
- Server replay key: notice ID plus user-scoped sequence.
- Grant key: one opaque `grant_id` tied to notice, profile, revision, and
  expiry.
- Browser ledger: IndexedDB test-and-set by notice ID; memory fallback is
  best-effort if IndexedDB is unavailable.
- OS collapse key: one opaque `tag` per user/thread.
- In-app state: per-run records grouped by thread.

Several completions in the same thread replace one OS notification and update
its generic bounded count. Completions across different threads use different
tags and may stack. Replacing an OS surface does not discard per-run durable
records.

### 9.3 Clearing and read evidence

Whenever a focused thread view confirms finalized replies through completion
sequence `N`, it invokes `thread_read`. The server marks matching notices read
with evidence, emits a clear event, and stops future grants/push ownership for
those notices. Every tab clears its grouped badge. The service worker calls:

```js
const notifications = await registration.getNotifications({ tag });
for (const notification of notifications) notification.close();
```

This path runs whether the user reached `T` from a toast, an OS click, sidebar
navigation, history navigation, or a copied deep link. A route commit without
finalized-reply render does not mark read.

Cross-device clearing is eventually consistent. A connected profile receives
the clear event immediately; a sleeping worker clears on its next stream or
push wake after consulting the durable read state. Already-rendered platform
notifications cannot be recalled from an offline device.

## 10. Configuration and defaults

### 10.1 User-facing behavior

| Setting/state | Default | Effect |
|---|---|---|
| In-app completion notices | On | Event-driven toast/badge while authenticated; independent of external notification target selection |
| Web-app external notification target | Existing user preference | Must be selected for local OS and Web Push presentation |
| Browser permission | Browser-managed `default` | No prompt and no OS surface until an explicit gesture grants it |
| Browser enrollment | Off per browser until explicit enrollment | Required for local OS intent validation and server push addressing |
| Per-thread completion preference | None in P0 | Avoids inventing a new thread-policy field; account-level web-app selection controls external completion notification |
| Reply preview/title on lock screen | Off, not configurable in P0 | Privacy-safe fixed copy only |

An empty notification-channel set still means no external route. In-app
completion events continue because they are part of the authenticated product,
not a catalog delivery target. Selecting Slack or Telegram without Web App does
not enable completion fanout to those channels.

### 10.2 Permission UX

Keep the shipped settings affordance as the canonical enrollment path. After a
user receives the first useful in-app completion toast, the toast or
notification center may show a non-modal “Notify me when I'm away” action. Only
that click may call `Notification.requestPermission` and, after clear consent,
enroll the browser and add the Web App target to the existing notification set.

- `default`: explain the benefit; wait for a click.
- `granted`: allow enrollment/repair; do not claim receipt until backend
  enrollment read-back succeeds.
- `denied`: show settings guidance, never prompt again programmatically.
- unsupported/service worker unavailable: in-app only.

## 11. Privacy, security, and failure posture

### 11.1 Redaction

OS-visible fields are fixed:

- title: `IronClaw`;
- body: `An agent run finished.` or a generic localized plural count;
- no thread title, prompt, response preview, user/agent/project name, tool
  activity, approval detail, failure detail, or arbitrary model text.

The encrypted payload contains only the notice ID, typed thread ID, same-origin
route, opaque collapse tag, bounded count, and timestamps needed for dedupe.
Push providers can still observe endpoint traffic timing and ciphertext size;
fixed, bounded payload shape reduces that side channel.

The in-app UI may resolve a thread title from existing authorized application
state. A stream or push payload never carries it as a shortcut.

### 11.2 Authorization

- Socket-ticket minting binds the exact authenticated caller; upgrade consumes
  one matching unexpired nonce, and the five-minute lifetime forces periodic
  bearer revalidation.
- Every logical selector is independently authorized through
  `ProductSurface::stream_events`. `RunCompletions` derives tenant/user owner
  scope from the bound caller; `Thread` resolves ordinary thread access.
- Client `subscription_id`, generation, and cursor values carry no authority.
- Notice intents, acknowledgements, reads, and queries return `NotFound` for
  foreign IDs.
- A push candidate requires projection authorization for actor, scope, and
  thread target before outbound planning.
- The web-app binding is revalidated at every attempt.
- Enrollment records remain host-owned and owner-scoped.
- VAPID material remains host-side and is injected only at restricted egress.
- URLs are typed/same-origin and never accepted from untrusted intent input.

### 11.3 Failure classification

| Failure | Behavior |
|---|---|
| Notice persistence fails | Observer does not advance its durable cursor; replay retries |
| Session socket disconnected | Mint a fresh one-time ticket, reconnect, and resume each logical selector from its own cursor; rebase only affected subscriptions when needed |
| Hidden tab throttled | No intent; authorized push fallback after the window |
| Intent/ACK duplicated | Stable IDs and CAS return the existing outcome |
| Grant stale | Client returns `stale_state`; one re-arbitration follows |
| Browser effect succeeds, ACK fails | Possible duplicate; local ledger/tag collapses ordinary case |
| Coordinator crashes before due time | Boot reconciliation rearms pending timer |
| Crash after `PushOwned` but before attempt | Recover same stable delivery identity |
| Crash in `Prepared` | Safe retry through existing outbound semantics |
| Crash in `Sending` | Existing `Unknown`; never blindly resend |
| Push endpoint returns 404/410 | Existing host path prunes registration |
| Push provider returns retryable error | Record sanitized failure and apply existing bounded retry policy; run remains completed |
| No enrollment | `NoExternalTarget`; retain unread in-app state |
| Permission denied/default | No OS attempt, no automatic prompt, retain unread state |
| Thread access revoked | Fail closed, reveal no thread content, clear unusable local surface |

A notification failure never changes the run's successful terminal state.
Operator-visible metrics count pending age, intent/grant outcomes, fallbacks,
provider classifications, stale grants, and dedupe hits without logging thread
titles, payloads, endpoints, or user navigation.

## 12. Options considered

### 12.1 Client decides; server always streams and pushes

The server emits one completion event and unconditionally Web Pushes every
enrolled registration. Each service worker dedupes the stream/push race and
chooses suppress, toast, or OS from fresh local state.

Advantages:

- lowest server complexity;
- freshest focus/route data;
- strong same-profile multi-tab behavior; and
- natural reuse of the shipped worker.

Rejected because it sends every completion across push-provider infrastructure
even when immediately suppressed, cannot stop a phone notification when a
laptop is focused on `T`, and leaves read state browser-local. Inferring “push
only if no subscriber” would not repair it and would violate the contract that
subscriber visibility and outbound push selection are separate.

### 12.2 Server decides from standing presence leases

Every tab reports route, visibility, focus, permission, and enrollment on
changes plus a lease renewal. A shared server presence store chooses the exact
surface.

Advantages:

- the server can rank devices before dispatch;
- straightforward global read/presence queries; and
- no arbitration intent window per completion.

Rejected because focus is stale immediately after reporting; correct replica
behavior requires shared, high-churn lease storage, expiry, reconnect
generations, and ACK/redecision anyway. It also persists sensitive browsing
telemetry and would require registration-specific selection because the
shipped adapter fans out to all registrations. This pays distributed-systems
cost for facts the browser knows better without eliminating the final race.

### 12.3 Hybrid post-effect claim

Browsers immediately present based on local state and then claim the notice;
the server delays push briefly and suppresses it if a claim arrives.

Advantages:

- immediate local UI;
- no standing presence; and
- durable push fallback can recover after restart.

Not selected as the final form because two browser profiles may both present
before either claim reaches the server, and a focused-on-`T` device cannot
prevent a faster background device from showing OS UI. The recommended
intent/grant variant adds one bounded collection window so the server can rank
responding profiles before any effect.

### 12.4 Decision scorecard

| Criterion | Client-owned always-push | Server presence | Post-effect claim | Recommended intent/grant |
|---|---:|---:|---:|---:|
| Fresh local focus/route | Excellent | Poor/medium | Excellent | Excellent |
| Same-profile multi-tab | Excellent | Good | Excellent | Excellent |
| Cross-profile suppression | Poor | Good | Medium | Good within window |
| No standing browsing telemetry | Excellent | Poor | Excellent | Excellent |
| Avoids unnecessary provider egress | Poor | Good | Good | Good |
| Restart-safe without new durable state | Poor | Poor | Poor | Poor |
| Server complexity | Low | Very high | High | High but bounded |
| Fits outbound subscriber separation | Good | Good if carefully separated | Good | Good |
| Presentation latency | Immediate | Lease/race dependent | Immediate | +1 second before local grant |
| Failure preference | Duplicates | Stale suppression risk | Duplicates | Duplicates over omissions |

The one-second latency is the deliberate cost of suppressing lower-priority
notifications when another responding profile is already showing the reply.

### 12.5 Event transport alternatives

Three transport shapes were considered separately from notification
arbitration:

| Shape | Advantages | Costs / decision |
|---|---|---|
| Keep thread SSE and add `/notifications/run-completions/events` | Smallest immediate diff | Repeats route, hook, connection, cursor, and retry machinery; preserves the shallow string selector; rejected |
| One session SSE carrying every event | Server-to-client semantics fit events | Dynamic thread subscriptions require reconnect/query mutation or over-delivery; independent stream failures become awkward; rejected |
| Read-only session WebSocket with typed logical subscriptions | One connection per page, dynamic selectors, independent cursors, shared thread/notification transport | Requires a bounded control protocol and staged migration; selected |

The selected socket is not the existing per-thread WebSocket implementation.
That route is unused by the SPA, duplicates the SSE loop, authenticates with a
long-lived bearer in the URL, and sends the wrong raw envelope. Phase 0 replaces
it with a ticketed session gateway over the shared stream driver and codec.

## 13. AgentExecution migration

The implementation must work on current `main`; it does not depend on the
agent-execution branch or PR #7562. Keep completion observation behind a narrow
product port:

```rust
trait CompletionObservationSource {
    async fn subscribe_completions(
        &self,
        after: Option<CompletionObservationCursor>,
    ) -> Result<CompletionObservationSubscription, CompletionObservationError>;
}
```

P0 adapts the committed process/turn journal through
`ProcessJournalCommitObserver`; no run-state polling is introduced. When
`AgentExecution::subscribe` becomes the canonical execution observation
facade, a new adapter maps its terminal `Completed` event plus the conversation
workflow's execution-to-thread association into the same completion notice.

The notice store, user stream, intent/grant protocol, service worker,
configuration, read state, push candidate, outbound attempts, and
`ChannelDelivery` remain unchanged. The engine still knows nothing about
notification routing or browser presence.

## 14. Phased implementation

### Phase 0 — deepen the session event pipeline

Architectural prerequisite with no notification behavior change:

1. Replace `stream_id: Option<String>` and `Vec<serde_json::Value>` with the
   typed `ProductStreamSelector` and `ProductStreamEventEnvelope` contracts.
2. Add the shared `ProductStreamDriver` and `WebUiSessionEventCodec`; make the
   current thread SSE handler use them without changing its browser wire shape.
3. Add short-lived single-use socket tickets, the read-only
   `/api/webchat/v2/session/websocket` protocol, per-connection subscription
   budgets, independent cursor resume, failure isolation, and backpressure.
4. Add the app-root `SessionEventClient`, migrate the active thread stream from
   route-local `useSSE`, and prove cumulative model text plus durable final
   reply parity through the real browser caller.
5. Remove the dormant per-thread WebSocket route. Keep per-thread SSE as a
   compatibility adapter and rollback path until the new transport is proven.

No product mutation gains a WebSocket representation in this phase.

### Phase 1 — event-driven in-app completion notices

Smallest shippable slice:

1. Add the completion observer and durable notice/read store.
2. Add `ProductStreamSelector::RunCompletions`, its user-scoped projection,
   cursor, redaction, admission, and rebase snapshot; do not add a route.
3. Add the HTTP intent, acknowledgement, and thread-read operations.
4. Add service-worker multi-tab arbitration for the focused cases. Every tab
   may receive the stream event; the worker dedupes per browser profile.
5. Add grouped in-app toast/badge UI, click-through, exact reply-render read
   evidence, and clearing.
6. Leave OS and Web Push completion delivery disabled.

This phase removes completion polling from the problem without touching
notification permission or egress. The existing approval notification poller
remains separate and unchanged.

### Phase 2 — local OS presentation for open browsers

1. Add notice-specific intents/grants and coordinator timers.
2. Add `LocalOs` grant application through
   `ServiceWorkerRegistration.showNotification`.
3. Add notification tags, counts, improved client selection, and clearing.
4. Reuse explicit enrollment/permission UX; add the optional first-value CTA.
5. Exercise hidden-tab throttling and grant-expiry fallback without server
   push.

### Phase 3 — closed-app Web Push fallback

1. Add `RunCompleted`/`RunCompletion` vocabulary and the additive
   `run_completions` target capability.
2. Resolve the effective notification set, structurally filter to the
   web-app target, and call the push-candidate authorization seam.
3. Add typed `RunCompletionNoticeView` rendering in the web-app adapter with
   `/chat/T`, safe fixed copy, tag, and count.
4. Wire stable delivery attempts, no-target settlement, recovery, and existing
   404/410 pruning.
5. Add session-expiry and unavailable-thread click handling.

### Phase 4 — recovery and cross-device hardening

1. Add boot reconciliation and multi-replica CAS race coverage.
2. Add cross-device clear propagation and stale-grant observability.
3. Tune the one-second window only from privacy-safe latency/duplicate metrics.
4. Evaluate mobile PWA platform differences before claiming parity.

## 15. Test strategy for implementation

Repository policy requires tests first and integration coverage through the
production caller, not helper-only tests.

### Unit and contract

- Typed selector authorization and cursor-domain tests; a thread cursor cannot
  resume `RunCompletions`, and no selector can name another caller's user.
- Product stream responses remain typed through the WebUI codec and never
  expose adapter/installation/target/delivery-attempt metadata.
- Session protocol accepts only subscribe/unsubscribe/ping, rejects operation
  IDs and mutation-shaped frames, bounds all identifiers/frames, and isolates a
  lagged subscription without closing healthy subscriptions.
- Single-use socket ticket expiry, replay rejection, caller binding, and
  same-origin upgrade enforcement.
- Cross-replica ticket mint/consume uses one CAS winner; missing shared ticket
  storage suppresses session-WebSocket capability advertisement.
- The shared driver/codec produces equivalent thread event payloads through
  session WebSocket and compatibility SSE framing.
- Notice ID and thread-tag purpose separation and stability.
- Store conformance on every filesystem backend: idempotent create, bounded
  CAS transitions, read/delivery orthogonality, intent ranking, grant expiry,
  push ownership, and restart reconciliation.
- Projection authorization, redaction validation, cursor scope, lag/rebase,
  buffer bounds, and foreign-user `NotFound` behavior.
- Target capability defaults false for every existing provider and true only
  for web-app.
- Web-app adapter payload contains no arbitrary `Text`, prompt, reply, title,
  error, endpoint, or user/tenant value.

### Reborn integration

- Production-wired HTTP thread creation and turn submission followed by a
  session `Thread` subscription streams cumulative sanitized text, preserves
  multi-model-call phases, and ends with the exact durable finalized reply.
- Two logical subscriptions on one socket resume independently; lag/rebase in
  one does not advance, reset, or authorize the other.
- Mutation attempts are exercised through their HTTP caller and no WebSocket
  handler reaches `ProductSurface::invoke`.
- Production-wired completed top-level turn with finalized reply creates one
  user-stream notice; subagent/detached/failed/cancelled runs do not.
- Focused-on-thread reply evidence settles the notice and produces no outbound
  attempt.
- Focused-elsewhere intent receives one in-app grant and remains unread until
  thread render.
- No eligible intent plus selected/enrolled web-app target produces exactly
  one authorized, durable attempt through
  `push_candidates_for_update -> OutboundPolicyService -> ChannelDelivery`.
- Slack + Telegram + Web App selected still yields only the Web App completion
  candidate.
- Empty target set, denied access, missing enrollment, and revoked binding
  produce no provider egress and retain sanitized state.
- Fake-clock claim/deadline/ACK races and restart states have one CAS owner.

### Browser E2E

- Create a new thread, submit a question over HTTP, observe incremental text
  and the durable final reply over the session socket, navigate away/back, and
  resume without duplicate assistant messages.
- Socket disconnect, ticket refresh, reconnect, per-selector resume, one
  selector rebase, five-minute reauthentication, hidden-page throttling, and
  compatibility SSE fallback.
- All core decision-matrix rows, including two tabs with one focused on `T`.
- Duplicate session frames, session/push race, stale grants, hidden pages, and
  tab closure.
- `default`, `denied`, `granted`, unsupported, and never-prompt-on-load
  permission behavior.
- Notification click prefers an existing `T` client, otherwise navigates one
  existing client, and opens only when none exists.
- Any navigation to `T` after render clears the thread tag and grouped badge.
- Several runs in one thread collapse; several threads stack.
- Expired session and inaccessible thread reveal no protected content.

### Backend/runtime and live canary

- Shared-backend multi-replica CAS and boot-recovery tests for pending,
  granted, and push-owned records.
- Recorded restricted-egress fixture for Web Push request shape and endpoint
  pruning; never rely on a live push provider in ordinary CI.
- A live canary, if added later, asserts transport acceptance only and never
  claims OS display.

## 16. Rollout, compatibility, and rollback

- Phase 0 is additive. The session bootstrap advertises protocol support and a
  client chooses exactly one thread transport. Older clients keep using SSE;
  newer clients fall back to SSE if ticket minting or socket upgrade fails.
- Session WebSocket and compatibility SSE share one driver and browser codec,
  so fallback does not define a second event contract. The dormant per-thread
  WebSocket is removed because it is unused and violates the documented frame
  shape.
- Roll back the transport by disabling the session capability advertisement;
  clients return to compatibility SSE without changing provider, projection,
  transcript, or finalized-reply behavior.
- No existing notification default changes in Phase 1: in-app notices are new
  product UI; external delivery still requires the user's existing target
  selection and enrollment.
- All wire additions are versioned and additive. Capability booleans use
  `serde(default)` false.
- New filesystem paths and indexes do not alter existing enrollment or
  outbound attempt records. Older binaries ignore them.
- Roll back notifications by disabling the new completion
  observer/selector/coordinator. Retained notice records become inert; existing
  gate, auth, failure, final-reply, and model-delivery behavior is unchanged.
- A partial rollout must keep the observer, coordinator, and product stream on
  compatible versions. The service worker ignores unknown payload schemas;
  the server falls back to unread in-app state rather than sending an old
  text-shaped completion push.

## 17. Non-goals

- Sending completed-run answers or previews to Slack, Telegram, email, or any
  other channel.
- Making the web-app target a final-reply/model-delivery target.
- Replacing the existing gate/auth/failure notification flow.
- Progress notifications or “agent is still working” reminders.
- Sending product commands, notification intents, acknowledgements, read
  evidence, or any other mutation through the WebSocket.
- Inventing a global cursor or total order across unrelated logical streams.
- A general-purpose user-presence service or durable focus-history store.
- Globally atomic exactly-once notification display across offline devices.
- Changing agent execution, scheduling, turn state, transcript storage, or the
  agent-execution design branch.
- Mobile PWA parity claims before platform-specific validation.
- Notification actions such as approve/deny from the lock screen.
- Carrying model-generated content, thread titles, or error detail on an OS
  notification.

## 18. Open questions, follow-ups, and deliberately deferred choices

1. **Blocked-on-approval:** the same browser arbitration and safe presentation
   renderer can later carry “Your agent needs a decision,” but it must reuse
   the existing gate/auth fact and routing rather than invent a second notice.
2. **Progress:** the dormant progress policy remains dormant. Progress needs
   explicit cadence, collapse, and opt-in design before using this path.
3. **Mobile PWA:** iOS delivery timing, installed-PWA requirements, focus
   semantics, and badge APIs need separate device testing.
4. **Arbitration tuning:** start at one second. Change only when metrics show a
   concrete latency/duplicate trade-off; do not expose a user setting.
5. **Cross-device exactness:** if product requirements later demand globally
   exactly one visual effect, design a longer two-phase device election with
   explicit leases. Do not stretch short-lived intents into standing presence.
6. **Existing approval poller:** migrate it to a typed session-event selector
   in a separate change after completion notifications prove the user-wide
   stream. Do not add another streaming route.

## 19. Recommendation

First deepen `ProductSurface::stream_events` into a typed logical-stream
interface and ship the read-only session WebSocket over a shared WebUI stream
driver/codec. Migrate current chat text streaming onto that transport without
changing the HTTP command path, live-text projection, or durable finalized
reply contract. Then ship the notification feature as a durable user-scoped
completion selector plus a bounded intent/grant coordinator, leaving fresh
multi-tab inspection and local effects to the service worker. Use the existing
web-app delivery surface only as the authorized no-presenter fallback, selected
structurally by an additive target capability and recorded through the existing
outbound attempt lifecycle. Add in-app presentation, then local OS presentation,
and finally Web Push so each phase provides user value with a contained failure
surface.
