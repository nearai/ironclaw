//! The Slack reply half: Slack's native Agent surface as a `ReplySink`
//! (`docs/internal/design/2026-08-31-progressive-reply-publication.md` §9).
//!
//! One run is one agent session (`agents.sessions.setStatus`) plus one
//! streaming message (`chat.startStream` → `chat.appendStream` × n →
//! `chat.stopStream`) in the conversation's thread. The host hands this sink
//! a desired-state `ReplyDocument` at each cadence point together with the
//! sink's own previous checkpoint; the sink computes what Slack has not seen
//! yet — the answer text past the appended char offset, task cards whose
//! fingerprint moved, a status line while the answer is still empty, an
//! attention block — and sends it in ONE `chat.appendStream`, or nothing at
//! all when the checkpoint already reflects the document. Every provider
//! verb, id, and offset lives behind the checkpoint; the host persists it
//! and never reads it.
//!
//! What this sink deliberately does NOT do:
//! - fall back to a conventional message when the workspace's app is not an
//!   Agent (`feature_disabled` / `not_agent_app`) — that is a setup failure
//!   the operator must see, so it is a `Permanent` outcome naming the missing
//!   capability;
//! - claim `Applied` for a request whose transport failed mid-flight — that
//!   is `Ambiguous`, and the next reconcile reads the streaming message back
//!   (`conversations.replies`) before appending more;
//! - invent a second planning model — Slack's `plan` display uses the run
//!   lifecycle as its header and groups only real activity facts beneath it.
//!
//! Slack facts the mechanics rest on (docs.slack.dev, verified 2026-08-31):
//! `chat.appendStream` `markdown_text` "is what will be appended to the
//! message received so far" (deltas, not cumulative); `recipient_user_id` /
//! `recipient_team_id` are "Required when streaming to channels"; sessions
//! "in `processing` time out after one hour and automatically transition to
//! `active`"; `chat.stopStream` "can set the session status via the
//! `session_status` parameter (defaults to `active`)"; after the stop button
//! "The session status does not update automatically ... Your app is
//! responsible for transitioning the status".

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_extension_contracts::channel_adapter::{ChannelError, PartDeliveryOutcome};
use ironclaw_extension_contracts::reply::{
    ReplyDocument, ReplyOutcome, ReplyOutcomeReason, ReplyProviderRef, ReplyReconcileRequest,
    ReplySink, ReplySinkEvidence, ReplySinkOutcome, ReplySinkReport,
};
use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;
use ironclaw_host_api::attachment::WorkspaceFile;
use ironclaw_host_api::ids::SecretHandle;
use serde_json::{Value, json};

use crate::api::SlackWebApiMethod;
use crate::channel::{SLACK_BOT_TOKEN_HANDLE, SlackChannelAdapter};
use crate::reply_context::SlackReplyContext;

mod agent_api;
mod checkpoint;
mod plan;

use agent_api::{SlackAgentApi, SlackApiFailure, outcome_for_failure, outcome_for_part};
use checkpoint::{
    SlackAppliedState, SlackReplyCheckpoint, SlackSessionStatus, SlackStreamState,
    SlackTerminalState, char_prefix, delta_landed_after, encode_checkpoint, load_checkpoint,
    normalize_for_match, normalized_tail,
};
use plan::{AnswerRewritten, ChunkPlan, plan_chunks, terminal_note};

/// The checkpoint payload version this sink writes and understands. A
/// checkpoint of any other version is treated as absent (fresh presentation)
/// rather than misapplied.
pub const SLACK_REPLY_CHECKPOINT_VERSION: u32 = 1;

/// `chat.appendStream` / `chat.startStream`: "Limit this field to 12,000
/// characters" — one markdown chunk never exceeds it; a longer delta is
/// split into consecutive chunks of the same request.
const SLACK_MARKDOWN_CHUNK_MAX_CHARS: usize = 12_000;
/// Progress text is published by whole paragraph (Slack renders every
/// markdown chunk as its own block). A paragraph this long with no blank
/// line yet is flushed at its last sentence boundary instead of waiting.
pub(crate) const SLACK_TEXT_HOLD_MAX_CHARS: usize = 600;

/// Slack documents a 256-character limit for `task_update` chunks. Keep each
/// user-visible task field within that boundary before it reaches the API.
const SLACK_TASK_FIELD_MAX_CHARS: usize = 256;

/// Sessions "in `processing` time out after one hour". A liveness point
/// re-asserts `processing` once the last assertion is older than this.
const SESSION_STATUS_REASSERT_AFTER: Duration = Duration::from_secs(30 * 60);

/// How much of the answer prefix the read-back compares. Long enough that a
/// coincidental match is negligible, short enough to ignore Slack's own
/// markdown → mrkdwn rendering elsewhere in the message.
const READ_BACK_TAIL_CHARS: usize = 200;

/// `task_display_mode` — group tool activity into Slack's
/// collapsible Agent plan instead of interleaving loose task cards with text.
const TASK_DISPLAY_MODE_PLAN: &str = "plan";

// ── The sink ─────────────────────────────────────────────────────────────

#[async_trait]
impl ReplySink for SlackChannelAdapter {
    async fn reconcile(
        &self,
        request: ReplyReconcileRequest,
        egress: &dyn RestrictedEgress,
    ) -> Result<ReplySinkReport, ChannelError> {
        let credential =
            SecretHandle::new(SLACK_BOT_TOKEN_HANDLE).map_err(|error| ChannelError::Render {
                reason: format!("invalid bot token handle: {error}"),
            })?;
        let api = SlackAgentApi {
            egress,
            credential: &credential,
        };
        let mut reconciler = Reconciler {
            api,
            request: &request,
            checkpoint: load_checkpoint(&request),
            evidence: ReplySinkEvidence::default(),
        };
        let outcome = reconciler.run().await;
        Ok(reconciler.report(outcome))
    }
}

struct Reconciler<'a> {
    api: SlackAgentApi<'a>,
    request: &'a ReplyReconcileRequest,
    checkpoint: SlackReplyCheckpoint,
    evidence: ReplySinkEvidence,
}

/// Where the reply goes: resolved once per reconcile from the stored reply
/// context (preferred) or the target conversation.
struct SlackReplyRoute {
    channel: String,
    thread_ts: Option<String>,
    recipient_user_id: Option<String>,
    recipient_team_id: Option<String>,
}

impl Reconciler<'_> {
    fn report(&mut self, outcome: ReplySinkOutcome) -> ReplySinkReport {
        let checkpoint =
            match encode_checkpoint(&mut self.checkpoint, &self.request.revision.document) {
                Ok(checkpoint) => Some(checkpoint),
                Err(reason) => {
                    return ReplySinkReport {
                        outcome: ReplySinkOutcome::Permanent { reason },
                        checkpoint: None,
                        evidence: std::mem::take(&mut self.evidence),
                    };
                }
            };
        ReplySinkReport {
            outcome,
            checkpoint,
            evidence: std::mem::take(&mut self.evidence),
        }
    }

    fn record_ref(&mut self, reference: &str) {
        // One report cites each provider ref once — a terminal that opens
        // and closes the same stream in a single reconcile is one message.
        if self
            .evidence
            .provider_refs
            .iter()
            .any(|existing| existing.as_str() == reference)
        {
            return;
        }
        match ReplyProviderRef::new(reference) {
            Ok(reference) => {
                if let Err(error) = self.evidence.provider_refs.push(reference) {
                    tracing::debug!(error = %error, "slack reply sink dropped a provider ref");
                }
            }
            Err(error) => {
                tracing::debug!(error = %error, "slack reply sink saw an unusable provider ref");
            }
        }
    }

    async fn run(&mut self) -> ReplySinkOutcome {
        let route = match resolve_route(self.request) {
            Ok(route) => route,
            Err(reason) => return ReplySinkOutcome::Permanent { reason },
        };
        if self.checkpoint.terminal == Some(SlackTerminalState::Applied) {
            // A repeated terminal reconcile: Slack already reflects it.
            return ReplySinkOutcome::Applied;
        }
        if let Err(outcome) = self.resolve_pending(&route).await {
            return outcome;
        }
        if self.request.revision.document.is_terminal() {
            return self.finish(&route).await;
        }
        self.progress(&route).await
    }

    /// An earlier request crossed into transport unanswered. Read the
    /// streaming message back and decide whether it landed before anything
    /// else is appended.
    async fn resolve_pending(&mut self, route: &SlackReplyRoute) -> Result<(), ReplySinkOutcome> {
        let Some(stream) = self.checkpoint.stream.clone() else {
            return Ok(());
        };
        let Some(pending) = stream.pending.clone() else {
            return Ok(());
        };
        let document = &self.request.revision.document;
        let text = document.answer.text.as_str();
        let carried_text = pending.to_chars > stream.appended_chars;
        let message = match self.api.read_back(&route.channel, &stream.ts).await {
            Ok(message) => message,
            Err(failure) => {
                let outcome = outcome_for_failure(SlackWebApiMethod::ConversationsReplies, failure);
                if matches!(
                    outcome,
                    ReplySinkOutcome::Retryable { .. } | ReplySinkOutcome::Unauthorized { .. }
                ) {
                    // A rate limit is retried; a token that cannot read is
                    // the contract's `Unauthorized`, never an ambiguity to
                    // retry until the terminal budget lapses.
                    return Err(outcome);
                }
                if carried_text {
                    // Read-back cannot determine whether the text landed
                    // (missing scope, a deleted parent, …): never repeat a
                    // fragment the user may already see. The pending stays on
                    // the checkpoint and the outcome stays `Ambiguous`; the
                    // host settles `Unknown` if this never resolves.
                    tracing::debug!(
                        outcome = outcome.kind_name(),
                        "slack reply read-back unavailable for a text-carrying append; staying ambiguous"
                    );
                    return Err(ReplySinkOutcome::Ambiguous {
                        reason: ReplyOutcomeReason::new(
                            "slack read-back is unavailable; a pending text append cannot be verified",
                        ),
                    });
                }
                // A pending that carried only task and status chunks is safe
                // to re-send: a repeated task card upserts by id.
                tracing::debug!(
                    outcome = outcome.kind_name(),
                    "slack reply read-back unavailable; re-sending the idempotent non-text chunks"
                );
                self.clear_pending();
                return Ok(());
            }
        };
        let message_text = match message {
            agent_api::SlackReadBack::Found(text) => text,
            agent_api::SlackReadBack::NotFound => {
                tracing::debug!("slack streaming message not found on read-back; re-sending");
                self.clear_pending();
                return Ok(());
            }
            agent_api::SlackReadBack::FoundWithoutText => {
                // The message exists but there is nothing to compare (a
                // blocks-only rendering omits `text`): a text-carrying
                // pending stays unverifiable — never re-send a fragment the
                // user may already see.
                if carried_text {
                    return Err(ReplySinkOutcome::Ambiguous {
                        reason: ReplyOutcomeReason::new(
                            "slack read-back found the message but no comparable text; a pending text append cannot be verified",
                        ),
                    });
                }
                tracing::debug!(
                    "slack read-back found no comparable text; re-sending the idempotent non-text chunks"
                );
                self.clear_pending();
                return Ok(());
            }
        };
        // Compare the pending append's OWN delta, not the full answer
        // prefix: earlier attention/status chunks interleave with answer
        // text in the message, so only the delta is guaranteed contiguous —
        // and look for it PAST the text already applied, so a delta that
        // repeats earlier text is proven by its own occurrence, never the
        // old one.
        let mut landed = false;
        if carried_text && let Some(prefix) = char_prefix(text, pending.to_chars) {
            // `applied` is a char-boundary prefix of `prefix` (both come
            // from `char_prefix`), so slicing past it is safe.
            let applied = char_prefix(prefix, stream.appended_chars).unwrap_or("");
            let expected = normalized_tail(&prefix[applied.len()..]);
            if expected.is_empty() {
                // Punctuation or whitespace only: read-back has nothing to
                // find, so the delta can be proven neither landed nor lost.
                // Never re-send a fragment the user may already see; the
                // pending stays for the host to settle `Unknown`.
                return Err(ReplySinkOutcome::Ambiguous {
                    reason: ReplyOutcomeReason::new(
                        "slack read-back cannot verify a pending append with no comparable \
                         text; not re-sending it",
                    ),
                });
            }
            landed = delta_landed_after(
                &normalize_for_match(&message_text),
                &normalized_tail(applied),
                &expected,
            );
        }
        if landed {
            self.evidence.read_back_verified = true;
            self.apply_state(&pending);
            if pending.closes_stream {
                self.checkpoint.terminal = Some(SlackTerminalState::StreamClosed);
                self.checkpoint.session_status = SlackSessionStatus::Active;
                self.checkpoint.status_asserted_at = Some(Utc::now());
                self.record_ref(&stream.ts);
            }
        } else {
            // Not landed — or unverifiable (a request that carried only task
            // and status chunks leaves no trace in the text). Re-send: a
            // repeated task card upserts by id.
            tracing::debug!(
                carried_text,
                "slack pending append did not land; re-sending"
            );
        }
        self.clear_pending();
        Ok(())
    }

    fn clear_pending(&mut self) {
        if let Some(stream) = self.checkpoint.stream.as_mut() {
            stream.pending = None;
        }
    }

    fn apply_state(&mut self, state: &SlackAppliedState) {
        if let Some(stream) = self.checkpoint.stream.as_mut() {
            stream.appended_chars = state.to_chars;
            stream.appended_hash = state.to_hash.clone();
            stream.pending = None;
        }
        self.checkpoint
            .tasks
            .extend(state.tasks.iter().map(|(id, fp)| (id.clone(), fp.clone())));
        if state.status_key.is_some() {
            self.checkpoint.status_key = state.status_key.clone();
        }
        if state.plan_title_key.is_some() {
            self.checkpoint.plan_title_key = state.plan_title_key.clone();
        }
        if state.attention_key.is_some() {
            self.checkpoint.attention_key = state.attention_key.clone();
        }
        if state.note_key.is_some() {
            self.checkpoint.note_key = state.note_key.clone();
        }
    }

    /// A non-terminal revision: open the presentation if needed, then
    /// append whatever the checkpoint has not seen, then reconcile the
    /// session status.
    async fn progress(&mut self, route: &SlackReplyRoute) -> ReplySinkOutcome {
        let document = self.request.revision.document.clone();
        // At most one re-presentation per call: a rewritten answer closes
        // the stale stream and opens a fresh one with the full text.
        for _ in 0..2 {
            let Some(stream) = self.checkpoint.stream.clone() else {
                return match self.open(route, &document).await {
                    Ok(()) => self.settle_session(route, &document, false).await,
                    Err(outcome) => outcome,
                };
            };
            let plan = match plan_chunks(
                &document,
                &self.checkpoint,
                stream.appended_chars,
                &stream.appended_hash,
                None,
            ) {
                Ok(plan) => plan,
                Err(AnswerRewritten) => {
                    if let Err(outcome) = self.re_present(route, &stream).await {
                        return outcome;
                    }
                    continue;
                }
            };
            if !plan.chunks.is_empty() {
                let body = json!({
                    "channel": stream.channel,
                    "ts": stream.ts,
                    "chunks": plan.chunks,
                });
                match self
                    .api
                    .post(SlackWebApiMethod::ChatAppendStream, body)
                    .await
                {
                    Ok(_) => self.apply_state(&plan.applied),
                    Err(SlackApiFailure::Ambiguous { reason }) => {
                        if let Some(stream) = self.checkpoint.stream.as_mut() {
                            stream.pending = Some(plan.applied);
                        }
                        return ReplySinkOutcome::Ambiguous {
                            reason: ReplyOutcomeReason::new(reason),
                        };
                    }
                    Err(failure) => {
                        return outcome_for_failure(SlackWebApiMethod::ChatAppendStream, failure);
                    }
                }
            }
            return self.settle_session(route, &document, false).await;
        }
        ReplySinkOutcome::Permanent {
            reason: ReplyOutcomeReason::new(
                "slack reply could not be re-presented after the answer was rewritten twice",
            ),
        }
    }

    /// `agents.sessions.setStatus { processing }` then, once the document
    /// holds something renderable, `chat.startStream` carrying it.
    async fn open(
        &mut self,
        route: &SlackReplyRoute,
        document: &ReplyDocument,
    ) -> Result<(), ReplySinkOutcome> {
        if self.checkpoint.stream_open_ambiguous {
            // An earlier `chat.startStream` may have created a stream this
            // sink has no handle for. Slack documents no idempotency key and
            // no way to locate a stream after a lost response, so opening
            // another could show the user two streams; fail closed instead.
            return Err(ReplySinkOutcome::Ambiguous {
                reason: ReplyOutcomeReason::new(
                    "an earlier chat.startStream went unanswered and slack has no way to \
                     locate the stream it may have created; not opening another",
                ),
            });
        }
        self.ensure_session_status(route, SlackSessionStatus::Processing, false)
            .await?;
        let plan = match plan_chunks(document, &self.checkpoint, 0, "", None) {
            Ok(plan) => plan,
            Err(AnswerRewritten) => {
                return Err(ReplySinkOutcome::Permanent {
                    reason: ReplyOutcomeReason::new(
                        "fresh slack stream cannot start from a prefix",
                    ),
                });
            }
        };
        if plan.chunks.is_empty() {
            // Nothing renderable yet (a `Preparing` revision, or an answer
            // still short of its first paragraph): the session status already
            // says the run is thinking, and Slack renders an empty stream as
            // an empty message — so no stream until there is content for it
            // to carry.
            return Ok(());
        }
        self.start_stream(route, &plan).await.map(|_| ())
    }

    /// The one `chat.startStream` call: opens the stream carrying `plan`'s
    /// chunks, establishes the checkpoint's stream state, and latches the
    /// unaddressable-ghost ambiguity (Slack documents no idempotency key and
    /// no way to locate a stream after a lost response).
    async fn start_stream(
        &mut self,
        route: &SlackReplyRoute,
        plan: &ChunkPlan,
    ) -> Result<SlackStreamState, ReplySinkOutcome> {
        let mut body = json!({
            "channel": route.channel,
            "task_display_mode": TASK_DISPLAY_MODE_PLAN,
        });
        if let Some(thread_ts) = &route.thread_ts {
            body["thread_ts"] = json!(thread_ts);
        }
        if let Some(user) = &route.recipient_user_id {
            body["recipient_user_id"] = json!(user);
        }
        if let Some(team) = &route.recipient_team_id {
            body["recipient_team_id"] = json!(team);
        }
        if !plan.chunks.is_empty() {
            body["chunks"] = Value::Array(plan.chunks.clone());
        }
        match self
            .api
            .post(SlackWebApiMethod::ChatStartStream, body)
            .await
        {
            Ok(response) => {
                let ts = response
                    .get("ts")
                    .and_then(Value::as_str)
                    .filter(|ts| !ts.trim().is_empty())
                    .map(str::to_string);
                let Some(ts) = ts else {
                    // Slack accepted the stream but returned no handle for
                    // it — the same unaddressable-ghost shape as a lost
                    // response.
                    self.checkpoint.stream_open_ambiguous = true;
                    return Err(ReplySinkOutcome::Ambiguous {
                        reason: ReplyOutcomeReason::new(
                            "slack chat.startStream answered ok without a message ts",
                        ),
                    });
                };
                let stream = SlackStreamState {
                    channel: route.channel.clone(),
                    ts: ts.clone(),
                    appended_chars: 0,
                    appended_hash: String::new(),
                    opened_at_revision: self.request.revision.revision,
                    pending: None,
                };
                self.checkpoint.stream = Some(stream.clone());
                self.apply_state(&plan.applied);
                self.record_ref(&ts);
                Ok(stream)
            }
            // No ts to read back, and Slack documents no way to find a
            // stream a lost response may have created: mark the checkpoint
            // so no later reconcile opens another stream (or presents the
            // terminal text beside a ghost stream). The host records the
            // ambiguity and settles `Unknown` when it never resolves.
            Err(SlackApiFailure::Ambiguous { reason }) => {
                self.checkpoint.stream_open_ambiguous = true;
                Err(ReplySinkOutcome::Ambiguous {
                    reason: ReplyOutcomeReason::new(reason),
                })
            }
            Err(failure) => Err(outcome_for_failure(
                SlackWebApiMethod::ChatStartStream,
                failure,
            )),
        }
    }

    /// The answer was rewritten under the stream (the prefix Slack shows no
    /// longer matches the document — a call the loop went on past had its
    /// paragraph streamed, or the canonical text differs). Close the stale
    /// stream, retract its message, and forget the presentation; the caller
    /// opens a fresh one with what the document shows now. A stale stream
    /// already closed, or a message already gone (a retraction whose answer
    /// was lost), is the state this wanted.
    async fn re_present(
        &mut self,
        route: &SlackReplyRoute,
        stream: &SlackStreamState,
    ) -> Result<(), ReplySinkOutcome> {
        tracing::debug!(
            channel = %route.channel,
            "slack reply answer was rewritten under the stream; re-presenting"
        );
        let body = json!({
            "channel": stream.channel,
            "ts": stream.ts,
            "session_status": "processing",
        });
        match self.api.post(SlackWebApiMethod::ChatStopStream, body).await {
            Ok(_) => {}
            Err(SlackApiFailure::Rejected { error, .. })
                if matches!(
                    error.as_str(),
                    "message_not_in_streaming_state" | "message_not_found"
                ) => {}
            Err(SlackApiFailure::Ambiguous { reason }) => {
                return Err(ReplySinkOutcome::Ambiguous {
                    reason: ReplyOutcomeReason::new(reason),
                });
            }
            Err(failure) => {
                return Err(outcome_for_failure(
                    SlackWebApiMethod::ChatStopStream,
                    failure,
                ));
            }
        }
        self.retract_message(stream).await?;
        self.checkpoint.forget_presentation();
        Ok(())
    }

    /// `chat.delete` the stale stream's message so the fresh presentation
    /// never stands beside text the document no longer shows. A message
    /// already gone is the retraction this wanted, which also makes a lost
    /// answer plainly retryable rather than ambiguous: the retry finds the
    /// message deleted or deletes it. A Slack refusal (a workspace that
    /// forbids the delete) leaves the stale message and is logged, because
    /// the fresh stream still carries the answer and failing the reply over
    /// it would be worse.
    async fn retract_message(&mut self, stream: &SlackStreamState) -> Result<(), ReplySinkOutcome> {
        let body = json!({ "channel": stream.channel, "ts": stream.ts });
        match self.api.post(SlackWebApiMethod::ChatDelete, body).await {
            Ok(_) => Ok(()),
            Err(SlackApiFailure::Rejected { error, .. }) if error == "message_not_found" => Ok(()),
            Err(SlackApiFailure::Rejected { error, .. }) => {
                tracing::debug!(
                    channel = %stream.channel,
                    ts = %stream.ts,
                    error = %error,
                    "slack refused to retract the stale stream's message; leaving it"
                );
                Ok(())
            }
            Err(SlackApiFailure::Ambiguous { reason }) => Err(ReplySinkOutcome::Retryable {
                reason: ReplyOutcomeReason::new(reason),
                retry_after: None,
            }),
            Err(failure) => Err(outcome_for_failure(SlackWebApiMethod::ChatDelete, failure)),
        }
    }

    /// Reconcile the session status to what the document implies (attention
    /// → `suspended`, otherwise `processing`), re-asserting a stale
    /// `processing` so the one-hour timeout never expires a live run.
    async fn settle_session(
        &mut self,
        route: &SlackReplyRoute,
        document: &ReplyDocument,
        force: bool,
    ) -> ReplySinkOutcome {
        if document.attention.is_none() {
            self.checkpoint.attention_key = None;
        }
        let desired = if document.attention.is_some() {
            SlackSessionStatus::Suspended
        } else {
            SlackSessionStatus::Processing
        };
        let stale = desired == SlackSessionStatus::Processing
            && self.checkpoint.session_status == SlackSessionStatus::Processing
            && self.checkpoint.status_asserted_at.is_none_or(|asserted| {
                Utc::now()
                    .signed_duration_since(asserted)
                    .to_std()
                    .is_ok_and(|age| age >= SESSION_STATUS_REASSERT_AFTER)
            });
        match self
            .ensure_session_status(route, desired, force || stale)
            .await
        {
            Ok(()) => ReplySinkOutcome::Applied,
            Err(outcome) => outcome,
        }
    }

    async fn ensure_session_status(
        &mut self,
        route: &SlackReplyRoute,
        desired: SlackSessionStatus,
        force: bool,
    ) -> Result<(), ReplySinkOutcome> {
        if self.checkpoint.session_unavailable
            || (self.checkpoint.session_status == desired && !force)
        {
            return Ok(());
        }
        let Some(status) = desired.wire() else {
            return Ok(());
        };
        let mut body = json!({
            "status": status,
            "channel_id": route.channel,
        });
        if let Some(thread_ts) = &route.thread_ts {
            body["thread_ts"] = json!(thread_ts);
        }
        match self
            .api
            .post(SlackWebApiMethod::AgentsSessionsSetStatus, body)
            .await
        {
            Ok(_) => {
                self.checkpoint.session_status = desired;
                self.checkpoint.status_asserted_at = Some(Utc::now());
                Ok(())
            }
            Err(SlackApiFailure::Rejected { error, .. })
                if matches!(
                    error.as_str(),
                    "thread_ts_required" | "thread_ts_not_allowed"
                ) =>
            {
                // The conversation shape has no session (a top-level DM
                // without a thread, a session channel). The stream itself
                // still works; only the session indicator is unavailable.
                tracing::debug!(
                    error = %error,
                    "slack session status unavailable for this conversation shape"
                );
                self.checkpoint.session_unavailable = true;
                Ok(())
            }
            Err(failure) => Err(outcome_for_failure(
                SlackWebApiMethod::AgentsSessionsSetStatus,
                failure,
            )),
        }
    }

    /// The terminal revision: close the stream with the remaining delta and
    /// the outcome note (`session_status: active`), or — when no stream was
    /// ever opened — create and close ONE native Agent stream carrying the
    /// terminal content; then upload attachments; then mark the terminal
    /// applied. The terminal answer is never posted as a conventional
    /// message.
    async fn finish(&mut self, route: &SlackReplyRoute) -> ReplySinkOutcome {
        let document = self.request.revision.document.clone();
        if self.checkpoint.terminal.is_none() {
            let outcome = match self.checkpoint.stream.clone() {
                Some(stream) => self.close_stream(route, &document, &stream).await,
                None => self.open_and_close_terminal_stream(route, &document).await,
            };
            if let Err(outcome) = outcome {
                return outcome;
            }
            self.checkpoint.terminal = Some(SlackTerminalState::StreamClosed);
        }
        if !self.request.materialized_attachments.is_empty()
            && !self.checkpoint.attachments_delivered
        {
            if self.checkpoint.attachment_upload_ambiguous {
                // An earlier completion went unanswered: the files may
                // already be visible, and Slack read-back cannot prove a
                // negative here, so nothing is ever re-uploaded. The host
                // records the ambiguity and settles `Unknown`.
                return ReplySinkOutcome::Ambiguous {
                    reason: ReplyOutcomeReason::new(
                        "an earlier slack attachment completion went unanswered; \
                         not re-uploading the files",
                    ),
                };
            }
            let files: Vec<&WorkspaceFile> = self.request.materialized_attachments.iter().collect();
            // One file per provider batch so confirmed progress is durable in
            // the checkpoint: a retry resumes after the last CONFIRMED file
            // instead of re-sending the whole list.
            let start = usize::try_from(self.checkpoint.attachments_progress)
                .unwrap_or(usize::MAX)
                .min(files.len());
            for index in start..files.len() {
                let outcomes = crate::attachment_transfer::send_files(
                    self.api.egress,
                    self.api.credential,
                    &route.channel,
                    route.thread_ts.as_deref(),
                    &files[index..=index],
                )
                .await;
                for outcome in outcomes {
                    match outcome {
                        PartDeliveryOutcome::Sent { vendor_message_ref } => {
                            if let Some(reference) = vendor_message_ref {
                                self.record_ref(&reference);
                            }
                        }
                        PartDeliveryOutcome::Ambiguous { reason } => {
                            self.checkpoint.attachment_upload_ambiguous = true;
                            return ReplySinkOutcome::Ambiguous {
                                reason: ReplyOutcomeReason::new(reason),
                            };
                        }
                        failed => return outcome_for_part(failed),
                    }
                }
                self.checkpoint.attachments_progress = (index + 1) as u64;
            }
            self.checkpoint.attachments_delivered = true;
        }
        self.checkpoint.terminal = Some(SlackTerminalState::Applied);
        ReplySinkOutcome::Applied
    }

    async fn close_stream(
        &mut self,
        route: &SlackReplyRoute,
        document: &ReplyDocument,
        stream: &SlackStreamState,
    ) -> Result<(), ReplySinkOutcome> {
        let note = terminal_note(document);
        let plan = match plan_chunks(
            document,
            &self.checkpoint,
            stream.appended_chars,
            &stream.appended_hash,
            note.as_deref(),
        ) {
            Ok(plan) => plan,
            Err(AnswerRewritten) => {
                // The canonical answer is not an extension of what was
                // streamed (a genuine rewrite — the in-place terminal fold
                // upstream absorbs the ordinary case): close the stale
                // stream as it stands, retract its message, and re-present
                // the canonical text on ONE fresh native stream, opened and
                // closed in this reconcile. Never as a conventional message
                // beside the stream — that is the duplicate-answer shape.
                self.re_present(route, stream).await?;
                return self.open_and_close_terminal_stream(route, document).await;
            }
        };
        let mut applied = plan.applied.clone();
        applied.closes_stream = true;
        self.stop_stream(route, document, stream, plan.chunks, &applied)
            .await
            .map(|_| ())
    }

    /// `chat.stopStream` with `session_status: active`. Returns whether the
    /// stream was closed by this call (`false` when the user's stop button
    /// already ended it and only the session needed settling).
    async fn stop_stream(
        &mut self,
        route: &SlackReplyRoute,
        document: &ReplyDocument,
        stream: &SlackStreamState,
        chunks: Vec<Value>,
        applied: &SlackAppliedState,
    ) -> Result<bool, ReplySinkOutcome> {
        let mut body = json!({
            "channel": stream.channel,
            "ts": stream.ts,
            "session_status": "active",
        });
        if !chunks.is_empty() {
            body["chunks"] = Value::Array(chunks);
        }
        match self.api.post(SlackWebApiMethod::ChatStopStream, body).await {
            Ok(_) => {
                self.apply_state(applied);
                self.checkpoint.session_status = SlackSessionStatus::Active;
                self.checkpoint.status_asserted_at = Some(Utc::now());
                self.record_ref(&stream.ts);
                Ok(true)
            }
            Err(SlackApiFailure::Ambiguous { reason }) => {
                let mut pending = applied.clone();
                pending.closes_stream = true;
                if let Some(state) = self.checkpoint.stream.as_mut() {
                    state.pending = Some(pending);
                }
                Err(ReplySinkOutcome::Ambiguous {
                    reason: ReplyOutcomeReason::new(reason),
                })
            }
            Err(SlackApiFailure::Rejected { error, .. })
                if matches!(
                    error.as_str(),
                    "stopped_by_user" | "message_not_in_streaming_state"
                ) =>
            {
                if error == "stopped_by_user"
                    && !matches!(document.outcome, Some(ReplyOutcome::Cancelled))
                {
                    Err(ReplySinkOutcome::StoppedByUser)
                } else {
                    // The stream is no longer streaming: an earlier close of
                    // ours whose response was lost (verified by exactly this
                    // answer on the re-send — a no-text close leaves nothing
                    // for read-back to compare), or the person's stop button
                    // already ended it. Either way the close this terminal
                    // revision wanted exists; Slack leaves the session to us.
                    self.ensure_session_status(route, SlackSessionStatus::Active, true)
                        .await?;
                    Ok(false)
                }
            }
            Err(failure) => Err(outcome_for_failure(
                SlackWebApiMethod::ChatStopStream,
                failure,
            )),
        }
    }

    /// No live stream exists at the terminal (the run ended before any
    /// renderable content reached Slack, or a rewrite closed the stale one):
    /// the terminal content still goes out on the native Agent surface — ONE
    /// stream created and closed here — never as a conventional message. A
    /// terminal with nothing to show opens nothing and only settles the
    /// session.
    async fn open_and_close_terminal_stream(
        &mut self,
        route: &SlackReplyRoute,
        document: &ReplyDocument,
    ) -> Result<(), ReplySinkOutcome> {
        if self.checkpoint.stream_open_ambiguous {
            // A ghost stream may exist and may already show the chunks the
            // unanswered `chat.startStream` carried; presenting the terminal
            // text beside it could duplicate content. Stay ambiguous; the
            // host settles `Unknown`.
            return Err(ReplySinkOutcome::Ambiguous {
                reason: ReplyOutcomeReason::new(
                    "an earlier chat.startStream went unanswered; the terminal text is \
                     not posted beside a stream slack may have created",
                ),
            });
        }
        let note = terminal_note(document);
        let plan = match plan_chunks(document, &self.checkpoint, 0, "", note.as_deref()) {
            Ok(plan) => plan,
            Err(AnswerRewritten) => {
                return Err(ReplySinkOutcome::Permanent {
                    reason: ReplyOutcomeReason::new(
                        "fresh slack stream cannot start from a prefix",
                    ),
                });
            }
        };
        if plan.chunks.is_empty() {
            // Nothing to show (a completed run with nothing to report): no
            // stream, no message — only the session settling.
            return self
                .ensure_session_status(route, SlackSessionStatus::Active, false)
                .await;
        }
        let stream = self.start_stream(route, &plan).await?;
        let mut applied = plan.applied.clone();
        applied.closes_stream = true;
        self.stop_stream(route, document, &stream, Vec::new(), &applied)
            .await
            .map(|_| ())
    }
}

// ── Route ────────────────────────────────────────────────────────────────

fn resolve_route(request: &ReplyReconcileRequest) -> Result<SlackReplyRoute, ReplyOutcomeReason> {
    let context = request.reply_context.as_ref().and_then(|bytes| {
        match SlackReplyContext::from_bytes(bytes.as_bytes()) {
            Ok(context) => Some(context),
            Err(error) => {
                tracing::debug!(error = %error, "stored slack reply context did not parse");
                None
            }
        }
    });
    let conversation = request.target.conversation.as_ref();
    let channel = match (&context, conversation) {
        (Some(context), _) => context.channel.clone(),
        (None, Some(conversation)) => conversation.conversation_id().to_string(),
        (None, None) => {
            return Err(ReplyOutcomeReason::new(
                "slack reply target names no conversation and carries no reply context",
            ));
        }
    };
    let thread_ts = request
        .target
        .thread_anchor
        .as_ref()
        .map(|anchor| anchor.as_str().to_string())
        .or_else(|| {
            conversation.and_then(|conversation| conversation.topic_id().map(str::to_string))
        })
        .or_else(|| {
            context
                .as_ref()
                .and_then(|context| context.thread_ts.clone())
        });
    let is_dm = context
        .as_ref()
        .map_or_else(|| channel.starts_with('D'), |context| context.is_dm);
    let (recipient_user_id, recipient_team_id) = match &context {
        Some(context) => (Some(context.user.clone()), context.team_id.clone()),
        None => (None, None),
    };
    if !is_dm {
        // `chat.startStream`: recipient_user_id / recipient_team_id are
        // "Required when streaming to channels".
        if recipient_user_id.is_none() {
            return Err(ReplyOutcomeReason::new(format!(
                "streaming into slack conversation {channel} requires recipient_user_id, \
                 which the stored reply context does not carry"
            )));
        }
        if recipient_team_id.is_none() {
            return Err(ReplyOutcomeReason::new(format!(
                "streaming into slack conversation {channel} requires recipient_team_id, \
                 which the stored reply context does not carry"
            )));
        }
    }
    Ok(SlackReplyRoute {
        channel,
        thread_ts,
        recipient_user_id,
        recipient_team_id,
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_extension_contracts::reply::{
        REPLY_SINK_CHECKPOINT_MAX_BYTES, ReplyActivityState, ReplyPhase, ReplyReasoningText,
    };

    use super::checkpoint::fingerprint;
    use super::plan::{markdown_pieces, task_fingerprint};
    use super::*;

    #[test]
    fn char_prefix_never_splits_a_multibyte_char() {
        let text = "héllo 世界";
        assert_eq!(char_prefix(text, 0), Some(""));
        assert_eq!(char_prefix(text, 2), Some("hé"));
        assert_eq!(char_prefix(text, 7), Some("héllo 世"));
        assert_eq!(char_prefix(text, 8), Some(text));
        assert_eq!(char_prefix(text, 9), None);
    }

    #[test]
    fn markdown_pieces_respect_the_chunk_limit_on_char_boundaries() {
        let text = "é".repeat(SLACK_MARKDOWN_CHUNK_MAX_CHARS + 5);
        let pieces = markdown_pieces(&text);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].chars().count(), SLACK_MARKDOWN_CHUNK_MAX_CHARS);
        assert_eq!(pieces[1], "é".repeat(5));
    }

    #[test]
    fn fingerprint_is_stable_and_separator_aware() {
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&["a", "b"]));
        assert_ne!(fingerprint(&["a", "b"]), fingerprint(&["ab", ""]));
        assert_eq!(fingerprint(&[""]).len(), 16);
    }

    #[test]
    fn read_back_normalization_ignores_formatting_but_keeps_content() {
        assert_eq!(normalize_for_match("**bold** _it_ `x` 世界"), "bolditx世界");
        let long = "a".repeat(READ_BACK_TAIL_CHARS + 10);
        assert_eq!(normalized_tail(&long).chars().count(), READ_BACK_TAIL_CHARS);
    }

    fn item(id: &str) -> ironclaw_extension_contracts::reply::ReplyItemId {
        ironclaw_extension_contracts::reply::ReplyItemId::new(id).expect("id")
    }

    fn title(value: &str) -> ironclaw_extension_contracts::reply::ReplyDisplayText {
        ironclaw_extension_contracts::reply::ReplyDisplayText::new(value).expect("title")
    }

    fn text_chunks(plan: &super::plan::ChunkPlan) -> Vec<String> {
        plan.chunks
            .iter()
            .filter(|chunk| chunk["type"] == "markdown_text")
            .map(|chunk| chunk["text"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn plan_for(document: &ReplyDocument) -> super::plan::ChunkPlan {
        plan_chunks(document, &SlackReplyCheckpoint::default(), 0, "", None)
            .expect("a fresh plan is append-only")
    }

    #[test]
    fn a_document_with_nothing_to_show_plans_nothing() {
        // Preparing and Thinking with no text, task, status, or attention:
        // nothing is renderable, so no chunk at all — the session status
        // alone carries the thinking state and the stream stays unopened.
        // No sentinel task is ever invented to make a header render.
        for phase in [ReplyPhase::Preparing, ReplyPhase::Thinking] {
            let document = ReplyDocument {
                phase,
                ..ReplyDocument::default()
            };
            let plan = plan_for(&document);
            assert!(
                plan.chunks.is_empty(),
                "{phase:?} planned {:?}",
                plan.chunks
            );
            assert!(plan.applied.plan_title_key.is_none());
            assert!(plan.applied.tasks.is_empty());
        }
    }

    #[test]
    fn the_plan_header_arrives_with_the_first_real_task() {
        let mut document = ReplyDocument::default();
        document.activity_started(item("act-0"), title("Read runbook"), None);

        let plan = plan_for(&document);

        assert_eq!(
            plan.chunks,
            vec![
                json!({ "type": "plan_update", "title": "Thinking" }),
                json!({ "type": "task_update", "id": "act-0", "title": "Read runbook", "status": "in_progress" }),
            ]
        );
        assert!(
            plan.chunks
                .iter()
                .all(|chunk| chunk.get("hide_title").is_none()),
            "every task card is a real activity with a visible title"
        );
    }

    #[test]
    fn a_tool_less_answer_streams_text_without_a_plan_header() {
        let mut document = ReplyDocument::default();
        document.append_answer("The answer is 19.\n\n");
        document.complete();

        let plan = plan_for(&document);

        assert_eq!(
            plan.chunks,
            vec![json!({ "type": "markdown_text", "text": "The answer is 19.\n\n" })],
            "no task, no header: the text is the whole presentation"
        );
    }

    #[test]
    fn approved_reasoning_never_becomes_a_plan_row() {
        let mut document = ReplyDocument {
            phase: ReplyPhase::Thinking,
            ..ReplyDocument::default()
        };
        document.reasoning =
            vec![ReplyReasoningText::new("Comparing the two trail profiles").expect("reasoning")];
        document.reasoning_open = true;

        assert!(plan_for(&document).chunks.is_empty());

        document.phase = ReplyPhase::Working;
        document.reasoning_open = false;
        assert!(plan_for(&document).chunks.is_empty());
    }

    #[test]
    fn progress_text_is_published_by_paragraph_and_the_rest_is_held() {
        // Slack renders every markdown chunk as its own block, so a chunk
        // must be a complete paragraph: the boundary is published (through
        // its blank line) and the unfinished tail waits.
        let mut document = ReplyDocument::default();
        document.append_answer("First paragraph.\n\nSecond paragraph that is still");

        let plan = plan_for(&document);

        assert_eq!(text_chunks(&plan), vec!["First paragraph.\n\n"]);
        assert_eq!(
            plan.applied.to_chars,
            "First paragraph.\n\n".chars().count() as u64
        );
        assert_eq!(
            plan.applied.to_hash,
            fingerprint(&["First paragraph.\n\n"]),
            "the hash covers exactly the published prefix so a rewrite is still detected"
        );

        // Multibyte text ahead of the boundary never moves it off a char.
        let mut multibyte = ReplyDocument::default();
        multibyte.append_answer("héllo 世界 — ünïcode.\n\nnächster Absatz");
        let plan = plan_for(&multibyte);
        assert_eq!(text_chunks(&plan), vec!["héllo 世界 — ünïcode.\n\n"]);
        assert_eq!(
            plan.applied.to_chars,
            "héllo 世界 — ünïcode.\n\n".chars().count() as u64
        );

        let mut unfinished = ReplyDocument::default();
        unfinished.append_answer("Still typing the first");
        let plan = plan_for(&unfinished);
        assert!(
            text_chunks(&plan).is_empty(),
            "no complete paragraph yet: nothing goes out"
        );
        assert_eq!(plan.applied.to_chars, 0);
    }

    #[test]
    fn held_text_is_not_flushed_ahead_of_an_attention_block() {
        // A gate is raised for a tool call the same model call produced, so
        // text ahead of it is narration by construction: it stays held (and
        // the document resets it once the loop goes on). Only the block
        // itself goes out.
        let mut document = ReplyDocument::default();
        document.append_answer("Let me send the summary.");
        document.require_attention(ironclaw_extension_contracts::reply::ReplyAttention {
            kind: ironclaw_extension_contracts::reply::ReplyAttentionKind::Approval,
            headline: title("Approve sending the summary"),
            body: None,
            action_url: None,
            gate_ref: Some(title("gate:approval-1")),
        });

        let plan = plan_for(&document);

        let chunks = text_chunks(&plan);
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.contains("Let me send the summary.")),
            "held narration does not flush ahead of the block: {chunks:?}"
        );
        assert!(
            chunks
                .iter()
                .any(|chunk| chunk.contains("Approve sending the summary")),
            "the block itself goes out: {chunks:?}"
        );
        assert_eq!(plan.applied.to_chars, 0);
    }

    #[test]
    fn a_terminal_revision_flushes_the_held_text() {
        let mut document = ReplyDocument::default();
        document.append_answer("First paragraph.\n\nSecond, unfinished");
        document.complete();

        let plan = plan_for(&document);

        assert_eq!(
            text_chunks(&plan),
            vec!["First paragraph.\n\nSecond, unfinished"]
        );
        assert_eq!(
            plan.applied.to_chars,
            document.answer.text.as_str().chars().count() as u64
        );
    }

    #[test]
    fn a_blank_line_inside_a_code_fence_is_not_a_paragraph_boundary() {
        let mut document = ReplyDocument::default();
        document.append_answer("Intro:\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\nAfter the");

        let plan = plan_for(&document);
        assert_eq!(
            text_chunks(&plan),
            vec!["Intro:\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\n"]
        );

        // The fence state carries over from the prefix already applied.
        let applied = "Intro:\n\n```rust\n";
        let plan = plan_chunks(
            &document,
            &SlackReplyCheckpoint::default(),
            applied.chars().count() as u64,
            &fingerprint(&[applied]),
            None,
        )
        .expect("the applied prefix is intact");
        assert_eq!(text_chunks(&plan), vec!["fn a() {}\n\nfn b() {}\n```\n\n"]);
    }

    #[test]
    fn a_long_paragraph_flushes_at_a_sentence_boundary_past_the_hold_bound() {
        let sentence = "This sentence keeps going and going. ";
        let mut long = String::new();
        while long.chars().count() <= SLACK_TEXT_HOLD_MAX_CHARS {
            long.push_str(sentence);
        }
        let tail = "Tail without an end";
        long.push_str(tail);
        let mut document = ReplyDocument::default();
        document.append_answer(&long);

        let plan = plan_for(&document);

        let published = text_chunks(&plan).concat();
        assert_eq!(published, long.trim_end_matches(tail));
        assert_eq!(plan.applied.to_chars, published.chars().count() as u64);
    }

    #[test]
    fn an_oversized_task_map_is_evicted_behind_the_floor() {
        use ironclaw_extension_contracts::reply::{ReplyDisplayText, ReplyItemId};
        let mut document = ReplyDocument::default();
        let mut checkpoint = SlackReplyCheckpoint::default();
        for index in 0..256u32 {
            let id = format!("{index:0>120}");
            document.activity_started(
                ReplyItemId::new(&id).expect("id"),
                ReplyDisplayText::new("t").expect("title"),
                None,
            );
            document.activity_finished(
                ReplyItemId::new(&id).expect("id"),
                ReplyActivityState::Completed,
                None,
                None,
            );
        }
        for activity in &document.activities {
            checkpoint
                .tasks
                .insert(activity.id.as_str().to_string(), task_fingerprint(activity));
        }
        let encoded = encode_checkpoint(&mut checkpoint, &document).expect("bounded");
        assert!(encoded.payload().len() <= REPLY_SINK_CHECKPOINT_MAX_BYTES);
        assert!(
            checkpoint.tasks_floor_ordinal > 0,
            "settled rows advance the floor"
        );
        // Every evicted activity row is settled: nothing is re-sent.
        let plan = plan_chunks(&document, &checkpoint, 0, "", None).expect("plan");
        assert!(
            plan.chunks
                .iter()
                .all(|chunk| chunk["type"] != "task_update"),
            "evicted settled rows are never re-sent"
        );
    }
}
