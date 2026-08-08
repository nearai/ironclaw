# Slack AI Streaming (issue #4491) — Implementation Plan

**Date:** 2026-08-08 (rev. 4 — implemented; six-lens review findings folded in)
**Status:** Implemented on `codex/slack-ai-streaming` (one PR). Six-lens review (security/bugs/perf/tests/conventions/structure) findings addressed: drop-on-append-failure (never re-send an ambiguously-failed suffix — duplication is unrecoverable, the LCP-tail recovers gaps exactly once), minimum inter-append pacing (no per-item retry storm, rate-limit friendly), append acceptance is authoritative (`append_to_stream` returns vendor acceptance — a false positive would compute an empty tail and lose the answer), empty-tail stop-failure retries the text-less stop then converges on the full re-drive (the short text is never appended to an answer-bearing stream), `no_text` → Retryable, the splitter uses a running counter, `stream_final_parts` returns the tail so the recovery shares one rule, the dead `FakeProjectionStream` subscription surface was removed, and the production `.expect()` was removed. Accepted-and-documented: recipient attribution race (D4), crash/restart orphans (§11.3), regeneration stale prefix, >12k-tail stop-failure duplication window. Size ceilings verified PASSING (`cargo test -p ironclaw_architecture_tests`, 95 tests).
**Related:** Issue #4491 (Slack AI streaming follow-up to PR #4490); branch `nosy-plain` @ HEAD `a8ea9f3a1`.
**Goal:** Replace the temporary "Ironclaw is thinking..." Slack status message with Slack-native AI response streaming where the **answer text streams token-by-token** (`chat.startStream` → `chat.appendStream` per live text delta → `chat.stopStream` finalizing the stream message as the answer), for Slack-originated runs. Non-streaming channels keep today's behavior unchanged.
**Architecture:** The channel-neutral delivery pipeline (`RunDeliveryObserver` → `DeliveryCoordinator` → `ChannelAdapter`) gains three new `OutboundPart` variants and a manifest-declared streaming capability. The observer subscribes to the **same live projection feed WebUI uses** (`ProjectionStream` → `ThreadLiveProjectionItem::Text`), converts cumulative text into deltas, and appends them to the Slack stream via a coalescing forwarder. At completion the final answer is delivered through the existing **policy path** as `StreamStop` with the LCP-tail (the part of the answer not already streamed) — so the finalized stream message contains the full answer exactly once, even when deltas were missed or the subscription started late.

> **What this phase explicitly does NOT do:** tool-call summaries as Slack `task_update` cards (deferred, §11.2). Everything else in the issue's acceptance criteria is delivered.

---

## 1. Verified seams (HEAD a8ea9f3a1)

| Seam | Location |
|---|---|
| Working indicator posted in `wait_for_actionable` | `crates/product/ironclaw_assistant/src/run_delivery/observer.rs:572-611` (`DeliveryIntent::Working`, `prompts::WORKING_MESSAGE` = "Ironclaw is thinking..." at `run_delivery/prompts.rs:15`) |
| `RunDeliveryServices::post_notice` | `crates/product/ironclaw_assistant/src/run_delivery.rs:385-419` (emits `OutboundPart::Text`, `thread_anchor: None`) |
| `DeliveryCoordinator::deliver_notice` | `crates/product/ironclaw_assistant/src/delivery_coordinator.rs:468-536` (fresh `OutboundDeliveryId` per notice attempt, 494-495) |
| Final-reply policy path | `coordinator.deliver` via `deliver_run_notification` (`observer.rs:783-869`); parts + attachments at 841-843; `record_gate_route_if_needed` at 529-541 |
| Slack `ChannelAdapter::deliver` | `crates/extensions/packages/slack/src/channel.rs:80-186`; `post_slack_chunk` (chat.postMessage) 303-355; `delete_slack_message` (chat.delete) 357-412; thread_ts = `thread_anchor || topic_id` 97-102 |
| `OutboundPart` enum | `crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs:278-304` (`Retract { vendor_message_ref }` at 299-303 is the mirror shape); `PartDeliveryOutcome::Sent { vendor_message_ref: Option<String> }` at 315-317 |
| reply_context resolution | `delivery_coordinator.rs:643-653` (`resolve_channel_context` → `IngressReplyContextSource`), handed to the adapter on `OutboundEnvelope.reply_context` (`channel_adapter.rs:267-270`); stored at inbound keyed by conversation fingerprint (`ironclaw_extension_host/src/ingress/router.rs:572-586`, durable `FilesystemReplyContextStore`) |
| `ResolvedChannelDelivery` | `crates/contracts/ironclaw_product_contracts/src/delivery.rs:35-40` (carries `adapter: Arc<dyn ChannelAdapter>`); built by `SnapshotChannelDeliveryResolver` (`crates/extensions/ironclaw_extension_host/src/channel_delivery.rs:75-138`) |
| Egress enforcement | `PolicyEnforcedChannelEgress` (`crates/extensions/ironclaw_extension_host/src/egress.rs:115-233`) built from the resolved manifest's `[[channel.egress]]` — undeclared path → `RestrictedEgressError` → `Permanent` |
| Slack manifest egress POST paths | `crates/extensions/packages/slack/manifest.toml:225-231` — exactly `chat.postMessage, chat.delete, conversations.open, files.completeUploadExternal` |
| Egress allowlist parity pin | `crates/app/ironclaw_composition/tests/first_party_manifest_v3_parity.rs:484-488` pins the exact Slack POST paths array |
| Observer cleanup sites | `observer.rs:489-497` (blocked branch, retract before gate prompt), `544-556` (terminal branch: working + `messages_to_delete_after_final`); leaks at `515-517` (`Ok(None)` early return) and `469-482` (timeout `?`) — pre-existing orphan bug, fixed by this plan |
| `DeliveryRunLedger` single-flight | `observer.rs:95-153` (in-memory, per-observer) — prevents duplicate live loops per run; pinned by e2e test `gate_prompt_is_posted_exactly_once_when_approval_ack_races_live_delivery_loop` (e2e_tests.rs:2617+) |
| Live projection feed (WebUI's source) | `ProjectionStream` trait (`crates/contracts/ironclaw_product_contracts/src/projection.rs:187`), `subscribe(ProjectionSubscriptionRequest { actor: TurnActor, scope: TurnScope, after_cursor })` (:163-168) → `ProjectionStreamItem::Update(ThreadLiveProjectionUpdate)`; `ThreadLiveProjectionItem::Text { id, run_id, body }` (`crates/events/ironclaw_event_streams/src/types.rs:174-179`) — **body is cumulative, replaceable state**, coalesced at 16 ms by `LiveProjectionPublisher` (`crates/product/ironclaw_assistant/src/projection/live_progress.rs:55-66, 434-439`); production handle `RebornProjectionServices::product_event_stream()` (composition CONTRACT.md:90); test fake `FakeProjectionStream` (`ironclaw_product_contracts/src/test_support/fakes.rs:26`) |
| e2e harness | `crates/extensions/ironclaw_extension_host/src/channel_host/e2e_tests.rs`: `slack_response_for_approved` 3590-3645 (generic fallback returns `{"ok":true}` with **no `ts`** — stream endpoints need ts-bearing arms); helpers `slack_messages()`/`slack_deletes()` 401-407 via `RecordingEgress::bodies_for`; working-indicator test 2526-2557; `TurnMode` enum 2792-2800 (no `Failed` variant); harness activates the **real bundled Slack manifest** (184-191, 502-531) |
| `ExternalConversationRef` accessors | `crates/contracts/ironclaw_extension_contracts/src/external.rs:231-245`: `space_id()`, `conversation_id()`, `topic_id()`, `reply_target_message_id()` |
| Other `ChannelAdapter` impls | telegram `deliver` exhaustive match (`crates/extensions/packages/telegram/src/channel.rs:236-292`); `HostServedChannelBridge` (`ironclaw_extension_host/src/generic_host.rs:694-697`, never matches parts); test fakes in `outbound_delivery_contract.rs:349+`, `run_delivery_contract.rs:329+`, `model_channel_delivery/tests.rs:309`, `channel_dm_provisioning.rs:317`, `ingress_router_contract.rs`. **No GitHub channel adapter exists.** |
| `validate_final_workspace_files` exhaustive filter | `delivery_coordinator.rs:1035-1040` — explicit `Text | AuthPrompt | Retract` arms; new variants break compilation |
| Contracts size ceiling | `reborn_contracts_crates_carry_a_checked_size_ceiling` (`reborn_dependency_boundaries.rs:610-728`): hard `lines > ceiling` with zero headroom; VERIFIED PASSING on this branch for all contract crates (95 arch tests green) — no re-capture needed |

## 2. Design decisions (validated)

- **D1 — Capability is manifest-declared, not a trait method.** The adapter contract doc says "the adapter never reports metadata (the resolved manifest is the authority)" (`channel_adapter.rs:9-10`); house rule: "route on a manifest-declared capability instead of naming the vendor". New `[channel.presentation]` field `streams_working_indicator = false` (serde default), surfaced on `ResolvedChannelDelivery`.
- **D2 — The coordinator owns the Text-vs-StreamStart reduction.** `post_notice` runs before channel resolution, so the choice moves into the coordinator (`deliver_working_notice`), matching the `OutboundEnvelope` doc ("parts already reduced from the semantic intent by the coordinator", `channel_adapter.rs:264-266`).
- **D3 — Token-by-token: the stream message IS the final answer.** `Working` → `StreamStart` (no initial text). A **forwarder task** (spawned when the stream starts) subscribes to the live projection for the run and converts cumulative text into append deltas: `body.starts_with(appended)` → `appendStream(body[len(appended):])`, coalesced to Slack-friendly cadence; on prefix mismatch (regeneration) appends hold until realignment. At completion the final reply rides the **policy path** with `[StreamAppend…, StreamStop { ref, tail }]` where **tail = final_answer[len(common_prefix(appended, final))..]** — the LCP-tail rule: whatever was already streamed is not re-sent, whatever was missed (late subscription, dropped updates, regeneration) arrives in the stop, so the finalized message contains the full answer **exactly once**. Empty tail (everything already streamed) → `StreamStop` with empty text; `no_text` rejection → retry stop with `WORKING_STREAM_STOP_TEXT`. Text > 12,000 chars split as `[StreamAppend…, StreamStop{tail}]` (last chunk on the stop; stop never carries a 0-length text when there is content to carry).
- **D4 — Recipient attribution via reply_context, user only.** `chat.startStream` requires `recipient_user_id` + `recipient_team_id` for channels (not DMs). The adapter stores `{"user": "<id>"}` in `reply_context` at inbound; team id is already available as `conversation.space_id()`. The latest-wins race (two users in one thread) is **attribution-only** — documented, accepted. Absent reply_context in a channel target → adapter returns `Permanent` → coordinator falls back to `Text` (never a hard failure).
- **D5 — thread_ts for streams:** `thread_anchor || topic_id || reply_target_message_id()` (the user's message `ts`, already carried). Plain DMs have `topic_id = None`; the third fallback is what makes the DM case work. Manual verification of stream-in-DM rendering on a real workspace required before merge.
- **D6 — Stop on every exit.** Restructure `deliver_final_reply` into an inner loop + outer cleanup so the stream (and the forwarder task) are stopped on success, failure, timeout, and the `Ok(None)` no-final-text path. Fixes the pre-existing orphaned-"thinking" leak; with streaming a leak would leave a message stuck in Slack's streaming state.
- **D7 — Degraded fallbacks.** (a) `startStream` fails → coordinator re-drives the Working notice as `Text`; the run then uses today's postMessage+delete behavior end to end (no forwarder, no subscription). (b) `stopStream` with non-empty tail fails permanently before any part was sent → re-drive the full final text as `postMessage` (answer never lost; the stuck stream residue is logged — a stuck open stream cannot be deleted). `message_not_in_streaming_state` → success-equivalent (already stopped; if the tail was non-empty the re-drive posts it). (c) Empty-tail stop fails → retry stop with `WORKING_STREAM_STOP_TEXT` (small artifact, never a stuck stream). (d) No-final-text terminal states (Failed/Cancelled/empty) → `StreamStop { WORKING_STREAM_STOP_TEXT }` + best-effort `chat.delete` (D6).
- **D8 — Blocked runs.** The stream stops with `WORKING_STREAM_STOP_TEXT` + best-effort delete (partial streamed text must not linger as an answer-shaped message) before the gate prompt posts; after gate resolution the loop re-enters and starts a **second** stream + fresh forwarder — intentional, pinned by a test.
- **D9 — Error mapping** reuses the existing `slack_error_kind` / `part_outcome_for_kind` seam (`channel.rs:410, 434+`); `message_not_in_streaming_state`/`message_not_owned_by_app` → `Permanent` on start/append, success-equivalent on stop; `no_text` on an empty stop → `Retryable` (the observer's retry-with-short-text resolves it), `Permanent` everywhere else.
- **D10 — Accepted limitations (documented, not fixed):** restart mid-run orphans the stream (ledger and live projection are in-memory); live projection is ephemeral (no replay) so pre-subscription text is missed — the LCP-tail stop recovers it; regeneration mid-stream can leave a stale prefix (appends hold on prefix mismatch; tail carries the corrected remainder); Slack's AI-apps feature must be enabled (a `Permanent` start failure degrades to the text fallback automatically); the finalized stream message's `ts` becomes the delivery identity.

## 3. Contract changes (Task 1)

`crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs` — extend the enum (after `AuthPrompt`):

```rust
    /// Start a vendor-native progressive-response stream (e.g. Slack
    /// `chat.startStream`). `markdown_text` is the optional initial text;
    /// when `None` the vendor renders its streaming state alone. Only
    /// emitted by the coordinator for channels that declare
    /// `streams_working_indicator`; other adapters return a typed failure.
    StreamStart { markdown_text: Option<String> },
    /// Append text to a stream started by `StreamStart` (e.g. Slack
    /// `chat.appendStream`). Used both for live text deltas and for
    /// splitting payloads over the vendor's per-call text cap; the last
    /// chunk rides `StreamStop` instead.
    StreamAppend { vendor_message_ref: String, markdown_text: String },
    /// Finalize a stream started by `StreamStart`; the finalized message
    /// IS the delivery (replaces a post). `vendor_message_ref` is the `ts`
    /// a previous `Sent` outcome returned; `markdown_text` may be empty
    /// only when the streamed content already is the full payload.
    StreamStop { vendor_message_ref: String, markdown_text: String },
```

`crates/contracts/ironclaw_extension_contracts/src/channel.rs` — `ChannelPresentation` (struct ~497-504; verify exact fields at implementation):

```rust
    /// The channel renders the agent's working state as a vendor-native
    /// progressive-response stream (e.g. Slack `chat.startStream`) instead
    /// of a transient text message. Manifest-declared; the host never
    /// guesses from the vendor name.
    #[serde(default)]
    pub streams_working_indicator: bool,
```

`crates/contracts/ironclaw_product_contracts/src/delivery.rs` — `ResolvedChannelDelivery` (line 35):

```rust
pub struct ResolvedChannelDelivery {
    pub extension_id: ExtensionId,
    pub installation_id: AdapterInstallationId,
    pub adapter: Arc<dyn ChannelAdapter>,
    /// Whether the channel's manifest declares working-indicator streaming.
    pub streams_working_indicator: bool,
}
```

Compile-enforced constructor updates: `SnapshotChannelDeliveryResolver` (`channel_delivery.rs:75-138`, read `extension.resolved.channel.presentation.streams_working_indicator` — verify exact manifest type path), plus test resolvers at `outbound_delivery_contract.rs:407`, `run_delivery_contract.rs:331`, `model_channel_delivery/tests.rs:310`, `channel_dm_provisioning.rs:318` (set `false`, except where a test needs streaming).

## 4. Slack manifest + parity pin (Task 2)

`crates/extensions/packages/slack/manifest.toml`:

```toml
[channel.presentation]
supports_markdown = true
supports_threads = true
max_message_chars = 40000
command_prefix = "/ironclaw "
streams_working_indicator = true
```

POST `[[channel.egress]]` paths += `"/api/chat.startStream"`, `"/api/chat.appendStream"`, `"/api/chat.stopStream"`.

`crates/app/ironclaw_composition/tests/first_party_manifest_v3_parity.rs:484-488` — add the three paths to the expected array.

## 5. Slack adapter (Task 3)

`crates/extensions/packages/slack/src/channel.rs`:

New helpers (mirror `post_slack_chunk`'s shape exactly — `RestrictedEgressRequest` POST to `https://{SLACK_API_HOST}/api/chat.*Stream`, `credential: SecretHandle::new(SLACK_BOT_TOKEN_HANDLE)`, JSON body, `application/json; charset=utf-8`, parse `{ok, error, ts}`):

```rust
async fn start_slack_stream(
    egress: &dyn RestrictedEgress,
    credential: &SecretHandle,
    channel: &str,
    thread_ts: &str,
    recipient_user_id: Option<&str>,
    recipient_team_id: Option<&str>,
) -> PartDeliveryOutcome
// body: {"channel", "thread_ts"} + recipient_user_id/recipient_team_id when Some
// ok:true -> Sent { vendor_message_ref: ts }; else part_outcome_for_kind(slack_error_kind(e), ...)

async fn append_slack_stream(
    egress: &dyn RestrictedEgress,
    credential: &SecretHandle,
    channel: &str,
    ts: &str,
    markdown_text: &str,
) -> PartDeliveryOutcome
// body: {"channel", "ts", "markdown_text"}; ok:true -> Sent { vendor_message_ref: ts }

async fn stop_slack_stream(
    egress: &dyn RestrictedEgress,
    credential: &SecretHandle,
    channel: &str,
    ts: &str,
    markdown_text: &str,   // may be "" when the full answer was already streamed
) -> PartDeliveryOutcome
// body: {"channel", "ts", "markdown_text"} (omitted when empty)
// "message_not_in_streaming_state" -> Sent { vendor_message_ref: Some(ts) } (already stopped)
// "no_text" -> Retryable (observer retries stop with WORKING_STREAM_STOP_TEXT)
// other errors via part_outcome_for_kind
```

New private inbound helper (payload.rs, both constructor sites at lines 224 and 318):

```rust
/// Opaque per-message context the host stores and hands back at delivery:
/// the originating user id, needed by chat.startStream for channel streams.
fn slack_reply_context(user: &str) -> Option<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({ "user": user })).ok()
}
```

Set `reply_context: slack_reply_context(user)` at both `NormalizedInboundMessage` construction sites in `payload.rs` (message path has `user`; slash-command path has `form.user_id`).

`deliver` arms (inside the existing part loop; `thread_ts` resolution and recipient rules shared by all three stream arms):

```rust
// shared: thread_ts = anchor || topic_id || reply_target_message_id (required by Slack)
// shared: is_dm = channel.starts_with('D'); recipients only for non-DM;
//         non-DM without reply_context user -> Permanent (coordinator falls back to Text)
OutboundPart::StreamStart { .. } => { /* start_slack_stream; standard sent/!sent handling */ }
OutboundPart::StreamAppend { vendor_message_ref, markdown_text } => {
    /* append_slack_stream(egress, &credential, &channel, vendor_message_ref, markdown_text) */
}
OutboundPart::StreamStop { vendor_message_ref, markdown_text } => {
    /* stop_slack_stream(...) — vendor_message_ref is the ts from the startStream Sent outcome */
}
```

Splitter: 12,000-char cap for stream calls (`const SLACK_STREAM_TEXT_LIMIT_CHARS: usize = 12_000`, distinct from the postMessage `slack_text_chunks` 35k/34.9k splitter at `mrkdwn.rs:6-7`) — chunked text rides `StreamAppend` except the last chunk, which rides `StreamStop`.

## 6. Coordinator + observer (Tasks 4-5)

### 6.1 Coordinator — `crates/product/ironclaw_assistant/src/delivery_coordinator.rs`

New outcome + method (notice path; refactor the resolution/drive internals of `deliver_notice` into a shared helper):

```rust
/// Outcome of a working-notice delivery: which vendor ref was accepted and
/// whether it is a progressive stream (stop via StreamStop) or a plain
/// message (stop via Retract).
pub struct WorkingNoticeOutcome {
    pub conversation: ExternalConversationRef,
    pub vendor_message_ref: Option<String>,
    pub streamed: bool,
}

impl DeliveryCoordinator {
    /// Reduces the Working intent for the resolved channel: `StreamStart`
    /// when the channel declares `streams_working_indicator`, else `Text`.
    /// On a Permanent/Unauthorized StreamStart failure, re-drives the notice
    /// as `Text` (the degraded path) so the working indicator is never lost.
    /// `request.parts` must be empty; the parts are built here.
    pub async fn deliver_working_notice(
        &self,
        request: NoticeDeliveryRequest<'_>,
        text: &str,
    ) -> Result<WorkingNoticeOutcome, CoordinatedDeliveryError>;
}
```

Flow: resolve channel context → `streams_working_indicator` → parts `[StreamStart { markdown_text: None }]`, drive; on failure, parts `[Text(text)]`, drive again (`streamed: false`); success → first `Sent` ref + `streamed` from which drive succeeded. The fallback mints a second `OutboundDeliveryAttempt` (fine — each attempt is separately persisted; OUT-3).

Stream appends (forwarder) and the final stop go through the same notice machinery with `DeliveryIntent::Working` / `Cleanup` and explicit parts — no coordinator behavior change beyond `deliver_working_notice`.

### 6.2 Services — `crates/product/ironclaw_assistant/src/run_delivery.rs`

```rust
/// A posted working indicator: either a vendor-native stream (finalized via
/// StreamStop) or a plain transient message (stopped via Retract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedWorkingNotice {
    pub conversation: ExternalConversationRef,
    pub vendor_message_ref: String,
    pub streamed: bool,
}

/// One live text delta awaiting a Slack append: the suffix of the run's
/// cumulative text not yet sent, coalesced by the forwarder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStreamAppend {
    pub vendor_message_ref: String,
    pub suffix: String,
}
```

- `RunDeliveryServices::start_working_notice(scope, run_id, conversation) -> Option<PostedWorkingNotice>` — calls `coordinator.deliver_working_notice(..., prompts::WORKING_MESSAGE)`; maps outcome (no ref → `None`), best-effort (log on error, return `None`).
- `RunDeliveryServices::append_to_stream(scope, run_id, append: PendingStreamAppend)` — `deliver_notice` with `DeliveryIntent::Working`, parts `[StreamAppend { vendor_message_ref, markdown_text: suffix }]`, best-effort; failures return `Err` to the forwarder so the suffix is **retained** (a failed append must not be dropped — the LCP-tail stop recovers it, but holding it avoids a gap in the middle of the stream).
- `RunDeliveryServices::stop_working_notice(scope, run_id, notice, markdown_text)` — final stop: `streamed` → `StreamStop { vendor_message_ref, markdown_text }` (+ trailing `Retract` only on the no-final-text/blocked paths, D7d/D8); plain → `Retract` alone. Via `DeliveryIntent::Cleanup`, notice_ref `retract-{ref}`, best-effort like today's `retract_message`.
- New prompt constant in `run_delivery/prompts.rs` (next to `WORKING_MESSAGE` at :15): `pub const WORKING_STREAM_STOP_TEXT: &str = "Ironclaw finished.";` — used for empty-tail stop retries, no-final-text stops, and blocked stops (D7c/D7d/D8).
- `RunDeliveryServices` gains `projection_stream: Arc<dyn ProjectionStream>` (production wiring: `RebornProjectionServices::product_event_stream()`, composition CONTRACT.md:90; e2e: `FakeProjectionStream`).
- `post_notice` keeps its current shape for all other intents (auth prompts, failure notices, connect nudges).

### 6.3 Observer — `crates/product/ironclaw_assistant/src/run_delivery/observer.rs`

1. `deliver_final_reply` restructure — outer owns the notice + forwarder handle, stops both on **every** exit:

```rust
async fn deliver_final_reply(&self, envelope, ack) -> Result<(), RunDeliveryError> {
    // ... existing claim/permit logic unchanged ...
    let mut working_notice: Option<PostedWorkingNotice> = None;
    let mut forwarder: Option<StreamForwarderHandle> = None;
    let result = self
        .deliver_final_reply_inner(envelope, ack, &mut working_notice, &mut forwarder)
        .await;
    if let Some(forwarder) = forwarder.take() { forwarder.shutdown().await; }
    if let Some(notice) = working_notice.take() {
        self.services.stop_working_notice(scope, run_id, notice,
            prompts::WORKING_STREAM_STOP_TEXT).await;
    }
    result
}
```

2. `deliver_final_reply_inner` = the existing loop with:
   - `working_message: Option<DeliveredChannelMessage>` → `working_notice: &mut Option<PostedWorkingNotice>` + `forwarder: &mut Option<StreamForwarderHandle>`, raised via `start_working_notice` in `wait_for_actionable` (602-609); **when the notice is `streamed`, spawn the forwarder** before returning from the poll:
   - `StreamForwarderHandle` — a spawned task owning `ProjectionStreamSubscription` (bounded buffer, e.g. 16) + a `mpsc::UnboundedSender<PendingStreamAppend>` back into the delivery loop (the loop is async and single-threaded per run; appends must not race the stop). Forwarder behavior per `ProjectionStreamItem::Update(ThreadLiveProjectionUpdate)`:
     - filter `ThreadLiveProjectionItem::Text { run_id, body }` for our run_id;
     - maintain `appended: String`; `if body.starts_with(&appended) { suffix = body[appended.len()..]; appended = body; }` else hold (regeneration guard, D10);
     - coalesce: flush when the pending suffix ≥ `SLACK_APPEND_CHUNK_CHARS` (≈ 250) or idle ≥ `SLACK_APPEND_IDLE_MS` (≈ 500) — Slack rate-limit friendliness (16 ms WebUI cadence is not Slack-appropriate);
     - each flush sends `PendingStreamAppend` on the channel; the delivery loop drains it between polls and calls `append_to_stream` (retaining on failure).
   - **terminal-delivered branch (544-556)** — final reply via the policy path:

     ```rust
     // final answer IS the stream; compute the LCP-tail so streamed content is
     // never duplicated and missed content is never lost
     let parts = if let Some(notice) = working_notice.take() {
         if notice.streamed {
             forwarder.take().map(|f| f.shutdown().await);   // stop accepting deltas
             let appended = forwarder_appended_text();       // shared Arc<Mutex<String>>
             let tail = final_text[common_prefix_len(&appended, &final_text)..];
             stream_final_parts(&notice.vendor_message_ref, tail)  // split at 12k;
                 // empty tail -> StreamStop { ref, markdown_text: "" }
         } else { vec![OutboundPart::Text(final_text.clone())] }
     } else { vec![OutboundPart::Text(final_text.clone())] };
     // coordinator.deliver (policy path); on failure with NO part sent and a
     // non-empty tail, re-drive once with vec![OutboundPart::Text(final_text)] (D7b)
     ```

     `common_prefix_len(a, b)` is the byte length of the longest common prefix (both are the same sanitized pipeline output at the tail; mismatch only via regeneration, which the hold-guard already limits). `record_gate_route_if_needed` (529-541) consumes the `stopStream` response `ts` as the delivery identity (D10).
   - blocked branch (489-497): `take()` notice, shutdown forwarder, `stop_working_notice(..., WORKING_STREAM_STOP_TEXT)` (partial text must not linger as an answer-shaped message) — before the gate prompt, preserving today's ordering;
   - `messages_to_delete_after_final` via `retract_message` unchanged (plain `chat.delete`);
   - `wait_for_actionable` timeout `?` and `notification_for_actionable_state` errors propagate to the outer cleanup.

3. `wait_for_actionable` signature: `working_notice: &mut Option<PostedWorkingNotice>`, `forwarder: &mut Option<StreamForwarderHandle>` (575).

## 7. Other adapters (Task 6)

- `crates/extensions/packages/telegram/src/channel.rs` — add to the `deliver` match (unreachable in practice; defense-in-depth):

```rust
OutboundPart::StreamStart { .. } | OutboundPart::StreamAppend { .. } | OutboundPart::StreamStop { .. } => {
    parts.push(PartDeliveryOutcome::Permanent {
        reason: "telegram channel does not support working-indicator streaming".to_string(),
    });
    break 'parts;
}
```

- `delivery_coordinator.rs:1035-1040` `validate_final_workspace_files` — add `StreamStart { .. } | StreamAppend { .. } | StreamStop { .. }` to the pass-through arm (they carry no workspace refs).
- Test fakes that never match parts need no changes (`HostServedChannelBridge`, `FakeChannelAdapter`).

## 8. Tests (Task 7)

### 8.1 e2e — `crates/extensions/ironclaw_extension_host/src/channel_host/e2e_tests.rs`

1. **Mock**: `slack_response_for_approved` (3590-3645) gains arms for `chat.startStream` / `chat.appendStream` / `chat.stopStream` returning ts-bearing bodies. Distinct ts per call: add an `AtomicU64` counter to the mock egress producing `format!("1700000000.000{n}")` (the current generic fallthrough `{"ok":true}` without `ts` would make `Sent { vendor_message_ref: None }` and silently no-op — the arms are required). Helpers: `slack_stream_starts()` / `slack_stream_appends()` / `slack_stream_stops()` = `bodies_for(...)`.
2. **Harness**: inject `FakeProjectionStream` (from `ironclaw_product_contracts::test_support::fakes`) into the observer services; add `harness.push_live_text(run_id, body)` feeding `ThreadLiveProjectionUpdate` items so tests control token flow deterministically.
3. **Rewrite** `slack_dm_posts_working_indicator_and_deletes_it_after_final_reply` (2526) → `slack_dm_streams_answer_text_and_finalizes_with_the_tail`: DM message → `startStream` (channel + `thread_ts` = user message `ts`, no recipient, no initial text); push live text "Hello " then "Hello world" → exactly one `appendStream` with body text `"world"` (suffix, not cumulative); `complete_active_run("Hello world")` → `stopStream` with the start `ts` and **empty** `markdown_text` (tail empty — everything streamed); no `chat.postMessage` for the answer, no `chat.delete`; order `startStream < appendStream < stopStream`.
4. **New — tail recovery**: push live text "Hello", complete with "Hello world" → `stopStream` text == `" world"` (missed content arrives in the stop).
5. **New — >12k answer**: final text over 12,000 chars with no live appends → `[appendStream(chunk1)…, stopStream(tail)]`, all with the start `ts`, no part > 12k, tail non-empty.
6. **New — failure path**: add `TurnMode::Failed` + `coordinator.fail_active_run()` (mirrors the #4490 review suggestion); assert the stream is stopped with `WORKING_STREAM_STOP_TEXT` + `chat.delete` of the stream `ts` (no orphaned open stream, no partial answer lingering).
7. **New — blocked→resume**: stream stopped on `BlockedApproval` (before the prompt), a **second** `startStream` after resume, live text streamed again, finalized with the tail — pins D8.
8. **New — stopStream failure fallback**: harness option making `chat.stopStream` return `{"ok": false, "error": "not_allowed_token_type"}` with non-empty tail → final answer still delivered as `chat.postMessage` (full text), exactly once.
9. **New — empty-stop `no_text` retry**: harness option making `chat.stopStream` fail once with `no_text` on an empty tail → retried with `WORKING_STREAM_STOP_TEXT`, then success.
10. **New — degraded start fallback**: harness option making `chat.startStream` return `{"ok": false, ...}` → working indicator as plain `chat.postMessage`, final answer via `postMessage`, cleanup via `chat.delete`, no projection subscription (today's behavior end to end).

### 8.2 Unit / contract

- `crates/extensions/packages/slack/src/channel.rs` mod tests: `startStream` request shape (channel+thread_ts, recipient only for non-DM conversation ids, absent reply_context in channel → `Permanent`); `appendStream`/`stopStream` bodies (ts + text, empty text omitted); 12k splitter boundaries (12,000 → one stop; 12,001 → append+stop); `message_not_in_streaming_state` on stop → `Sent` with the ts; `no_text` on stop → `Retryable`; ts from response.
- `crates/product/ironclaw_assistant/tests/run_delivery_contract.rs` (path corrected — there is no `src/run_delivery/tests/`): streaming resolver → `StreamStart` part recorded for `Working`; `StreamStart` `Permanent` → `Text` fallback, `streamed: false`; forwarder converts cumulative → suffix (`"Hello "` → `"world"`), holds on prefix mismatch, coalesces by threshold/idle (zero-delay clock in tests); final reply parts = `StreamStop` with LCP-tail (empty-tail and non-empty-tail cases); stopStream `Permanent` with no part sent → re-drive `Text`; blocked stop dispatches `StreamStop` vs `Retract` by kind; failed append is retained and re-flushed; `ResolvedChannelDelivery` constructor updated at 331.
- `crates/product/ironclaw_assistant/tests/outbound_delivery_contract.rs` (constructor at 407) and the other fakes listed in §3 — compile updates only.
- `first_party_manifest_v3_parity.rs` — egress array + presentation flag.

## 9. Verification checklist (Task 8, final)

```bash
cargo fmt --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test -p ironclaw_extension_contracts
cargo test -p ironclaw_assistant          # run_delivery_contract, outbound_delivery_contract, coordinator tests
cargo test -p ironclaw_extension_host     # channel_host e2e (adjust if the e2e module is feature-gated)
cargo test -p ironclaw_composition        # first_party_manifest_v3_parity
cargo test -p ironclaw_architecture_tests # dependency boundaries + contract size ceiling (re-capture in same PR if exceeded)
python3.11 scripts/check_no_panics.py --base origin/main --head HEAD   # repo convention from PR #4490
git diff --check
```

Plus one manual verification on a real Slack workspace: stream-in-DM rendering (thread-anchored stream on the user's message), that the workspace has Slack's AI-apps feature enabled, live text cadence against Slack rate limits (adjust `SLACK_APPEND_CHUNK_CHARS` / `SLACK_APPEND_IDLE_MS`), and an empty-tail stop (finalize with already-streamed content).

PR body per repo rules: describe every layer (contract → manifest → adapter → coordinator → observer → forwarder → tests), note compatibility (other adapters untouched), rollback (manifest flag off restores prior behavior), and the Test Strategy section per tier with evidence.

## 10. Commit sequence

1. `feat(contracts): add StreamStart/StreamAppend/StreamStop parts and streams_working_indicator capability` (Task 1)
2. `feat(slack): declare streaming capability and egress paths in manifest` + parity test (Task 2)
3. `feat(slack): map stream parts to chat.startStream/appendStream/stopStream over host egress` (Task 3)
4. `feat(delivery): reduce Working to a stream with text fallback in the coordinator` (Task 4)
5. `feat(delivery): stream live text deltas via projection subscription with coalescing forwarder` (Task 5a)
6. `feat(delivery): finalize streamed runs with the LCP-tail via stopStream, re-driving postMessage on failure` (Task 5b)
7. `chore(delivery): explicit non-stream arms for telegram and file validation` (Task 6)
8. `test(slack): e2e coverage for token streaming, tail recovery, failure, and fallback` (Task 7)
9. `chore: gates, docs, PR` (Task 8)

## 11. Deferred

### 11.1 Tool-call summaries
Slack `task_update` / `plan_update` chunks (`chat.*Stream` `chunks` param) rendered from `ThreadLiveProjectionItem::CapabilityActivity` (types.rs:185-189) — native task-timeline UI. Requires the forwarder to emit structured chunks (a new part variant or a chunk payload on `StreamAppend`), and redaction review of capability summaries for channel exposure. Separately scoped.

### 11.2 Post-stream attachments
Attachments already ride the final policy-path envelope after the stop (D3). Ordering/files-UI polish (files attached to the streamed message vs the thread) to be confirmed against vendor behavior on a real workspace.

### 11.3 Restart durability
Persisting the stream ref (`vendor_message_ref`) so a restart can stop an orphaned open stream at boot (requires a small store; the in-memory ledger is per-observer). Accepted for now (D10).

Related reading: `docs/internal/design/agent-activity-streaming.md`.

## 12. Risks & limitations

| Risk | Mitigation |
|---|---|
| `stopStream` with empty text rejected (`no_text`) | Adapter maps `no_text` → `Retryable`; observer retries with `WORKING_STREAM_STOP_TEXT` (D7c); e2e 8.1.9 |
| Appended live text ≠ finalized text (regeneration, sanitization) | Appends hold on prefix mismatch; LCP-tail stop delivers the remainder — answer always complete, never duplicated (D3) |
| Missed deltas (late subscription, dropped updates, slow consumer) | Live feed is ephemeral by design; LCP-tail stop recovers everything (D3, D10) |
| `stopStream` fails → answer lost | Re-drive full text as `postMessage` when no part was sent and tail non-empty (D7b); e2e 8.1.8 |
| Slack rate limits on `chat.appendStream` | Coalescing (250 chars / 500 ms constants, D3); tune against real workspace in §9 |
| Stuck open stream on crash/restart (in-memory ledger) | Accepted (D10); stop-on-every-exit covers in-process paths; §11.3 tracked |
| Recipient attribution race (latest-wins reply_context) | Attribution-only impact; document; absent context degrades to text (D4/D7) |
| Slack workspace without AI-apps feature | `startStream` `Permanent` → text fallback automatically (D7) |
| DM stream vendor rendering unverified | Manual workspace check before merge (§9) |
| Delivery identity becomes the stream `ts` | `record_gate_route_if_needed` and `DeliveredChannelMessage` consumers use the `stopStream` response `ts`; documented (D10) |
| `ironclaw_extension_contracts` size ceiling | ~440 lines headroom; re-capture in same PR if exceeded |
| Two observers on gate-resume (pre-existing dedup gap) | Out of scope (ledger covers live path; `triggered.rs` never posts `Working`); noted, not changed |
