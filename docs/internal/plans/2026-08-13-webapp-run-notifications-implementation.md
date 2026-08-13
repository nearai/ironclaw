# Web-app run-completion notifications — implementation plan

> **For agentic workers:** execute tasks in order; every task ends with its
> named checks green before the next begins. Steps use checkbox (`- [ ]`)
> syntax for tracking.

**Goal:** Implement the approved design in
`docs/internal/design/2026-08-13-webapp-run-notifications.md` end to end —
typed product stream selectors, the ticketed read-only session WebSocket, the
durable run-completion notification system through Web Push fallback — as one
reviewable PR stacked on `docs/webapp-run-notifications`.

**Architecture:** Deepen `ProductSurface::stream_events` into a typed
logical-stream interface; multiplex independent typed subscriptions over one
ticket-authenticated session WebSocket driven by a shared WebUI stream driver
and browser codec (per-thread SSE kept as a compatibility adapter, dormant
per-thread WS removed). Layer the notification system on a durable
owner-scoped notice store: journal-observer ingest → notice store →
user-scoped completion stream → intent/grant coordinator → service-worker
presentation → capability-filtered Web Push fallback through the existing
outbound attempt lifecycle.

**Tech stack:** Rust (axum, tokio), `ironclaw_filesystem` CAS/index plane,
Vite/TypeScript SPA (React, vitest), hand-written `sw.js`, Web Push (existing
RFC 8291/VAPID stack in `ironclaw_web_app`).

All proposal section references (§N) are to
`docs/internal/design/2026-08-13-webapp-run-notifications.md`. All file:line
references were verified against this branch (`90a75faa0`).

## Global constraints (from the proposal + repo rules)

- Every product mutation stays on authenticated HTTP; the WebSocket accepts
  only `subscribe` / `unsubscribe` / `ping` control frames and never reaches
  `ProductSurface::invoke` (§3.8, §7.1).
- One transport route only: `POST /api/webchat/v2/session/websocket-ticket` +
  `GET /api/webchat/v2/session/websocket`. No event-specific streaming routes.
- `ProductSurface` keeps exactly `invoke`/`query`/`stream_events`
  (freeze ratchet); the selector deepening rides inside the stream DTOs.
- No global cursor: thread and run-completion subscriptions keep independent
  authorization, cursors, replay, rebase, lag, and failure domains (§3.9).
- Preserve chat streaming semantics byte-for-byte on the browser wire:
  cumulative phase-keyed live text, 16 ms coalescing, process-local live
  epoch, durable finalized reply upgrade (§2.1, §7.5).
- OS/push surfaces carry only fixed copy + typed IDs/tags/counts — never
  model-generated or protected content (§7.10, §11.1).
- Streams never authorize push; push requires durable arbitration state,
  projection authorization, the `run_completions` target capability,
  enrollment, and outbound policy (§3.4, §7.9).
- No new Cargo features; deployment shape decisions read
  `DeploymentConfig::storage_shape()` (never `RebornProfile` — ratcheted).
- No production `unwrap`/`expect`; cause-preserving error mapping; `debug!`
  logging only in background tasks; durable records retained with timestamps
  (cache eviction ≠ deletion).
- New-code coverage bar: ≥90 % changed-line coverage
  (`tests/integration/changed-coverage-exemptions.toml` policy); integration-
  first test placement per `.claude/rules/testing.md`.
- Retired vocabulary: never write `web_push`/`web-push`/`WebPush` outside the
  pinned legacy files; avoid `delivery_target_id`, `stored_preference_target`,
  `TriggeredDelivery`, `outbound_delivery_target_set` identifiers.

## Ownership map (recon-verified placements)

| Component | Home | Notes |
|---|---|---|
| `ProductStreamSelector`, `ProductStreamEventEnvelope`, `ProductStreamEvent` | `ironclaw_product_contracts::surface` | replaces `stream_id: Option<String>` + `Vec<serde_json::Value>`; freeze ratchet allows DTO changes, not new methods |
| Run-completion wire vocabulary + operation descriptors (`webui.run_completion.*.v1`, unread view) | `ironclaw_product_contracts::run_completions` (new module) | `notification_setup` module precedent; keeps the webui→assistant symbol residue at 103 |
| Session-socket ticket port | `ironclaw_product_contracts::session_transport` (new module) | `OperatorSecretValueStore` precedent: webui consumes, composition implements the shared adapter; in-memory adapter lives in webui host-auth |
| Shared stream driver + browser codec + session WS protocol | `ironclaw_webui::webui_v2::session_events` (new module) + `handlers/session_events.rs` | charter map gains a `session-events` sub-owner row |
| Notice store, ingest, user stream hub, coordinator, push facade | `ironclaw_assistant::run_completions` (new module tree) | proposal §5.1 assigns all four to `ironclaw_assistant`; composition constructs them (`DeliveryCoordinator` precedent) |
| Journal observer adapter | `ironclaw_composition` (new `run_completion_observer.rs`) | maps `ProcessJournalCommit` → assistant ingest port; keeps kernel dep out of assistant; §13 seam |
| `/run-notices` per-user mount alias | `ironclaw_composition::lib::PER_USER_ALIASES` + storage-placement §4 row | rides the CAS-capable `/tenants` plane; no new physical mount |
| `OutboundPart::RunCompletion` + `RunCompletionNoticeView` + renderer | `ironclaw_extension_contracts` (`channel_adapter.rs` + new `run_completion.rs`) | `AuthPrompt` boxed-view precedent |
| `OutboundPushKind::RunCompletion`, `RunNotificationEventKind::RunCompleted`, `DeliveryTargetCapabilities.run_completions` | `ironclaw_outbound` | `notifications` capability triad precedent |
| Web Push payload v2 fields + adapter arm | `ironclaw_web_app::message` + `packages/web-app/src/channel.rs` | additive fields; adapter builds `/chat/<pct-encoded-id>` from the typed view |
| SPA session client, notification UI, SW arbitration/ledger | `frontend/src/lib/session-events/`, `frontend/src/lib/run-completions/`, `frontend/public/sw.js` | lazy-loaded (bundle budget ~0 headroom); SW stays dependency-free |

## Key interfaces (single source of truth for all tasks)

### Typed stream contracts (`ironclaw_product_contracts::surface`)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProductStreamSelector {
    Thread { thread_id: String },   // parsed/authorized in product, as today
    RunCompletions,                  // added in Phase 1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSurfaceStreamRequest {
    pub selector: ProductStreamSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductStreamEventEnvelope {
    pub cursor: ProjectionCursor,
    pub event: ProductStreamEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProductStreamEvent {
    Thread(ProductOutboundPayload),
    RunCompletion(RunCompletionStreamEvent),   // added in Phase 1
}

pub struct ProductSurfaceStreamResponse {
    pub events: Vec<ProductStreamEventEnvelope>,
    pub next_cursor: Option<String>,
    #[serde(skip)]
    pub subscription: Option<ProductSurfaceEventSubscription>,
}
```

The adapter/installation/target/delivery-attempt fields of
`ProductOutboundEnvelope` stop at the product boundary (they are synthesized
constants on the WebUI path today and dropped by the browser codec).

### Session WebSocket protocol (`webui.session_event.v1`)

Client → server (only these; anything else closes the socket):

```json
{"type":"subscribe","subscription_id":"…≤64B…","selector":{"kind":"thread","thread_id":"…"},"after_cursor":null}
{"type":"unsubscribe","subscription_id":"…"}
{"type":"ping"}
```

Server → client frames all carry `schema:"webui.session_event.v1"`:
`subscribed{subscription_id,generation,cursor}` ·
`event{subscription_id,generation,cursor,event:<browser event>}` ·
`subscription_error{subscription_id,generation,error,kind,retryable,last_cursor}` ·
`unsubscribed{subscription_id,generation}` · `pong` ·
`reconnect_hint{reason:"lifetime_expired"}`.

For thread subscriptions `event` is exactly today's `WebChatV2EventFrame`
event body (same tag names, produced by the same codec the SSE adapter uses).
Bounds (host constants in `session_events::protocol`): ≤16 active
subscriptions/socket, ≤64-byte `subscription_id`, ≤8 KiB control frame,
16 queued batches/subscription, 256/socket, 5-minute lifetime, connection
budget shared with SSE (3 per caller).

### Ticket port (`ironclaw_product_contracts::session_transport`)

```rust
pub struct SessionSocketTicket {
    pub tenant_id: String,
    pub user_id: String,
    pub operator_config: bool,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
}

#[async_trait]
pub trait SessionSocketTicketStore: Send + Sync {
    /// Store a freshly minted single-use ticket under `nonce`.
    async fn mint(&self, nonce: &str, ticket: SessionSocketTicket)
        -> Result<(), SessionSocketTicketStoreError>;
    /// Atomically consume; at most one caller ever receives the ticket.
    async fn consume(&self, nonce: &str)
        -> Result<Option<SessionSocketTicket>, SessionSocketTicketStoreError>;
}
```

Adapters: `InMemorySessionSocketTicketStore` (webui host-auth; 1024-entry
bound, TTL sweep-on-insert, single-use `take` — `SessionTicketStore`
pattern) and `SecretStoreSessionSocketTicketStore` (composition, over the
injected secret store's one-shot lease/consume — durable-PKCE precedent,
multi-replica CAS). Ticket TTL 15 s; nonce = 32 random bytes hex. Selection:
`StorageShape::LocalFilesystemRoot` (single-process standalone) → in-memory;
`HostedSingleTenantPool`/`OperatorSupplied` (shared DB, replicas possible) →
secret-store adapter; if neither can be wired → session WebSocket feature not
advertised (fail closed) and SPA stays on SSE.

### Run-completion wire vocabulary (`ironclaw_product_contracts::run_completions`)

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunCompletionStreamEvent {
    Notice(RunCompletionNoticeEvent),
    Grant(RunCompletionGrantEvent),
    Clear(RunCompletionClearEvent),
}
// Notice: schema "webui.run_completion.v1": sequence, notice_id, run_id,
//   thread_id, thread_tag (opaque collapse digest), completed_at, read,
//   unread_count_for_thread.
// Grant: schema "webui.run_completion_grant.v1": notice_id, grant_id,
//   browser_instance_id, state_revision, surface
//   (no_surface_watching_thread | in_app | local_os), expires_at.
// Clear: schema "webui.run_completion_clear.v1": notice_id, thread_id, read_at.
```

Operation IDs + descriptors (constants here, behavior in assistant):
`webui.run_completion.intent.v1`, `webui.run_completion.acknowledge.v1`,
`webui.run_completion.thread_read.v1` (capabilities) and
`webui.run-completions.unread.v1` (view). Input DTOs carry the §7.8 shapes
with byte bounds on every opaque ID (≤128 B) and `observedRunIds`-style lists
capped at 128; intents capped at 32 per notice server-side.

### Notice store records (`ironclaw_assistant::run_completions::store`)

Records per §5.3 (`RunCompletionNotice`, `CompletionDeliveryState`,
`CompletionSurface`, `CompletionReadState`, `CompletionReadEvidence`) plus a
per-owner `RunCompletionSequence` counter record. Paths under the `/run-notices`
alias (owner scope from `ResourceScope`):

```
/run-notices/notices/{notice_id}.json          # one immutable-fact + state machine record
/run-notices/sequence.json                      # per-owner monotonic sequence (CAS)
```

Ordered index `run_notice_order`: equality key `owner_key`
(tenant/user digest), sort key `sequence` (zero-padded), tie-break
`notice_id`; declared with `ensure_index`, queried with `query_ordered`
keyset pagination (thread_index.rs pattern). Every transition goes through
`cas_update`. `notice_id` =
`hex(blake3(owner_key ‖ run_id ‖ "web-app-run-completion/v1"))[..32]`;
`thread_tag` = `hex(blake3(owner_key ‖ thread_id ‖
"web-app-run-completion-collapse/v1"))[..32]`.

### Completion ingest port (assistant) + composition adapter

```rust
// ironclaw_assistant::run_completions::ingest
pub struct CompletionObservation {
    pub run_id: TurnRunId,
    pub scope: TurnScope,                 // includes thread + owner
    pub owner_user_id: UserId,
    pub top_level: bool,                  // parent/root process ids both None
    pub completed_at: Timestamp,
}
pub enum CompletionIngestOutcome { NoticeCreated, AlreadyRecorded, Ineligible, NoFinalReply }
impl RunCompletionIngest {
    pub async fn ingest(&self, observation: CompletionObservation)
        -> Result<CompletionIngestOutcome, RunCompletionIngestError>; // retryable flag inside
}
```

Composition's `RunCompletionJournalObserver` implements
`ProcessJournalCommitObserver` (`process_observer_id =
"web-app-run-completion-observer-v1"`), filters
`kind == Completed && process_kind == AgentTurn && parent_process_id.is_none()
&& root_process_id.is_none() && scope.thread_id.is_some() && owner set`,
maps fields, calls `ingest`; returns `Err` only for
`RunCompletionIngestError { retryable: true }` (journal store retries with
backoff), `Ok` otherwise (anomalies counted, cursor advances).

### Outbound vocabulary (Phase 3)

- `ironclaw_outbound`: `OutboundPushKind::RunCompletion` (+`as_str`),
  `RunNotificationEventKind::RunCompleted` (`delivery_kind()` →
  `RunCompletion`), `DeliveryTargetCapabilities { …, #[serde(default)]
  run_completions: bool }`, `resolve_run_completion_target` gate beside
  `resolve_notification_target`, and a deliberate
  `plan_push_targets_from_policy` arm: `RunCompletion => true` (candidate is
  the already-capability-filtered explicit binding — §7.9).
- `ironclaw_extension_contracts::run_completion::RunCompletionNoticeView
  { notice_id: String, thread_id: String, thread_tag: String,
  unread_count_for_thread: u16 }` + `OutboundPart::RunCompletion(Box<…>)`.
- `ironclaw_web_app::message::WebAppNotificationPayload` gains additive
  optional fields: `schema` (`"web_app_notification.v2"` for completions),
  `kind` (`"run_completion"`), `notice_id`, `thread_id`, `unread_count`.
  Adapter fixed copy: title `IronClaw`, body `An agent run finished.`;
  URL `/chat/<percent-encoded thread_id>`; tag = `thread_tag`.

---

## Phase 0 — deepen the session event pipeline

### Task 0.1 — typed selector/envelope contracts

**Files:** `crates/contracts/ironclaw_product_contracts/src/surface.rs`,
`tests/product_contract.rs`; ripple:
`crates/product/ironclaw_assistant/src/reborn_services.rs`
(decode/encode/open_product_surface_event_subscription),
`crates/product/ironclaw_webui/src/webui_v2/handlers.rs` (SSE + WS callers,
delete `decode_product_outbound_events`),
`crates/product/ironclaw_openai_compat/src/mount.rs` (Thread selector, typed
drain), `crates/extensions/ironclaw_extension_host/src/channel_host.rs`
(pass-through), test doubles (`RecordingProductSurface`,
webui `tests/support/product_surface.rs`, assistant contract tests,
`crates/app/ironclaw_composition/src/runtime/tests/core.rs`).

- [ ] Pin new wire shapes in `product_contract.rs` first (selector tagged
  form, envelope `{cursor, event}` with `kind`-tagged event, request/response
  round-trips); watch fail.
- [ ] Land the enum/struct changes; `ProductStreamEvent::Thread` carries
  `ProductOutboundPayload`; keep `subscription: Option<…>` semantics and the
  not-`Clone` pins.
- [ ] Assistant: `decode_product_surface_stream_request` matches on selector
  (Thread arm = today's behavior; no RunCompletions yet); replace
  `encode_product_surface_stream_response` with direct envelope construction
  `ProductStreamEventEnvelope { cursor: envelope.projection_cursor, event:
  Thread(envelope.payload) }` in both drain and subscription paths.
- [ ] WebUI/openai_compat: construct `ProductStreamSelector::Thread`, consume
  typed events (SSE frame mapping takes `(cursor, payload)`); delete the
  `serde_json::Value` decode helpers in both crates.
- [ ] Checks: `cargo test -p ironclaw_product_contracts -p ironclaw_assistant
  -p ironclaw_webui -p ironclaw_openai_compat`, then
  `cargo test -p ironclaw_architecture_tests --test
  reborn_transport_product_boundary --test reborn_service_method_freeze_ratchet`.

### Task 0.2 — shared driver + codec; SSE rides them

**Files:** new `crates/product/ironclaw_webui/src/webui_v2/session_events/`
(`mod.rs`, `driver.rs`, `codec.rs`), rename
`sse_capacity.rs` → `event_capacity.rs` (`SseCapacity` →
`EventConnectionCapacity`, `SseSlot` → `EventConnectionSlot`,
`SseAcquireResult` → `EventConnectionAcquireResult`; keep constants),
`handlers.rs` `streaming` owner refactor, `webui_v2/schema.rs` (codec entry
point `WebChatV2EventFrame::from_stream_event(cursor, payload)`).

- [ ] `ProductStreamDriver::run(surface, caller, selector, after_cursor,
  sink, lifetime_budget)` — extracts today's `build_sse_stream` loop
  (drain → subscription inner loop → idle poll fallback → error), yielding
  typed `ProductStreamEventEnvelope` batches + terminal error; cursor advance
  owned here.
- [ ] `WebUiSessionEventCodec`: `Thread(payload)` → existing
  `WebChatV2Event` mapping (same wire names/`id` rules — keep-alive carries
  no cursor id); rejects (drops + debug-logs) events that fail serialization.
- [ ] Rewrite the SSE generator over driver+codec; wire bytes unchanged —
  existing `webui_v2_handlers_contract.rs` streaming tests are the parity
  proof and must pass unmodified (except type renames).
- [ ] Checks: `cargo test -p ironclaw_webui --all-features` (streaming tests
  green), `cargo clippy -p ironclaw_webui --all-targets --all-features -- -D warnings`.

### Task 0.3 — ticket store + mint route + session WebSocket

**Files:** `crates/contracts/ironclaw_product_contracts/src/session_transport.rs`
(new), `crates/product/ironclaw_webui/src/session_socket_tickets.rs` (new;
in-memory adapter), `crates/app/ironclaw_composition/src/session_ticket_store.rs`
(new; secret-store adapter),
`crates/product/ironclaw_webui/src/webui_v2/handlers/session_events.rs` (new
handlers), `descriptors.rs` (+2 routes),
`webui_serve.rs` (`WebuiServeConfig::with_session_socket_ticket_store`,
auth-middleware ticket path), `webui_v2/router.rs` (state), `handlers.rs`
(`WebUiV2Features.session_events` + `get_session`),
`crates/app/ironclaw_cli/src/commands/serve.rs` (wiring by storage shape),
`crates/contracts/ironclaw_host_api/src/ingress.rs` (only if a ticket auth
scheme variant is needed), tests: `tests/webui_v2_descriptors_contract.rs`,
`tests/webui_v2_handlers_contract.rs`, `tests/auth_route_contract.rs`,
`src/webui_rate_limit_router_contract_test.rs`,
`crates/app/ironclaw_composition/tests/webui_v2_serve.rs`.

Routes:

| id | method | pattern | policy |
|---|---|---|---|
| `webui.v2.session_websocket_ticket` | POST | `/api/webchat/v2/session/websocket-ticket` | bearer, `Limited{12/60 PerCaller}`, 4 KiB body, ProjectionOnly, audit Mutation-class |
| `webui.v2.session_websocket` | GET | `/api/webchat/v2/session/websocket` | ticket-authenticated, `SameOriginRequired`, `StreamingMode::WebSocket`, NoBody, 30/60 PerCaller, ProjectionOnly, StreamingSubscription |

- [ ] Tests first: ticket mint→consume single-use; replay rejected; expiry
  rejected; exact caller binding (record fields equal minting caller);
  in-memory bound + eviction; upgrade without ticket/with consumed ticket →
  401; `?token=` still never authenticates a WS upgrade (adapt
  `query_token_rejected_on_websocket_route`); same-origin 403s (repoint
  composition `ws_upgrade_*` tests at the session route).
- [ ] Session WS handler: authenticate via single-use ticket consumed in the
  bearer middleware (session-socket path recognizer alongside
  `is_v2_sse_event_request`; injects the same three extensions); acquire one
  `EventConnectionCapacity` slot; then the socket task:
  - control-frame parse with hard bounds (≤8 KiB, unknown `type` → typed
    error + close);
  - per-subscription: authorize selector via a first driver call before
    admission-swap; connection-scoped monotonically increasing `generation`;
    replacement authorizes first, then atomically swaps and cancels the old
    generation (stale generations drop frames at the writer);
  - per-subscription bounded queue (16 batches) + 256 aggregate budget,
    round-robin fair drain, `subscription_error{…, last_cursor}` +
    generation cancel on per-sub overflow/lag (socket stays open); aggregate
    backpressure/send-timeout closes the socket;
  - 5-minute lifetime → `reconnect_hint` + close; ping→pong.
- [ ] Mint handler: bearer caller → nonce → store.mint; response
  `{ticket, expires_in_ms, socket_path}`. Never logs the nonce.
- [ ] Feature advertisement: `WebUiV2Features.session_events = true` only
  when a ticket store is wired; serve.rs picks the adapter by
  `DeploymentConfig::storage_shape()` (LocalFilesystemRoot → webui in-memory;
  pooled/operator-supplied → composition secret-store adapter; none → SSE
  only). Composition read-back pattern (`effective_workspace_scoping`).
- [ ] Contract tests: no-mutation proof — a fuzz-shaped list of
  operation-ID/turn-submission/gate frames over the socket produce only a
  typed protocol error and zero `ProductSurface::invoke` calls on the
  recording surface; two selectors resume independently (cursor isolation);
  stale-generation frames never delivered; lagged subscription isolated.
- [ ] Checks: webui crate suite + descriptors contract + composition
  `webui_v2_serve.rs` + `cargo test -p ironclaw_architecture_tests`.

### Task 0.4 — remove the dormant per-thread WebSocket

**Files:** `handlers.rs` (delete `stream_events_ws`, `ws_drain_loop`,
`ws_send_with_timeout`), `descriptors.rs` (row + const), `webui_v2/mod.rs`
re-exports, `frontend/src/lib/api.ts::openEventSocket` (delete),
CONTRACT.md route table + streaming section + charter `streaming` row,
tests listed in recon (descriptors table row, raw-envelope WS tests deleted;
capacity-sharing test rewritten against the session socket).

- [ ] Keep ≥1 WS descriptor invariant satisfied by the session route (ws-origin
  state test), keep SSE untouched as the rollback adapter.
- [ ] Checks: full webui suite + descriptors contract + charter test.

### Task 0.5 — SPA `SessionEventClient` + chat migration

**Files:** new `frontend/src/lib/session-events/client.ts` (+ `protocol.ts`,
`client.test.ts`), new `frontend/src/pages/chat/hooks/useThreadEvents.ts`
(+ test), `frontend/src/app/auth.ts` (+`sessionEventsEnabled`),
`frontend/src/layout/gateway-layout.tsx` (client mount),
`frontend/src/pages/chat/hooks/useChat.ts` (swap `useSSE` for
`useThreadEvents`), `frontend/src/lib/api.ts` (`mintSessionSocketTicket`).

- [ ] `SessionEventClient`: one socket per authenticated page; mint→connect;
  jittered bounded backoff (reuse useSSE constants), online/offline handling;
  **stays connected while hidden** (that is the point of the session socket);
  lifetime-expiry reconnect on `reconnect_hint`/close; per-subscription
  cursor tracking + resubscribe-on-reconnect from each cursor;
  `subscription_error` → per-subscription rebase callback; generation
  filtering; transport-independent `subscribe(selector, {onEvent, onError,
  fromCursor})` returning a handle.
- [ ] `useThreadEvents`: uses the session client when
  `features.session_events` and the socket is healthy; falls back to the
  untouched `useSSE` otherwise (rollout/rollback §16). Frame payloads are
  identical `WebChatV2EventFrame` bodies, so `useChatEvents` is unchanged.
- [ ] Vitest: protocol encode/decode, cursor-per-subscription resume, stale
  generation ignored, fallback selection, hidden-page persistence.
- [ ] Checks: `pnpm --dir crates/product/ironclaw_webui/frontend test`,
  `pnpm --dir … lint`, bundle budget via `pnpm --dir … build`.

### Task 0.6 — Phase 0 integration + E2E evidence

**Files:** new `tests/integration/session_events.rs` (bin
`reborn_integration_session_events`), extend
`tests/integration/webui_v2_product_api.rs`; new
`tests/e2e/scenarios/test_reborn_webui_v2_session_socket.py`;
`tests/CLAUDE.md` rows; workspace `Cargo.toml` `[[test]]`.

- [ ] Integration: HTTP thread create + submit → session-socket Thread
  subscription streams cumulative sanitized text (multi-phase) ending with
  the exact durable finalized reply; two logical subscriptions resume
  independently; mutation frames rejected without reaching `invoke`;
  mint/consume race has one winner (concurrent consumes of one nonce);
  SSE and session-socket browser payload equivalence through the shared codec.
- [ ] E2E (Playwright): send question over HTTP, watch incremental text +
  exact final reply over the socket; navigate away/back resumes without
  duplicates; socket kill → ticket refresh → per-selector resume; SSE
  fallback when the feature flag is off.

## Phase 1 — event-driven in-app completion notices

### Task 1.1 — notice store + sequence + tests

**Files:** new `crates/product/ironclaw_assistant/src/run_completions/`
(`mod.rs`, `store.rs`, `records.rs`), `reborn_services.rs` (`mod` decl),
assistant `AGENTS.md` charter rows, `ironclaw_composition/src/lib.rs`
(`/run-notices` alias), `docs/internal/reborn/contracts/storage-placement.md`
§4 row.

- [ ] Store API: `create_notice` (idempotent by notice_id; allocates the next
  owner sequence via CAS counter; returns Created|Existing),
  `transition` (typed CAS state-machine steps from §5.3 — each transition
  function validates the from-state), `mark_read` (evidence + settle),
  `unread_snapshot(limit=250)`, `list_after(sequence, limit)`
  (query_ordered), `pending_due / granted_due / push_owned` scans for boot
  reconciliation (indexed by state key), `record_intent`
  (≤32/notice, newer revision replaces same browser profile).
- [ ] Conformance-style tests on `InMemoryBackend` + libSQL (temp dir):
  idempotent create, sequence monotonicity, CAS conflict single-winner,
  read/delivery orthogonality, snapshot bound 250, restart re-open reads
  identical state; illegal transitions rejected.

### Task 1.2 — ingest + journal observer + wiring

**Files:** `run_completions/ingest.rs` (assistant), composition
`run_completion_observer.rs` + registration in
`build_runtime_with_resource_governor` (after `processes` handle exists),
`RebornRuntime` field + accessor for the notice services.

- [ ] Ingest: eligibility (§1.1) — thread visibility probe
  (`read_thread` metadata probe with owner scope), finalized reply resolve
  (`finalized_assistant_message_by_run`); `NoFinalReply` → anomaly counter +
  `Ok` (cursor advances); store failures → retryable error; duplicate →
  `AlreadyRecorded` + coordinator wake.
- [ ] Integration (extend `webui_v2_product_api.rs` or the new session bin):
  production-wired completed top-level turn creates exactly one notice;
  subagent/failed/cancelled runs create none (drive via harness scripts).

### Task 1.3 — RunCompletions selector + user stream

**Files:** product_contracts (`run_completions.rs` wire module + selector/
event variants), assistant `run_completions/stream.rs` (per-owner broadcast
hub + snapshot/replay + admission bounds + lag→rebase),
`reborn_services.rs` stream dispatch (`RunCompletions` arm authorizes
caller-owner scope; no thread probe; no caller-selectable identity),
webui codec RunCompletion arm → browser frames, session client
`run-completions` subscription in the SPA layer (store only, UI next task).

- [ ] Cursor = decimal sequence string, independent of thread cursors; old/
  invalid cursor or lag → bounded rebase snapshot (unread+unsettled ≤250)
  then live resume; foreign selector cursor rejected (a thread cursor cannot
  resume RunCompletions — typed cursor namespace prefix `rc:`).
- [ ] Contract tests: cross-user isolation (foreign caller sees nothing /
  NotFound), redaction (wire event contains only IDs/timestamps/counts —
  assert serialized JSON field allowlist), rebase behavior, independent
  failure domains on one socket (thread lag doesn't advance/reset completions).

### Task 1.4 — HTTP operations + unread view

**Files:** product_contracts descriptors, assistant
`run_completions/operations.rs` + dispatch registration (+charter),
webui `handlers/session_events.rs` or new `handlers/run_completions.rs`
(3 POST routes + unread usage via existing query route machinery),
`descriptors.rs` (+3 routes), descriptors contract rows, CONTRACT.md tables.

Routes: `POST /api/webchat/v2/notifications/run-completions/intents` ·
`…/acknowledgements` · `…/thread-read` (bearer, 60/60 PerCaller, 4 KiB
bodies, ProductSurface effect path).

- [ ] Server validation: foreign notice → `NotFound`; `local_os` intents
  validated against live web-app target selection + host-owned enrollment
  (Phase 2 completes this — Phase 1 rejects `local_os` as unavailable);
  thread-read advances only owned notices for that thread at/below the
  supplied sequence with finalized replies; acknowledgements are
  grant_id+revision-checked CAS transitions; every write idempotent
  (duplicate returns existing outcome).

### Task 1.5 — coordinator (arbitration core) + grants/clears

**Files:** assistant `run_completions/coordinator.rs` (+`spawn` module),
composition spawn + `RebornRuntime` handle + `shutdown` ordering.

- [ ] State machine per §5.3/§5.4: notice write wakes due queue; 1 s intent
  window; ranking per §5.6 (reply_observed > watching_thread > in_app >
  local_os > unavailable; revision → focus epoch → lexicographic tie-break);
  grants (`NoSurfaceWatchingThread`/`InApp` in P1) with 2 s ack timeout; one
  re-arbitration on expiry/stale_state; read evidence settles anywhere in the
  flow and emits `Clear`; timers via injected clock (`now: fn() ->
  Timestamp` + tokio time; tests run paused-clock).
- [ ] Grant/clear events published through the stream hub after their CAS
  transitions commit.
- [ ] Unit tests: full matrix rows that don't need a browser (§6.1 rows 1–8,
  16–17 equivalents) against a fake clock.

### Task 1.6 — service worker arbitration (focused cases) + in-app UI

**Files:** `frontend/public/sw.js` (message protocol, IndexedDB ledger,
tab-state registry, intent proposals, grant application, clear handling),
new `frontend/src/lib/run-completions/` (`worker-bridge.ts`, `store.ts`,
`useRunCompletions.ts`, tests), `frontend/src/pages/chat/` reply-render
evidence (`thread_read` on finalized-reply render), toast/badge UI wired into
`GatewayLayout` (+ notification-center section), i18n keys (all locales),
click-through navigation, `tab_id`/`browser_instance_id` generation
(crypto.randomUUID; instance id persisted in localStorage, correlated
host-side at enrollment time in Phase 2).

- [ ] Page↔worker protocol (postMessage): `tab_state` reports (§5.5 shape,
  ≤128 observed run ids), `notice`/`grant`/`clear` forwards, worker→page
  `propose_intent` + `apply_in_app` + `effect_result`; IndexedDB
  test-and-set ledger by notice_id (memory fallback), ≤250 entries LRU.
- [ ] Focused-thread suppression: focused visible tab on T with R rendered →
  `reply_observed` intent → server settles → no UI; focused elsewhere →
  `in_app` → one toast in one deterministic tab + grouped badge; unread
  retained until thread render → `thread_read` → clear everywhere.
- [ ] Vitest: reducer/store logic, ledger TAS dedupe, evidence emission
  (vm harness); sw.js logic factored into testable pure helpers where
  practical while keeping the file dependency-free.

### Task 1.7 — Phase 1 integration + E2E

- [ ] Integration: completed turn → notice on the user stream; focused-thread
  read evidence settles with zero outbound attempts; focused-elsewhere intent
  → one in-app grant, notice stays unread until thread-read; duplicate
  intents/acks return existing outcomes; restart replays unread snapshot.
- [ ] E2E: two tabs (one on T) → no duplicate presentation; toast
  click-through; badge clear on read; multi-run same-thread grouping.

## Phase 2 — local OS presentation

### Task 2.1 — `local_os` validation + grants + expiry fallback

- [ ] Server: `local_os` intent validation = web-app target selected
  (notification-channel resolution) + browser instance enrolled
  (DeliveryRegistrationService list, instance↔registration correlation
  recorded at enrollment via a `browser_instance_id` echo field on the
  enrollment payload — additive, opaque); grants carry surface `local_os`;
  grant expiry → one re-arbitration → (Phase 3: push).
- [ ] SW: `LocalOs` grant → recheck clients → `showNotification` with fixed
  copy, `tag = thread_tag`, generic bounded count body when >1; effect ack
  over HTTP; `stale_state` on revision/client mismatch; clear closes
  `getNotifications({tag})`.
- [ ] Permission/enrollment UX: reuse `enrollThisBrowser`; non-modal
  first-toast CTA ("Notify me when I'm away") gated on `default` permission;
  never prompt programmatically otherwise; `denied` → settings guidance only.
- [ ] Tests: hidden-tab throttling (no intent → window closes → P3 fallback
  path stubbed to NoExternalTarget in P2), grant-expiry re-arbitration,
  permission-state matrix in vitest; integration rows for local_os grant
  issuance + ack.

## Phase 3 — Web Push fallback

### Task 3.1 — outbound vocabulary + capability

**Files:** `ironclaw_outbound` (`types.rs`, `delivery_resolution.rs`,
`store.rs`, `delivery_targets.rs` + capability tests),
`product_wire.rs` twin + mapper, `packages/web-app/src/targets.rs`
(`run_completions: true` + test), extension_contracts (`OutboundPart::
RunCompletion` + view + `render_channel_run_completion` fixed-copy helper),
slack/telegram contract tests proving the part reports unsupported
(`Permanent`) without entering shared conformance envelopes.

### Task 3.2 — push path: coordinator → candidates → policy → delivery

**Files:** assistant `run_completions/push.rs` (facade mirroring
`run_delivery::notifications::notify`): resolve effective channels →
structural `run_completions` filter (yields ≤1 web-app target even with
Slack+Telegram selected) → `EventStreamManager::push_candidates_for_update`
(kind `RunCompletion`, thread projection scope/view/target, capability-
filtered binding as `reply_target`, `projection_ref =
"run-completion/{notice_id}"`) → `DeliveryCoordinator` typed completion
entry (route `Delivery`, enrollment-required, parts
`[OutboundPart::RunCompletion(view)]`) with stable attempt id + claim CAS;
coordinator transition `PendingArbitration → PushOwned{delivery_id}` before
egress; `NoExternalTarget` settle when nothing resolves.

- [ ] Web-app adapter arm renders the §7.10 payload; 404/410 pruning and
  attempt lifecycle already generic; push failure never mutates run state
  (notice stays `PushOwned` with failed attempt evidence; run untouched).
- [ ] SW push handler: v2 payload parse (v1 still handled), ledger dedupe by
  notice_id, tag collapse + `renotify:false`, click focuses existing client
  (prefer one already on `/chat/T`) else navigates one else `openWindow`;
  session-expiry → login with validated same-origin return path; revoked
  thread → generic unavailable state (existing router behavior + test).
- [ ] Integration (extend `delivery_user_journeys.rs` with explicit opt-in
  completion wiring in the harness): no-intent + selected/enrolled → exactly
  one authorized durable attempt with encrypted push POST;
  Slack+Telegram+web-app selected → only web-app candidate; empty set /
  no enrollment / revoked access → zero egress + sanitized state; `410 Gone`
  prunes after a completion push; push transport failure leaves the run
  `Completed` and the notice recoverable.
- [ ] Redaction proof: capture the push plaintext in the fake push service —
  assert exact field set + fixed copy; no reply text/title/actor/project
  names anywhere in payload or logs.

## Phase 4 — recovery + cross-device hardening

### Task 4.1 — boot reconciliation + CAS races + clears + metrics

- [ ] Coordinator startup: one bounded scan over
  pending/granted-expired/push-owned; re-arm timers from `closes_at`/
  `expires_at`; crash-after-PushOwned recovers the same stable delivery id
  (attempt id derivation is deterministic); crash in Prepared/Sending follows
  existing outbound semantics (tests via restartable LibSql group harness).
- [ ] Multi-replica: two coordinator instances over one store — exactly one
  grant/push owner per notice (CAS loser observes and stands down);
  ticket mint/consume across two store handles → one winner (Postgres leg via
  existing testcontainers matrix where available).
- [ ] Cross-device clear: read on device A emits Clear; connected device B
  dismisses; sleeping worker clears on next wake (SW consults ledger+event).
- [ ] Observability: `debug!`-level counters (pending age, intent/grant
  outcomes, fallbacks, stale grants, dedupe hits, anomaly count) with no
  titles/payloads/endpoints/navigation in fields; doc note for mobile-PWA
  validation left explicitly unclaimed (§ non-goals).

## Cross-cutting: gates and docs checklist (do continuously, verify at end)

- [ ] `webui_v2_descriptors_contract.rs` table (net +5 routes, −1)
- [ ] webui CONTRACT.md: route table, streaming section rewrite (session
  socket + compat SSE), charter rows (`session-events` owner; `streaming`
  row updated); handlers waiver lines intact
- [ ] assistant AGENTS.md charter rows for every new `reborn_services`-tree item
- [ ] `reborn_transport_product_boundary.rs`: expect **no** baseline change
  (all new vocabulary in product_contracts); verify residue stays 103
- [ ] product_contracts size ceiling re-pin (+dated note) if >16,317 lines;
  extension_contracts if >8,922
- [ ] composition budget: measure merged tree via
  `bash scripts/ci/check-composition-budget.sh --print`; re-pin
  `loc_ceiling`/`arc_dyn_ceiling` + `reborn_restructure_baselines.rs`
  (`COMPOSITION_ABSOLUTE_SRC_LOC`, `WS0_COMPOSITION_ARC_DYN_SITES`) same commit
- [ ] `composition-pubuse.snapshot` only if a new `pub use` is added (avoid)
- [ ] same-layer edge inventory: expect unchanged (71)
- [ ] `tests/CLAUDE.md` rows for every new/changed scenario;
  `coverage-floor.toml` recapture for `ironclaw_assistant`/`ironclaw_outbound`
  denominators (numbers from this PR's CI ratchet output)
- [ ] `storage-placement.md` §4 row; `.env.example` untouched (no new env)
- [ ] `python3 scripts/ci/docs_publication_boundary.py`
- [ ] retired-vocabulary scans clean (`web_push` spelling, taxonomy terms)

## Final verification (before PR)

```
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo clippy -p ironclaw_webui -- -D warnings          # prod-shape lane
cargo clippy -p ironclaw_assistant -- -D warnings
cargo test
cargo test -p ironclaw_architecture_tests
pnpm --dir crates/product/ironclaw_webui/frontend test && pnpm --dir … lint && pnpm --dir … build
python3 scripts/ci/docs_publication_boundary.py
bash scripts/ci/check-composition-budget.sh
tests/e2e: session-socket + notifications scenarios (Docker + Playwright)
```

Plus the finishing sweeps from AGENTS.md: unwrap/expect scan over changed
production files, lost-cause `map_err` scan, byte-slicing scan, hardcoded
temp paths, secrets/PII in logs and payloads, trait-impl enumeration for
every trait change (`ProductSurface` DTOs: all impls/doubles updated), and
old-path search after renames (`sse_capacity`, `stream_events_ws`,
`openEventSocket`).

## Rollback story (must remain true at every commit)

- Session socket off (feature not advertised / ticket store absent) → SPA
  uses compatibility SSE; no browser contract change (shared codec).
- Notifications off (observer/coordinator not wired) → notice records inert;
  gate/auth/failure notifier and all existing delivery behavior unchanged.
- All wire additions versioned/additive; `run_completions` capability
  defaults false everywhere; SW ignores unknown payload schemas.
