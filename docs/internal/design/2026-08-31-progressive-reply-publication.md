# Progressive reply publication — one reply projection, one publication owner, one sink seam

**Status:** implemented on this branch (2026-08-31); see §11 for the
verification that backs each claim. This document is the owning design for
the work on this branch. It describes the consolidated ownership model
that superseded an earlier draft of this same document, which had proposed a
second reply journal (`ironclaw_replies`, a `/replies` mount) and a separate
publication service. Those were withdrawn: they duplicated history the event
system already retains and machinery the delivery coordinator already owns.

Supersedes the "verify, do not own" ruling in
`2026-08-11-channel-adapter-contract.md` §4.4 and the
`ReplyTransport::Stream` = "host projection, adapter absent" semantics that
ruling produced. Builds on `2026-08-10-unified-channel-model.md` §5 ("the
reply sink sits *on top of* the durable event/projection pipeline — a
consumer of durable reply events, never a replacement") and
`agent-activity-streaming.md` (the WebUI activity UX this feeds).

> File/symbol references are a point-in-time trace. Re-verify with
> `rg -n "ReplySink|ReplyDocument|reply_publication|ReplyPublicationState" crates`.

---

## 0. The rules this document exists to make obvious

1. **One canonical history.** Reply state is a projection over the durable
   facts the system already retains — the process journal's turn lifecycle
   events, the thread transcript, approval/auth records, and the runtime
   event log. No second reply journal. Partial answer text, reasoning
   summaries, driver status, and display previews are *ephemeral by the
   frozen events contract* (`docs/internal/reborn/contracts/events-projections.md`
   §5–§6); they ride the in-process live path for latency and are never the
   source of correctness, replay, or delivery evidence.
2. **One safe reply projection.** `ironclaw_assistant` owns the only
   composer/reducer from facts to the provider-neutral `ReplyDocument`
   (disclosure policy applied per audience, bounded display text only). The
   same module rebuilds the document from durable facts after a restart.
3. **One publication owner.** `DeliveryCoordinator` + `OutboundStateStorePort`
   are the sole writers of publication state — lease, fence, desired and
   published revisions, sink checkpoint, provider evidence, settlement — as
   a serde-defaulted substate of the existing outbound delivery attempt
   aggregate. No second attempt store, retry scheduler, or evidence system.
4. **One sink seam.** Every channel that declares `[channel.reply]` binds one
   `ReplySink`. `ReplyTransport` selects cadence only: `stream` receives
   progressive revisions, `message` receives the terminal materialization.
   WebUI, Slack, Telegram, and every other channel differ only after that
   seam. `ChannelReply` (the `OutboundEnvelope` final-answer half) is
   removed at cutover; no legacy final-reply pipeline remains.
5. `[channel.delivery]` stays orthogonal: out-of-band notification delivery
   and source-routed system notices ride `DeliveryCoordinator::deliver` /
   `deliver_notice` on the delivery half.

## 1. Problem (verified on `origin/main` `ea111c67b9`)

- WebUI progress was process-local: `LiveProgressMilestoneSink` →
  `InMemoryProjectionUpdateSource` under synthetic, epoch-scoped cursors;
  only the finalized transcript row, run status, gates, and capability
  activity were restart-safe.
- `DeliveryCoordinator::record_stream_reply` re-drained that projection to
  "verify" a browser reply and wrote a `Delivered` attempt from it, never
  calling an adapter; `check_binding` required a `stream` channel to bind
  **no** reply half; `ChannelDescriptor::validate` coupled `stream` to
  authenticated-session ingress. Streaming was a WebUI privilege.
- Slack received one terminal message from `RunDeliveryObserver`'s poll loop
  plus a retract-and-repost "working" notice and 👀/✅/⚠️/❌ reactions;
  Slack's native Agent surface was unreachable.
- Session-lane (browser) turns had no `ChannelRunDeliveryObserver`, so
  browser replies produced no delivery record at all.

## 2. Ownership

| Concern | Owner | Status |
| --- | --- | --- |
| Provider-neutral reply vocabulary: `ReplyDocument`, `ReplyChange`, `ReplyRevision`, `ReplyTarget`, `ReplySink`, bounded newtypes, checkpoint/evidence/report types, `ReplyTransport::reconciles_at` | `ironclaw_extension_contracts::reply` (+ `channel`) | implemented |
| `[channel.reply]` binding rule: a declared reply transport requires a real bound `ReplySink`; no fake bridge half satisfies it | `ironclaw_extension_host::entrypoint::check_channel_halves`, `generic_host.rs` | planned (in this branch) |
| The reply projection: milestones → safe changes → document; rebuild from durable facts; disclosure policy | `ironclaw_assistant::reply_projection` (consolidates `projection/live_progress.rs`) | planned |
| Publication: per-target worker, coalescing, lease/fence, retries, heartbeat, terminal settlement, sink/egress resolution | `ironclaw_assistant::delivery_coordinator` (a `reply_publication` submodule of the same owner) | planned |
| Publication state persistence: guarded CAS operations on the attempt aggregate | `ironclaw_outbound::OutboundStateStorePort` / `OutboundStateStore` | planned |
| WebUI edge: revision → live projection items over the existing `LiveProjectionPublisher`; durable facets reach the browser through the existing turn-event/runtime projection | `ironclaw_assistant::projection::reply_sink::ProjectionReplySink` | partially implemented (live facets: answer, reasoning, activity, status) |
| Slack edge: native Agent session/stream/task rendering, error and rate-limit mapping, read-back | `crates/extensions/packages/slack/src/reply_sink.rs` | planned |
| Telegram edge: terminal materialization (answer + attachments, failure/cancel copy) | `crates/extensions/packages/telegram` | planned |
| Generation resolution, mediated egress, bounded `Retry-After` hint | `ironclaw_extension_host` (`SnapshotChannelDeliveryResolver`, `channel_egress.rs`) | hint field implemented; host parsing planned |
| Wiring only: session-channel target, sink binding, worker supervision | `ironclaw_composition`, the `ironclaw` binary | partially implemented (`ProjectionReplySink` bound + publisher slot) |

Composition names no extension. The binary binds `ProjectionReplySink` to the
web-app extension exactly as it binds Slack's adapter to `slack`.

## 3. Canonical history — the durable source of every document field

| `ReplyDocument` field | Durable source (rebuildable after restart) | Live enrichment (latency only) |
| --- | --- | --- |
| `phase` | `WaitingForInput` ← process-journal turn event `Blocked`; `Completed`/`Failed`/`Cancelled` ← turn events `Completed`/`Failed`/`Cancelled` (`TurnEventProjectionSource`) | `Thinking`/`Working` ← `ModelStarted`, `CapabilityInvoked` milestones |
| `status` | none — driver notes are ephemeral by contract | `DriverNote` milestones (`LoopSafeSummary`) |
| `answer.text`, `answer.finalized` | transcript row `SessionThreadService::finalized_assistant_message_by_run` (retained forever) | `ModelTextDelta` milestones until `AssistantReplyFinalized` |
| `reasoning` | none — model-visible-sanitized reasoning summaries are live UI hints by contract; never persisted | `ModelReasoningDelta` (Private audiences only) |
| `activities` (id, title, state) | runtime event log `CapabilityActivity{Requested,Succeeded,Failed}` via `ReplayEventProjectionService` (durable, write-behind best-effort) | `CapabilityInvoked/Completed/Failed` milestones |
| `activities[].detail / output_preview` | none — `CapabilityDisplayPreviewStore` is process-local | display-preview source |
| `attention` | turn event `Blocked` (gate kind, gate ref) + `ApprovalRequestStorePort`/`ApprovalPromptContextSource` (approval copy) + `BlockedAuthPromptSource` (auth challenge; URL disclosed only to Private audiences) | same facts, observed through the milestone `Blocked` |
| `attachments` | transcript row attachments (`AttachmentRef`, bytes in the project filesystem) | — |
| `outcome` | turn terminal events + sanitized failure category/summary (`run_failure_summary`, `FailureExplanationProvider`) | `Completed`/`Failed` milestones |

Consequences, stated plainly: after a restart mid-run the run itself does not
survive (lease expiry produces a durable `Failed`), so the terminal reply is
always reconstructable — finalized answer, attachments, gate state, outcome
— while partial text, reasoning, status, and previews are not, by design. No
LLM data is deleted: the transcript, turn events, runtime events, and
approval/auth records are retained exactly as today.

## 4. The reply projection *(implemented — `ironclaw_assistant::reply_projection`)*

```text
durable turn/runtime fact  ─┐
live milestone             ─┼─► apply disclosure/safety policy ─► ReplyChange ─► ReplyDocument::apply ─► ReplyRevision
approval/auth context      ─┘
```

- One module (`ironclaw_assistant::reply_projection`) holds the composer
  (milestone/fact → `ReplyChange`, reusing `sanitize_model_visible_text`,
  `LoopSafeSummary`, `CapabilityDisplayPreviewSource`, and the existing gate
  prompt composition), the reducer (`ReplyDocument::apply`, in contracts), and
  `rebuild(scope, run)` from the durable sources in §3.
- The `LoopHostMilestoneSink` decorator position formerly held by
  `LiveProgressMilestoneSink` feeds this module; skill-activation live items
  stay on `LiveProjectionPublisher` unchanged.
- Disclosure: `ReplyAudience::Private` (authenticated session, direct chat)
  keeps reasoning summaries, previews, and attention action URLs;
  `ReplyAudience::Shared` (channels/groups) strips them. Audience is decided by
  the host from the trigger class and conversation model at target
  registration.
- Item identities: activity id = invocation uuid, attachment id = attachment
  id, answer = one row per run; so every edge can upsert rather than
  duplicate.
- As built: `ReplyProjection::observe_milestone` composes; `ModelTextDelta`
  is the *cumulative* text of the current model call, so the projection
  tracks finished-call text per run and emits `AnswerAppended` for growth
  and `AnswerRewritten` for anything else (the contract gained
  `AnswerRewritten`, `ReasoningAppended` — the open reasoning segment a
  `ReasoningSummary` closes — a `work: ReplyStatusKind` hint on
  `StatusSummary`, and `ReplyActivityProvenance` on finished activities so
  the WebUI keeps provider/runtime/output-size badges). Terminal facts come
  only from `apply_terminal_facts` (durable transcript row + committed run
  status; `nothing_to_report` publishes an empty terminal revision).
  `raise_revision_floor` lets a publisher that resumes a run on another node
  number the rebuilt terminal revision above what the store already saw.
  `ReplyProjectionMilestoneSink` is the decorator (`LiveProgressMilestoneSink`
  and its text coalescer are deleted); `disclose_for_audience` is applied to
  the copy each target receives.

## 5. Publication on the attempt aggregate *(implemented — `ironclaw_outbound::reply_publication`, `DeliveryCoordinator` ops, `ironclaw_assistant::reply_publication`)*

`OutboundDeliveryAttempt` today models a one-shot send (`Prepared → Sending`
claimed once; a `Sending` row found after a crash becomes `Unknown`;
`Delivered`/`Failed`/`NoTarget` terminal). Progressive publication performs
many reconciliations over time, so it does **not** reuse the send claim as a
lease. The persisted row gains a serde-defaulted `publication` substate and
the store gains guarded operations:

```text
ReplyPublicationState {
  target: { reply run id, exact target key },            // identity
  fence: u64, lease: Option<{ owner, expires_at }>,      // ownership
  desired_revision, published_revision,                  // monotonic
  terminal_revision: Option<u64>,                        // set when the terminal desired state is known
  generation: Option<u64>, checkpoint: Option<ReplySinkCheckpoint>,   // generation-pinned adapter state
  evidence: { provider_refs (≤32×256 B), read_back_verified, last_outcome, generation_changed },
  status: Active | Settled(Delivered | Unknown | Failed(kind))
}
```

Operations (all CAS on the row; each rejects a stale fence):
`open_reply_publication` (idempotent by reply + exact target; a different
target under the same id is a conflict) · `claim_reply_publication_lease`
(acquire when unleased/expired, bump fence, `Prepared → Sending` once) ·
`renew_reply_publication_lease` · `advance_reply_publication` (published
revision monotonic and ≤ desired; checkpoint/evidence/generation recorded;
refused once settled) · `settle_reply_publication` (`Delivered` only when the
terminal revision was applied; `Unknown` for an unverifiable terminal
reconcile; `Failed(kind)` for permanent/unauthorized/abandoned). Crash
recovery: `recover_interrupted_delivery_attempt` leaves publication rows
alone — they recover through lease expiry and takeover, never by being marked
`Unknown`. Wake-up after a crash: the existing `ProcessJournalCommitObserver`
on terminal run commits plus lease expiry; no outbox.

Worker rules (inside the coordinator family): one task per `(reply, target)`;
different replies publish concurrently; replaceable revisions coalesce under
backpressure; control-critical and terminal revisions are reconciled
individually; `Retryable` honours `retry_after` or exponential backoff on the
same revision; `Ambiguous` is recorded as `Unknown` evidence and retried only
with the sink's own read-back checkpoint; `Permanent`/`Unauthorized` settle
`Failed`; `StoppedByUser` cancels the run and settles. Provider latency never
touches the model path: the milestone sink hands a revision to the worker
mailbox and returns.

As built (deltas from the sketch above): the substate also persists a
`ReplyPublicationTargetDescriptor` (channel, actor, authorized reply-target
binding, vendor conversation, thread anchor, audience, transport) so any node
can resume a target from the row alone; the coordinator opens the row through
the store in one write (`open_reply_publication` creates attempt + substate
together — a plain attempt row is a different aggregate). Worker: one tokio
task per `(run, exact target)`, woken by the projection; it always publishes
the latest snapshot (natural coalescing) under `min_progress_interval`
pacing for `Progress`, never delays `ControlCritical`/`Terminal`, reconciles
idle `stream` targets at `Heartbeat` every `heartbeat_interval` (20 min
default — Slack sessions expire after an hour), persists the checkpoint a
sink returns with `Retryable`/`Ambiguous` outcomes (so a retry resumes the
provider presentation instead of opening another), classifies
`ChannelError` (transfer faults retryable; parse/render/config permanent),
materializes workspace attachments only for the terminal reconcile under the
attachment budgets (missing/denied → `Failed(Rejected)`, unavailable reader →
retried then `Failed(TransportUnavailable)`), and settles `Unknown` after the
terminal attempt budget when the provider stays ambiguous. Graceful shutdown
releases held leases; a crash lets them lapse. Wake-up after a crash is the
`ReplyPublicationCommitObserver` on terminal process-journal commits
(`run_terminal` → resume open publications from the store + fetch terminal
facts). The observer also calls `run_terminal` when it polls a terminal state
and waits (bounded, 20 s) for settlement before retracting its working
indicator, so a `message` channel never shows a gap between indicator and
answer.

## 6. The sink seam *(implemented in contracts; sinks: projection, Telegram, Slack Agent, acme fixture)*

```rust
pub trait ReplySink {
    async fn reconcile(&self, request: ReplyReconcileRequest, egress: &dyn RestrictedEgress)
        -> Result<ReplySinkReport, ChannelError>;
}
```

`ReplyReconcileRequest` = desired `ReplyRevision` + `ReplyReconcilePoint`
(`Opened | Progress | ControlCritical | Terminal | Heartbeat`) + `ReplyTarget`
(scope, actor, run, optional vendor conversation + bounded thread anchor,
audience) + bounded `reply_context` + the previous `ReplySinkCheckpoint` +
extension generation + (terminal only) materialized attachments.
`ReplySinkReport` = `Applied | Retryable{retry_after} | Ambiguous | Permanent |
Unauthorized | StoppedByUser` + next checkpoint + bounded evidence. Reasons,
provider refs, checkpoints, anchors, and reply context are bounded by
construction (newtypes), not by optional validation.

Cadence: `ReplyTransport::Stream.reconciles_at(_) == true`;
`ReplyTransport::Message.reconciles_at(point) == (point == Terminal)`.

## 7. Cutover inventory — `RunDeliveryObserver`

| Existing behavior | After cutover |
| --- | --- |
| Pre-run notices in `observe_ack` (command feedback, rejection hint, connect nudge, busy hint), `observe_error`, `post_connection_status_notice` | retained; source-routed notices ride `deliver_notice` on the **delivery half** |
| Single-flight per run (`DeliveryRunLedger`) | retained for the observer's own loop |
| `FinalReply` (text + attachments) | **replaced**: terminal reconcile of the bound `ReplySink` (both transports) |
| Failure/cancel copy (`RUN_FAILED_MESSAGE`, auth-cancel copy) | **replaced**: `ReplyOutcome::Failed{summary}`/`Cancelled` rendered by the sink at terminal |
| Working notice + 👀, "still working" nudges, terminal ✅/❌ + retract | `message`: retained, observer-owned, delivery half; `stream`: sink-owned (document phase / heartbeat) |
| Approval/auth gate prompts (+ ⚠️, DM-only auth rule, unserviceable-auth cancel, inbox mirror, gate routes) | `message`: retained, observer-owned via `deliver` on the delivery half with the same policy; `stream`: document `attention` composed by the projection with the same copy and the same host-side policy |
| Wait timeout → failure notice | replaced by durable terminal facts (run lease expiry → `Failed`) |
| Background run notices (`run_delivery/notifications.rs`), model delivery | retained (delivery axis) |
| `record_stream_reply` / `StreamDelivered*` / `bind_projection_stream` | deleted *(done)* |

Cutover status: done. `DeliveryIntent::FinalReply` no longer exists; the
observer registers the originating conversation as a publication target
(`ReplyTargetRegistration`, audience from the trigger class) before it
watches the run, and a channel that cannot publish a reply gets the neutral
failure notice rather than a fallback send. On a `stream` channel the
observer skips the working indicator, nudges, and gate-prompt sends
(`progressive`), keeping the source reactions, inbox mirrors, and the
unserviceable-auth cancel. Every remaining coordinator send — prompts,
notices, notifications, model deliveries — rides the channel's delivery half
regardless of route; the coordinator is transport-blind.

## 8. WebUI *(implemented)*

- `ProjectionReplySink` publishes live facets (answer, reasoning, activity,
  status) as the existing live items with stable ids over
  `LiveProjectionPublisher`; attention, terminal status, cancellation/failure,
  finalized text, and attachments reach the browser through the existing
  durable projection (`turn_events.rs`, runtime projection, timeline) — the
  same facts §3 lists as canonical. Durability claims are limited to those
  facts.
- Delivery evidence for browser replies is the same attempt row + publication
  substate as any channel.
- Binding: the web-app package binds no reply sink itself; the binary marks
  its binding `host_owned_reply` and composition
  (`bind_host_owned_reply_sinks`) attaches the product-tier
  `ProjectionReplySink` before activation checks the halves against the
  manifest — the binary never depends on `ironclaw_assistant`. Composition
  records that channel as the runtime's `session_reply_channel`, and the
  publication service registers it as a target for every run at its first
  revision (private audience), so WebUI live progress is the same
  publication path Slack uses.
- The projection checkpoint fingerprints the answer text (a same-length
  rewrite republishes) and maps `ReplyStatusKind` and activity provenance
  back onto the live item fields the browser already renders.

## 9. Slack native Agent *(implemented in `crates/extensions/packages/slack`; live workspace untouched)*

- App manifest: `features.agent_view` (`agent_description`, `suggested_prompts`),
  Messages tab enabled/writable, bot scopes + `assistant:write` (+ optional
  `chat:write.customize`), bot events + `app_home_opened`,
  `app_context_changed`, `assistant_thread_started`,
  `assistant_thread_context_changed`, `agent_session_stopped`,
  `agent_session_title_changed`.
- Ingress: `agent_session_stopped` → the channel's declared `stop` command;
  the other agent events are authenticated no-ops; every normalized message
  carries a Slack `reply_context`.
- `SlackReplySink`: `agents.sessions.setStatus{processing}` →
  `chat.startStream` → `chat.appendStream` (`markdown_text` deltas by offset,
  `task_update` chunks from activity facts) → attention: markdown chunk +
  `session_status: suspended` → `chat.stopStream` with `session_status`;
  `429`/`ratelimited` + `retry_after` → `Retryable`; transport ambiguity →
  `Ambiguous` then `conversations.replies` read-back; `stopped_by_user` →
  `StoppedByUser`; `message_not_in_streaming_state`/`message_not_owned_by_app`
  → `Permanent`; auth errors → `Unauthorized`.
- **No silent conventional fallback.** A workspace whose app lacks the Agent
  feature (`feature_disabled`/`not_agent_app`) is a clear activation/setup
  failure surfaced to the operator, not a mode switch.
- Native plan mode (`plan_update`, `task_display_mode: plan`) is **not**
  claimed: no plan producer exists in the loop; task UI is driven from real
  activity facts in timeline mode.
- As built: manifest `[channel.reply] transport = "stream"` + the five agent
  endpoints as exact-path bot-token egress; `src/reply_sink/` renders the
  document (status line, markdown text by char offset, `task_update` from
  activity rows, attention as a quoted block + `suspended`, terminal
  `stopStream` + attachments after the stream closes) with a versioned,
  bounded checkpoint; `agent_session_stopped` normalizes to the product
  `stop` command; lockstep tests pin the documented app manifest and setup
  guides to the code. No live Slack app or workspace was modified.

## 10. Migration and compatibility

- Persisted resolved manifests: `[channel.reply]` wire shape unchanged.
- Attempt rows: additive `publication` substate; rows without it behave as
  today.
- Rollback: reverting restores the previous paths; publication substate on
  rows is ignored by older readers (serde default).
- The Slack `agent_view` manifest change is irreversible per Slack; the setup
  doc must say so.

## 11. Verification (2026-08-31, this branch)

| Claim | Evidence | Command |
| --- | --- | --- |
| Reply vocabulary is bounded by construction, reducer deterministic, cadence rule, single sink seam | `crates/contracts/ironclaw_extension_contracts/tests/reply_contract.rs` (21) | `cargo test -p ironclaw_extension_contracts --all-features` |
| Publication substate: open/claim/renew/advance/settle/release guards, lease takeover with fence bump, pre-change rows unchanged, libSQL parity | `crates/domains/ironclaw_outbound/tests/outbound_state_store_contract.rs` | `cargo test -p ironclaw_outbound` |
| Projection composes bounded, redacted documents; terminal from durable facts; phases; disclosure; capacity | `crates/product/ironclaw_assistant/src/reply_projection/tests.rs` (10) | `cargo test -p ironclaw_assistant --lib -- reply_projection` |
| Publication worker: stream vs message cadence, session target, disclosure, retry/ambiguous/permanent/unauthorized/stop, another-node resume with persisted checkpoint and generation change, held lease, heartbeat, retry checkpoint, microburst coalescing, attachments | `crates/product/ironclaw_assistant/src/reply_publication/tests.rs` (17) | `cargo test -p ironclaw_assistant --lib -- reply_publication` |
| Observer cutover: answer via the sink, notices/prompts via the delivery half, nothing-to-report, attachments, resolution-ack dedupe, working indicator retraction after settlement | `crates/product/ironclaw_assistant/tests/run_delivery_contract.rs` (79), `tests/outbound_delivery_contract.rs` (35) | `cargo test -p ironclaw_assistant` |
| WebUI live items from the projection sink; cursor rebasing; text phases under one id; tool failure redaction | `crates/product/ironclaw_assistant/src/projection/tests/{reply_sink,live_progress_stream,runtime_stream}.rs` | `cargo test -p ironclaw_assistant --lib -- projection` |
| Telegram terminal-only sink; Slack Agent sink against a stateful fake Agent API; manifest lockstep | `crates/extensions/packages/telegram/src/tests/reply.rs`, `crates/extensions/packages/slack/tests/{reply_sink_agent_api,agent_app_manifest_lockstep}.rs` | `cargo test -p ironclaw_telegram_extension`, `cargo test -p ironclaw_slack_extension` |
| Activation refuses a declared reply without a real sink; host bridge binds no fake sink | `crates/extensions/ironclaw_extension_host/src/{entrypoint,generic_host}.rs` tests | `cargo test -p ironclaw_extension_host` |
| Channel-host Slack DM journeys ride the Agent wire end-to-end through the production assembly: gate/auth prompts as streamed attention with the message-path copy (approval instruction; auth headline + private-DM setup link; `Shared` strips the link), working state as session status (`processing`/`suspended`), final replies at the single stream close, exactly-once under gate-resolution ack races, and a bare threaded `approve` resolving via the observer's source-conversation route record with no delivered prompt message | `crates/extensions/ironclaw_extension_host/src/channel_host/e2e_tests.rs` (54, incl. `bare_approve_in_dm_resolves_gate_recorded_by_observer`); the scripted coordinator feeds the shared `ReplyProjection` the loop's milestones, standing in for `ReplyProjectionMilestoneSink` | `cargo test -p ironclaw_extension_host` |
| Layer/edge gates, frozen contract names, contracts size ceiling re-pinned (11 451 → 12 928) | `crates/app/ironclaw_architecture_tests` | `cargo test -p ironclaw_architecture_tests` |
| Whole-path integration: a signed Slack channel event becomes a run whose reply is STREAMED through the native Agent surface (startStream with recipient/thread from the stored reply context → text → one stopStream, bot token injected host-side, never a plain post); a Slack-origin run delivers to Telegram while its own reply streams back | `tests/integration/extension_delivery.rs::slack_final_reply_flows_through_the_real_delivery_coordinator`, `tests/integration/delivery_user_journeys.rs::slack_origin_delivers_to_telegram_and_acks_in_slack` | `cargo test -p ironclaw_integration_tests --test reborn_integration_extension_delivery --test reborn_integration_delivery_user_journeys` |

Known follow-ups (not claimed): gate reply routes (`record_gate_route_if_needed`)
are recorded only when a prompt *message* was delivered, so on a `stream`
channel a bare threaded `approve` relies on the run's actionable gate rather
than a delivered-route record; a deployment composed without a channel egress
transport has no coordinator and therefore no publication (and no WebUI live
progress) — the web-app binding always exists in the shipping binary, so this
is a test-composition shape only.
