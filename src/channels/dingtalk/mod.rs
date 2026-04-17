//! DingTalk (钉钉) enterprise bot channel via Stream mode (WebSocket).
//!
//! Uses DingTalk's Stream API to maintain a persistent connection without
//! requiring a public IP. Messages arrive over WebSocket and replies are
//! sent via the Robot API.
//!
//! # Features
//!
//! - Stream mode (WebSocket) — no public IP needed
//! - Private chat (1:1) and group chat (@robot)
//! - Text, rich text, image message types
//! - Markdown reply mode
//! - OAuth2 access token management with auto-refresh
//! - Proactive message sending (broadcast)
//!
//! # Configuration
//!
//! ```json5
//! {
//!   "channels": {
//!     "dingtalk": {
//!       "enabled": true,
//!       "clientId": "dingxxxxxx",
//!       "clientSecret": "your-app-secret",
//!       "robotCode": "your-robot-code"  // optional, auto-detected from messages
//!     }
//!   }
//! }
//! ```

mod card_service;
mod connection;
pub mod docs_api;
pub mod feedback;
mod filters;
pub(super) mod media;
pub(super) mod metrics;
pub(super) mod scrubber;
mod send;
mod stream;
mod types;

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lru::LruCache;
use reqwest::Client;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::{CardStreamMode, DingTalkConfig, DingTalkMessageType};
use crate::error::ChannelError;

use types::{CardPhase, CardState, DingTalkMetadata, MarkdownMsgParam};

const MAX_REPLY_TARGETS: usize = 10000;
const REPLY_TARGETS_CAP: NonZeroUsize = NonZeroUsize::new(MAX_REPLY_TARGETS).unwrap();
const DEFAULT_DINGTALK_API_BASE_URL: &str = "https://api.dingtalk.com";

/// Final-overwrite variants. Drives the first-line icon + summary on the
/// terminal PUT; `render_final` picks the appropriate zh-CN template.
#[derive(Debug)]
enum TerminalKind {
    /// Agent completed normally; `body` is the final answer text.
    Finished { body: String },
    /// Agent failed irrecoverably; `reason` is the scrubbed bucket message,
    /// `partial` is whatever content had already been streamed (may be empty).
    Failed { reason: String, partial: String },
    /// User sent `/stop`; keep any partial streamed answer so they don't
    /// lose what they already saw.
    CancelledByStop { partial: String },
    /// User recalled the original message — wipe partial content.
    CancelledByRecall,
    /// A newer in-flight card superseded this one (same conversation+user).
    CancelledBySupersede,
}

/// Semantic error buckets (R11/R12/R13). Coarse and stable — every
/// ToolCompleted error gets classified into one of these before rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorBucket {
    EmptyResult,
    Network,
    Timeout,
    RateLimit,
    Permission,
    Unknown,
}

/// Heuristic classifier on a free-text error message. Intentionally coarse
/// — the scope boundary says we don't need per-tool mappings yet.
fn classify_error_message(err: &str) -> ErrorBucket {
    let lower = err.to_ascii_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") || lower.contains("deadline") {
        ErrorBucket::Timeout
    } else if lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
    {
        ErrorBucket::RateLimit
    } else if lower.contains("403")
        || lower.contains("401")
        || lower.contains("forbidden")
        || lower.contains("unauthorized")
        || lower.contains("permission")
    {
        ErrorBucket::Permission
    } else if lower.contains("empty")
        || lower.contains("no result")
        || lower.contains("not found")
        || lower.contains("404")
    {
        ErrorBucket::EmptyResult
    } else if lower.contains("5xx")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("connection")
        || lower.contains("network")
        || lower.contains("dns")
    {
        ErrorBucket::Network
    } else {
        ErrorBucket::Unknown
    }
}

/// zh-CN user-facing message per bucket. Stable strings — never
/// interpolate error details into these (they're seen by bystanders in
/// group chats).
fn bucket_message(bucket: ErrorBucket) -> &'static str {
    match bucket {
        ErrorBucket::EmptyResult => {
            "❌ 外部查询暂无结果，正在改用其他方式。可回复 /retry 或重新提问。"
        }
        ErrorBucket::Network => "❌ 网络请求失败。可回复 /retry 或重新提问。",
        ErrorBucket::Timeout => "❌ 操作超时。可回复 /retry 或重新提问。",
        ErrorBucket::RateLimit => "❌ 访问频率过高，请稍后重试。",
        ErrorBucket::Permission => "❌ 该操作无权限。请联系管理员或重新提问。",
        ErrorBucket::Unknown => "❌ 遇到问题。可回复 /retry 或重新提问。",
    }
}

/// Standalone renderer used by the per-card tick task (and anywhere else
/// we can't hold `&DingTalkChannel`). Keep `render` and this in lock-step.
fn render_static(
    state: &CardState,
    now: std::time::Instant,
    card_stream_mode: CardStreamMode,
) -> Option<String> {
    use crate::channels::dingtalk::types::{AgentPhase, SlowTier};

    let elapsed_s = now.saturating_duration_since(state.created_at).as_secs();
    let phase_label = state.agent_phase.label();

    let mut bits: Vec<String> = Vec::new();
    bits.push(phase_label.to_string());

    if state.agent_phase == AgentPhase::UsingTool
        && let Some(tool) = &state.current_tool
        && !tool.summary.is_empty()
    {
        bits.push(format!(" · {}", tool.summary));
    }

    if state.agent_phase == AgentPhase::Thinking
        && state.reasoning_summary_enabled
        && let Some(ref excerpt) = state.reasoning_excerpt
        && !excerpt.is_empty()
    {
        bits.push(format!(" · {excerpt}"));
    }

    bits.push(format!(" ({elapsed_s}s)"));

    let slow_suffix = match state.slow_tier {
        SlowTier::None => "",
        SlowTier::Warn => " ⚠️ 耗时较长，稍等",
        SlowTier::Critical => " ⚠️ 耗时较长，如需取消可回复 /stop",
    };
    if !slow_suffix.is_empty() {
        bits.push(slow_suffix.to_string());
    }

    let status_line = bits.join("");

    let thinking = if card_stream_mode == CardStreamMode::All {
        state.thinking_buffer.trim()
    } else {
        ""
    };
    let content = state.content_buffer.trim();

    let body = match (thinking.is_empty(), content.is_empty()) {
        (true, true) => String::new(),
        (false, true) => thinking.to_string(),
        (true, false) => content.to_string(),
        (false, false) => format!("{thinking}\n\n{content}"),
    };

    if body.is_empty() {
        Some(status_line)
    } else {
        Some(format!("{status_line}\n\n{body}"))
    }
}

/// DingTalk channel using Stream mode (persistent WebSocket).
pub struct DingTalkChannel {
    config: DingTalkConfig,
    client: Client,
    reply_targets: Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
    /// Cached access token with expiry.
    access_token: Arc<RwLock<Option<(String, std::time::Instant)>>>,
    /// Active AI card states, keyed by message UUID.
    card_states: Arc<RwLock<std::collections::HashMap<Uuid, CardState>>>,
    /// Per-message lock to serialize card status updates.
    status_locks: Arc<Mutex<std::collections::HashMap<Uuid, Arc<Mutex<()>>>>>,
    /// Notify handle to trigger WebSocket reconnect on reconfigure.
    reconnect_notify: Arc<tokio::sync::Notify>,
    /// Conversations that have received a stop/interrupt signal recently.
    stopped_conversations: Arc<RwLock<std::collections::HashMap<String, std::time::Instant>>>,
    /// Cancellation token: fired by shutdown() so run_stream_listener exits
    /// cleanly (the tokio::spawn task can then be joined or dropped safely).
    /// Without this, `ChannelManager::hot_add` on /api/reconfigure leaves
    /// the old WebSocket task running — competing with the replacement and
    /// populating an orphaned `reply_targets` map. Reliable single-instance
    /// behavior across reconfigures depends on this actually stopping.
    shutdown_signal: Arc<tokio::sync::Notify>,
    /// JoinHandle of the spawned stream task, behind a mutex so shutdown()
    /// can reclaim and abort it even if someone else holds a read lock.
    stream_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Active card count across this channel process. Used to enforce
    /// `DingTalkConfig::max_active_cards`. Incremented on card create,
    /// decremented on cleanup.
    active_card_count: Arc<std::sync::atomic::AtomicU32>,
    /// `(conversation_id, originating_user_id) → msg_id` index for detecting
    /// supersedes when a user sends a new message while a prior card is
    /// in-flight (see Unit 7).
    conv_user_to_card: Arc<RwLock<std::collections::HashMap<(String, String), Uuid>>>,
}

impl DingTalkChannel {
    pub fn new(config: DingTalkConfig) -> Result<Self, ChannelError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        Ok(Self {
            config,
            client,
            reply_targets: Arc::new(RwLock::new(LruCache::new(REPLY_TARGETS_CAP))),
            access_token: Arc::new(RwLock::new(None)),
            card_states: Arc::new(RwLock::new(std::collections::HashMap::new())),
            status_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
            reconnect_notify: Arc::new(tokio::sync::Notify::new()),
            stopped_conversations: Arc::new(RwLock::new(std::collections::HashMap::new())),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
            stream_task: Mutex::new(None),
            active_card_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            conv_user_to_card: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Get the reconnect notify handle.
    ///
    /// Callers (e.g. the gateway reconfigure handler) can call `notify_one()` on this
    /// to trigger the DingTalk Stream WebSocket to reconnect with fresh config.
    pub fn reconnect_notify(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.reconnect_notify)
    }

    pub(super) fn api_url(path: &str) -> String {
        let base = std::env::var("IRONCLAW_TEST_DINGTALK_API_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_DINGTALK_API_BASE_URL.to_string());
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn status_can_activate_card(status: &StatusUpdate) -> bool {
        matches!(
            status,
            StatusUpdate::StreamChunk(_)
                | StatusUpdate::Thinking(_)
                | StatusUpdate::ToolStarted { .. }
                | StatusUpdate::ToolCompleted { .. }
                | StatusUpdate::PhaseChanged(_)
        )
    }

    fn card_delivery_enabled(&self) -> bool {
        self.config.message_type == DingTalkMessageType::Card
            && self.config.card_template_id.is_some()
    }

    fn status_supports_live_flush(&self, status: &StatusUpdate) -> bool {
        match status {
            StatusUpdate::StreamChunk(_) => self.config.card_stream_mode != CardStreamMode::Off,
            StatusUpdate::Thinking(_)
            | StatusUpdate::ToolStarted { .. }
            | StatusUpdate::ToolCompleted { .. } => {
                self.config.card_stream_mode == CardStreamMode::All
            }
            _ => false,
        }
    }

    /// Compose the DingTalk AI card body from the current [`CardState`] at
    /// wall-clock `now`. This is the single authoritative renderer — every
    /// PUT (tick-driven, event-driven, or terminal) goes through here.
    ///
    /// Layout (zh-CN literals baked in per scope boundary):
    /// ```text
    /// {🧠|🔧|✍️} {phase}{ · <tool-summary>}{ · 最近思路：...}{ (Ns)}{ ⚠️ 耗时较长，稍等}
    ///
    /// {accumulated answer / streaming content, optional}
    /// ```
    fn render(&self, state: &CardState, now: std::time::Instant) -> Option<String> {
        use crate::channels::dingtalk::types::{AgentPhase, SlowTier};

        // Cumulative seconds since card creation — never reset across phases.
        let elapsed_s = now.saturating_duration_since(state.created_at).as_secs();

        let phase_label = state.agent_phase.label();

        // Optional pieces.
        let mut bits: Vec<String> = Vec::new();
        bits.push(phase_label.to_string());

        if state.agent_phase == AgentPhase::UsingTool {
            if let Some(tool) = &state.current_tool {
                if !tool.summary.is_empty() {
                    bits.push(format!(" · {}", tool.summary));
                }
            }
        }

        // Reasoning excerpt (capped + escaped upstream).
        if state.agent_phase == AgentPhase::Thinking
            && state.reasoning_summary_enabled
            && let Some(ref excerpt) = state.reasoning_excerpt
            && !excerpt.is_empty()
        {
            bits.push(format!(" · {excerpt}"));
        }

        bits.push(format!(" ({elapsed_s}s)"));

        // Slow-op suffix. Kept on the same line so the status row stays a
        // single logical line for mobile rendering.
        let slow_suffix = match state.slow_tier {
            SlowTier::None => "",
            SlowTier::Warn => " ⚠️ 耗时较长，稍等",
            SlowTier::Critical => " ⚠️ 耗时较长，如需取消可回复 /stop",
        };
        if !slow_suffix.is_empty() {
            bits.push(slow_suffix.to_string());
        }

        let status_line = bits.join("");

        // Answer body (during ✍️ phase). Optional thinking buffer rendering
        // preserved for `card_stream_mode = all` parity with prior behavior.
        let thinking = if self.config.card_stream_mode == CardStreamMode::All {
            state.thinking_buffer.trim()
        } else {
            ""
        };
        let content = state.content_buffer.trim();

        let body = match (thinking.is_empty(), content.is_empty()) {
            (true, true) => String::new(),
            (false, true) => thinking.to_string(),
            (true, false) => content.to_string(),
            (false, false) => format!("{thinking}\n\n{content}"),
        };

        if body.is_empty() {
            Some(status_line)
        } else {
            Some(format!("{status_line}\n\n{body}"))
        }
    }

    /// Back-compat alias: existing call sites use `rendered_card_content`.
    /// Delegates to [`Self::render`] with the current wall clock.
    fn rendered_card_content(&self, state: &CardState) -> Option<String> {
        self.render(state, std::time::Instant::now())
    }

    /// Self-contained render used by the per-card tick task. Accepts only
    /// the fields needed from `DingTalkConfig` so the tick task doesn't
    /// need `&Self`.
    fn render_self_contained(
        state: &CardState,
        now: std::time::Instant,
        card_stream_mode: CardStreamMode,
    ) -> Option<String> {
        render_static(state, now, card_stream_mode)
    }

    /// Compose the final overwrite body on terminal transition. Replaces
    /// the whole card content with "icon summary / divider / final answer".
    ///
    /// `tools_used` + `elapsed_s` come from [`CardState`]; `body` is the
    /// finalized answer text (for FINISHED) or error description.
    fn render_final(
        &self,
        state: &CardState,
        terminal: TerminalKind,
        now: std::time::Instant,
    ) -> String {
        let elapsed_s = now.saturating_duration_since(state.created_at).as_secs();
        let n = state.tools_used;

        match terminal {
            TerminalKind::Finished { body } => {
                let summary = format!("✅ 本次调用 {n} 个工具·{elapsed_s}s");
                if body.trim().is_empty() {
                    summary
                } else {
                    format!("{summary}\n\n---\n\n{body}")
                }
            }
            TerminalKind::Failed { reason, partial } => {
                let summary = format!("❌ {reason}");
                if partial.trim().is_empty() {
                    summary
                } else {
                    format!("{summary}\n\n---\n\n{partial}")
                }
            }
            TerminalKind::CancelledByStop { partial } => {
                let summary = format!("⏹ 已取消·已用 {n} 个工具·{elapsed_s}s");
                if partial.trim().is_empty() {
                    summary
                } else {
                    format!("{summary}\n\n---\n\n{partial}")
                }
            }
            TerminalKind::CancelledByRecall => {
                "⏹ 原问题已撤回，已停止".to_string()
            }
            TerminalKind::CancelledBySupersede => {
                format!("⏹ 被新问题替代·已用 {n} 个工具·{elapsed_s}s")
            }
        }
    }

    fn append_line(buffer: &mut String, line: &str) {
        if line.is_empty() {
            return;
        }
        if !buffer.is_empty() && !buffer.ends_with('\n') {
            buffer.push('\n');
        }
        buffer.push_str(line);
    }

    /// Spawn a per-card tick task. Its sole side effect is calling
    /// [`card_service::stream_ai_card`] with a freshly-rendered body at
    /// `status_tick_ms` cadence during NON-`✍️ Generating` phases. In
    /// `Generating`, token-stream chunks own the PUT channel — the tick
    /// task updates slow_tier in-state but skips the PUT.
    ///
    /// Cancellation: owner fires `cancel.notify_one()`; the task exits on
    /// the next `tokio::select!` branch. Cleanup awaits the JoinHandle
    /// with a bounded grace period (see [`Self::cleanup_message_state`]).
    ///
    /// Logging: `debug!` only (runtime-log contract — `info!` in a 2s
    /// interval task would corrupt the REPL/TUI, see
    /// `docs/solutions/ironclaw-runtime-logging-pattern.md`).
    fn spawn_tick_task(
        msg_id: Uuid,
        instance_id: String,
        card_states: Arc<RwLock<std::collections::HashMap<Uuid, CardState>>>,
        access_token: Arc<RwLock<Option<(String, std::time::Instant)>>>,
        client: Client,
        config: DingTalkConfig,
        cancel: Arc<tokio::sync::Notify>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let tick_interval = std::cmp::max(config.status_tick_ms, 250);
            let period = std::time::Duration::from_millis(tick_interval);
            // `interval_at` with a start = now + period skips the immediate
            // first tick; the event-driven flushes in send_status cover the
            // activation + first-chunk PUTs, and the tick task's job is to
            // cover SUBSEQUENT silent periods.
            let mut interval = tokio::time::interval_at(
                tokio::time::Instant::now() + period,
                period,
            );
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = cancel.notified() => {
                        tracing::debug!(
                            channel = "dingtalk",
                            msg_id = %msg_id,
                            "tick task cancelled"
                        );
                        return;
                    }
                    _ = interval.tick() => {
                        // Snapshot state under read lock + decide whether to
                        // escalate slow_tier under write lock, all before
                        // the network PUT (keep locks bounded).
                        let mut should_flush = true;
                        let rendered: Option<String> = {
                            let mut states = card_states.write().await;
                            let Some(state) = states.get_mut(&msg_id) else {
                                return; // owner removed state → exit loop
                            };
                            if state.fallback_required {
                                return;
                            }

                            let now = std::time::Instant::now();
                            let elapsed_s = now
                                .saturating_duration_since(state.created_at)
                                .as_secs();

                            // Slow-tier monotonic escalation.
                            use crate::channels::dingtalk::types::SlowTier;
                            let (warn_at, critical_at) = config.slow_threshold_secs;
                            if state.slow_tier == SlowTier::None && elapsed_s >= warn_at {
                                state.slow_tier = SlowTier::Warn;
                            }
                            if state.slow_tier != SlowTier::Critical && elapsed_s >= critical_at {
                                state.slow_tier = SlowTier::Critical;
                            }

                            // During Generating phase, token chunks own the
                            // PUT channel — tick task does not emit.
                            use crate::channels::dingtalk::types::AgentPhase;
                            if state.agent_phase == AgentPhase::Generating {
                                should_flush = false;
                            }

                            // Degraded mode (>= max_active_cards): only flush
                            // on slow-tier threshold crossings — NOT on every
                            // interval.
                            if state.tick_degraded {
                                let just_crossed_warn =
                                    state.slow_tier == SlowTier::Warn && elapsed_s == warn_at;
                                let just_crossed_critical = state.slow_tier
                                    == SlowTier::Critical
                                    && elapsed_s == critical_at;
                                if !(just_crossed_warn || just_crossed_critical) {
                                    should_flush = false;
                                }
                            }

                            if should_flush {
                                render_static(state, now, config.card_stream_mode)
                            } else {
                                None
                            }
                        };

                        if let (true, Some(body)) = (should_flush, rendered) {
                            // Fetch current access token; skip this tick if
                            // we can't (next tick retries).
                            let token = {
                                let guard = access_token.read().await;
                                guard.as_ref().map(|(t, _)| t.clone())
                            };
                            let Some(token) = token else {
                                tracing::debug!(
                                    channel = "dingtalk",
                                    msg_id = %msg_id,
                                    "tick skipped — no access token"
                                );
                                continue;
                            };
                            if let Err(e) = card_service::stream_ai_card(
                                &client,
                                &token,
                                &instance_id,
                                &body,
                                &config.card_template_key,
                                false,
                                false,
                            )
                            .await
                            {
                                tracing::warn!(
                                    channel = "dingtalk",
                                    msg_id = %msg_id,
                                    error = %e,
                                    "tick PUT failed"
                                );
                            }
                        }
                    }
                }
            }
        })
    }

    /// Drain a prior in-flight card to a `⏹ supersede` terminal state
    /// before its successor's tick task starts. Best-effort: if the
    /// finalize PUT or access-token fetch fails, we still clean up local
    /// state so we don't leak.
    async fn supersede_card(&self, prior_msg_id: Uuid) {
        let final_body = {
            let states = self.card_states.read().await;
            states.get(&prior_msg_id).map(|state| {
                (
                    state.instance_id.clone(),
                    state.fallback_required,
                    self.render_final(
                        state,
                        TerminalKind::CancelledBySupersede,
                        std::time::Instant::now(),
                    ),
                )
            })
        };

        if let Some((instance_id, fallback_required, body)) = final_body
            && !fallback_required
            && !instance_id.is_empty()
            && let Ok(token) = self.get_access_token().await
        {
            let _ = card_service::finalize_ai_card(
                &self.client,
                &self.config,
                &token,
                &instance_id,
                &body,
            )
            .await;
        }

        self.cleanup_message_state(prior_msg_id).await;
    }

    async fn cleanup_message_state(&self, msg_id: Uuid) {
        // Capture any tick handle so we can await it outside the map lock.
        let (tick_handle, tick_cancel, conv_user_key, tick_was_present) = {
            let mut states = self.card_states.write().await;
            match states.remove(&msg_id) {
                Some(mut state) => {
                    let handle = state.tick_handle.take();
                    let cancel = state.tick_cancel.clone();
                    let key = (
                        // Note: we don't have conversation_id on CardState; look up via
                        // reply_targets below if needed. For now, decrement by user-id
                        // alone is handled in the secondary-index cleanup path.
                        String::new(),
                        state.originating_user_id.clone(),
                    );
                    let was_present = !state.instance_id.is_empty();
                    (handle, Some(cancel), Some(key), was_present)
                }
                None => (None, None, None, false),
            }
        };

        // Reply metadata has the conversation_id we need for the secondary
        // index cleanup. Grab it before we drop it from reply_targets.
        let conv_id = self
            .reply_targets
            .read()
            .await
            .peek(&msg_id)
            .map(|m| m.conversation_id.clone());

        self.reply_targets.write().await.pop(&msg_id);
        self.status_locks.lock().await.remove(&msg_id);

        if let (Some(conv_id), Some(mut key)) = (conv_id, conv_user_key) {
            key.0 = conv_id;
            let mut idx = self.conv_user_to_card.write().await;
            // Only remove if the index still points at us — avoid clobbering
            // a newer card that superseded us.
            if idx.get(&key) == Some(&msg_id) {
                idx.remove(&key);
            }
        }

        // Cancel + drain tick task with a bounded grace period (5s, mirroring
        // shutdown()'s pattern). Do this LAST so we don't hold map locks.
        if let Some(cancel) = tick_cancel {
            cancel.notify_one();
        }
        if let Some(handle) = tick_handle {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }

        if tick_was_present {
            self.active_card_count
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    async fn mark_card_fallback_required(&self, msg_id: Uuid) {
        let mut states = self.card_states.write().await;
        states
            .entry(msg_id)
            .and_modify(|state| state.fallback_required = true)
            .or_insert_with(|| CardState {
                instance_id: String::new(),
                content_buffer: String::new(),
                thinking_buffer: String::new(),
                last_content_update: None,
                phase: CardPhase::Processing,
                fallback_required: true,
                created_at: std::time::Instant::now(),
                // Fail-closed: assume Group until proven otherwise.
                channel_level: crate::channels::dingtalk::types::ChannelLevel::Group,
                agent_phase: crate::channels::dingtalk::types::AgentPhase::Thinking,
                current_tool: None,
                reasoning_excerpt: None,
                reasoning_summary_enabled: false,
                slow_tier: crate::channels::dingtalk::types::SlowTier::None,
                tick_cancel: std::sync::Arc::new(tokio::sync::Notify::new()),
                tick_handle: None,
                tick_degraded: false,
                seen_sensitive: std::collections::HashSet::new(),
                originating_user_id: String::new(),
                tools_used: 0,
                retry_attempt: 0,
            });
    }

    async fn ensure_card_ready(&self, msg_id: Uuid) -> bool {
        if !self.card_delivery_enabled() {
            return false;
        }

        {
            let states = self.card_states.read().await;
            if let Some(state) = states.get(&msg_id) {
                return !state.fallback_required;
            }
        }

        let (reply_meta, cache_len, has_any_entries) = {
            let targets = self.reply_targets.read().await;
            (
                targets.peek(&msg_id).cloned(),
                targets.len(),
                targets.len() > 0,
            )
        };
        let Some(reply_meta) = reply_meta else {
            tracing::warn!(
                msg_id = %msg_id,
                client_id = %self.config.client_id,
                reply_targets_len = cache_len,
                has_any_entries,
                "No reply metadata for card creation — cache miss \
                 (zombie channel instance after hot_add is the usual cause)"
            );
            self.mark_card_fallback_required(msg_id).await;
            return false;
        };

        let token = match self.get_access_token().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to get token for card creation");
                self.mark_card_fallback_required(msg_id).await;
                return false;
            }
        };

        tracing::info!(
            msg_id = %msg_id,
            conversation_id = %reply_meta.conversation_id,
            conversation_type = %reply_meta.conversation_type,
            "Creating DingTalk AI card"
        );

        let instance_id = match card_service::create_ai_card(
            &self.client,
            &self.config,
            &token,
            &reply_meta.conversation_id,
            &reply_meta.conversation_type,
            &reply_meta.sender_staff_id,
        )
        .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to create AI card, will fall back to Markdown");
                self.mark_card_fallback_required(msg_id).await;
                return false;
            }
        };

        tracing::info!(out_track_id = %instance_id, "DingTalk AI card created");

        // Group vs DM is derived from DingTalk's conversation_type.
        // Per stream.rs, "2" = group, otherwise DM. Fail-closed to Group on
        // anything ambiguous so bystander privacy holds by default.
        let channel_level = if reply_meta.conversation_type == "2" {
            crate::channels::dingtalk::types::ChannelLevel::Group
        } else if reply_meta.conversation_type == "1" {
            crate::channels::dingtalk::types::ChannelLevel::Dm
        } else {
            crate::channels::dingtalk::types::ChannelLevel::Group
        };

        let originating_user_id = reply_meta.sender_staff_id.clone();

        // Active-card cap: the 1001st card still ships but runs degraded.
        let tick_degraded = self
            .active_card_count
            .load(std::sync::atomic::Ordering::Relaxed)
            >= self.config.max_active_cards;

        // Maintain (conversation, user) → msg_id secondary index and
        // detect supersede: if a prior in-flight card exists for the same
        // (conv, user), drain it to a ⏹ terminal before activating the
        // new card's tick loop.
        let prior_msg_id: Option<Uuid> = {
            let mut idx = self.conv_user_to_card.write().await;
            let key = (
                reply_meta.conversation_id.clone(),
                originating_user_id.clone(),
            );
            let prior = idx.get(&key).copied();
            idx.insert(key, msg_id);
            prior
        };
        if let Some(prior_id) = prior_msg_id
            && prior_id != msg_id
        {
            self.supersede_card(prior_id).await;
        }

        self.active_card_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Per-card tick task: drives R1 (2s status-line refresh) and R9
        // (15s/60s slow-tier escalation) during silent periods when no
        // StatusUpdate event naturally triggers a flush. The token-stream
        // phase suppresses its own PUTs (handled inside the task body) so
        // answer chunks own the overwrite channel during ✍️.
        let tick_cancel = std::sync::Arc::new(tokio::sync::Notify::new());
        let tick_handle = if self.config.status_tick_ms > 0 {
            Some(Self::spawn_tick_task(
                msg_id,
                instance_id.clone(),
                std::sync::Arc::clone(&self.card_states),
                std::sync::Arc::clone(&self.access_token),
                self.client.clone(),
                self.config.clone(),
                std::sync::Arc::clone(&tick_cancel),
            ))
        } else {
            None
        };

        let mut states = self.card_states.write().await;
        states.entry(msg_id).or_insert_with(|| CardState {
            instance_id,
            content_buffer: String::new(),
            thinking_buffer: String::new(),
            last_content_update: None,
            phase: CardPhase::Inputing,
            fallback_required: false,
            created_at: std::time::Instant::now(),
            channel_level,
            agent_phase: crate::channels::dingtalk::types::AgentPhase::Thinking,
            current_tool: None,
            reasoning_excerpt: None,
            reasoning_summary_enabled: false,
            slow_tier: crate::channels::dingtalk::types::SlowTier::None,
            tick_cancel,
            tick_handle,
            tick_degraded,
            seen_sensitive: std::collections::HashSet::new(),
            originating_user_id,
            tools_used: 0,
            retry_attempt: 0,
        });

        true
    }

    async fn flush_card_if_needed(&self, msg_id: Uuid, force: bool) {
        let pending = {
            let mut states = self.card_states.write().await;
            let Some(state) = states.get_mut(&msg_id) else {
                return;
            };

            if state.fallback_required {
                return;
            }

            let Some(content) = self.rendered_card_content(state) else {
                return;
            };

            let now = std::time::Instant::now();
            let should_flush = if force {
                true
            } else {
                match state.last_content_update {
                    None => true,
                    Some(last) => {
                        now.duration_since(last)
                            >= Duration::from_millis(self.config.card_stream_interval_ms)
                    }
                }
            };

            if !should_flush {
                return;
            }

            state.last_content_update = Some(now);
            Some((state.instance_id.clone(), content))
        };

        let Some((instance_id, content)) = pending else {
            return;
        };

        let token = match self.get_access_token().await {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!(error = %e, "Failed to get token for card stream");
                self.mark_card_fallback_required(msg_id).await;
                return;
            }
        };

        if let Err(e) = card_service::stream_ai_card(
            &self.client,
            &token,
            &instance_id,
            &content,
            &self.config.card_template_key,
            false,
            false,
        )
        .await
        {
            tracing::warn!(error = %e, "Failed to stream AI card update");
            self.mark_card_fallback_required(msg_id).await;
        }
    }

    #[cfg(any(test, feature = "integration"))]
    pub async fn seed_reply_target_for_test(
        &self,
        message: &IncomingMessage,
    ) -> Result<(), ChannelError> {
        let metadata: DingTalkMetadata =
            serde_json::from_value(message.metadata.clone()).map_err(|e| {
                ChannelError::SendFailed {
                    name: "dingtalk".into(),
                    reason: format!("invalid DingTalk metadata for test seeding: {e}"),
                }
            })?;

        self.reply_targets.write().await.put(message.id, metadata);
        Ok(())
    }

    /// Get a valid access token, refreshing if expired.
    async fn get_access_token(&self) -> Result<String, ChannelError> {
        // Check cache first
        {
            let cache = self.access_token.read().await;
            if let Some((ref token, expiry)) = *cache {
                if expiry > std::time::Instant::now() {
                    return Ok(token.clone());
                }
            }
        }

        // Fetch new token
        use secrecy::ExposeSecret;
        let resp = self
            .client
            .post(Self::api_url("/v1.0/oauth2/accessToken"))
            .json(&serde_json::json!({
                "appKey": self.config.client_id,
                "appSecret": self.config.client_secret.expose_secret(),
            }))
            .send()
            .await
            .map_err(|e| ChannelError::Http(format!("token request: {e}")))?;

        let token_resp_value = send::parse_business_response(resp, "token API")
            .await?
            .ok_or_else(|| ChannelError::Http("token API returned empty body".to_string()))?;
        let token = token_resp_value
            .get("accessToken")
            .or_else(|| token_resp_value.get("access_token"))
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("accessToken"))
            })
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("access_token"))
            })
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ChannelError::Http("no access_token in response".to_string()))?
            .to_string();

        let expires_in = token_resp_value
            .get("expireIn")
            .or_else(|| token_resp_value.get("expiresIn"))
            .or_else(|| token_resp_value.get("expires_in"))
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("expireIn"))
            })
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("expiresIn"))
            })
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("expires_in"))
            })
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
            })
            .unwrap_or(7200);
        // Refresh 5 minutes before expiry
        let expiry =
            std::time::Instant::now() + Duration::from_secs(expires_in.saturating_sub(300));

        let mut cache = self.access_token.write().await;
        *cache = Some((token.clone(), expiry));

        Ok(token)
    }

    fn is_terminal_status_message(msg: &str) -> bool {
        let trimmed = msg.trim();
        trimmed.eq_ignore_ascii_case("done")
            || trimmed.eq_ignore_ascii_case("interrupted")
            || trimmed.eq_ignore_ascii_case("awaiting approval")
            || trimmed.eq_ignore_ascii_case("rejected")
    }

    /// Send a markdown message via Robot API.
    async fn send_markdown(
        &self,
        token: &str,
        robot_code: &str,
        conversation_id: Option<&str>,
        user_ids: &[&str],
        title: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        let msg_param = serde_json::to_string(&MarkdownMsgParam {
            title: title.to_string(),
            text: text.to_string(),
        })
        .map_err(|e| ChannelError::Http(format!("serialize: {e}")))?;

        let mut body = serde_json::json!({
            "msgKey": "sampleMarkdown",
            "msgParam": msg_param,
            "robotCode": robot_code,
        });

        let url = if let Some(conv_id) = conversation_id {
            body["openConversationId"] = serde_json::Value::String(conv_id.to_string());
            Self::api_url("/v1.0/robot/groupMessages/send")
        } else {
            body["userIds"] = serde_json::Value::Array(
                user_ids
                    .iter()
                    .map(|u| serde_json::Value::String(u.to_string()))
                    .collect(),
            );
            Self::api_url("/v1.0/robot/oToMessages/batchSend")
        };

        let resp = self
            .client
            .post(url)
            .header("x-acs-dingtalk-access-token", token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Http(format!("send: {e}")))?;

        send::ensure_business_success(resp, "Robot API").await
    }
}

#[async_trait]
impl Channel for DingTalkChannel {
    fn name(&self) -> &str {
        "dingtalk"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        let config = self.config.clone();
        let client = self.client.clone();
        let reply_targets = Arc::clone(&self.reply_targets);
        let reconnect_notify = Arc::clone(&self.reconnect_notify);
        let stopped_conversations = Arc::clone(&self.stopped_conversations);
        let shutdown_signal = Arc::clone(&self.shutdown_signal);
        let client_id_for_log = self.config.client_id.clone();

        let handle = tokio::spawn(async move {
            tracing::info!(
                client_id = %client_id_for_log,
                "DingTalk stream task starting"
            );
            tokio::select! {
                res = stream::run_stream_listener(
                    config,
                    client,
                    tx,
                    reply_targets,
                    reconnect_notify,
                    stopped_conversations,
                ) => {
                    match res {
                        Ok(()) => tracing::info!(
                            client_id = %client_id_for_log,
                            "DingTalk stream task exited cleanly"
                        ),
                        Err(e) => tracing::error!(
                            client_id = %client_id_for_log,
                            error = %e,
                            "DingTalk Stream listener exited with error"
                        ),
                    }
                }
                _ = shutdown_signal.notified() => {
                    tracing::info!(
                        client_id = %client_id_for_log,
                        "DingTalk stream task received shutdown signal"
                    );
                }
            }
        });

        // Retain the handle so shutdown() can reclaim and abort this task.
        // Replacing any prior handle is safe: start() should only be called
        // once per instance, but if it isn't, the old task is orphaned the
        // same way it was before this change — at worst, no regression.
        *self.stream_task.lock().await = Some(handle);

        tracing::info!(
            client_id = %self.config.client_id,
            "DingTalk channel enabled (Stream mode)"
        );

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let (metadata, cache_len) = {
            let targets = self.reply_targets.read().await;
            (targets.peek(&msg.id).cloned(), targets.len())
        };

        let Some(metadata) = metadata else {
            self.cleanup_message_state(msg.id).await;
            tracing::warn!(
                msg_id = %msg.id,
                client_id = %self.config.client_id,
                reply_targets_len = cache_len,
                "No reply metadata found for DingTalk message — response will be dropped \
                 (usual cause: zombie DingTalk channel instance after hot_add; \
                 check for duplicate 'DingTalk channel enabled' logs)"
            );
            return Ok(());
        };

        let conversation_key = if !metadata.conversation_id.is_empty() {
            metadata.conversation_id.as_str()
        } else {
            metadata.sender_staff_id.as_str()
        };

        if stream::is_conversation_stopped(&self.stopped_conversations, conversation_key).await {
            self.cleanup_message_state(msg.id).await;
            tracing::debug!(
                msg_id = %msg.id,
                conversation = %conversation_key,
                "Skipping DingTalk reply for stopped conversation"
            );
            return Ok(());
        }

        // Snapshot just the fields finalize needs — avoid cloning CardState
        // itself (it owns a non-Clone JoinHandle).
        let card_snapshot: Option<(String, bool)> = {
            let states = self.card_states.read().await;
            states
                .get(&msg.id)
                .map(|s| (s.instance_id.clone(), s.fallback_required))
        };

        if let Some((instance_id, fallback_required)) = card_snapshot {
            if self.config.message_type == DingTalkMessageType::Card && !fallback_required {
                match self.get_access_token().await {
                    Ok(token) => {
                        if let Err(e) = card_service::finalize_ai_card(
                            &self.client,
                            &self.config,
                            &token,
                            &instance_id,
                            &response.content,
                        )
                        .await
                        {
                            tracing::warn!(
                                error = %e,
                                msg_id = %msg.id,
                                "Failed to finalize AI card, falling back to markdown"
                            );
                        } else {
                            self.cleanup_message_state(msg.id).await;
                            tracing::info!(msg_id = %msg.id, "Skipping markdown reply — AI card used");
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            msg_id = %msg.id,
                            "Failed to get token for AI card finalize, falling back to markdown"
                        );
                    }
                }
            }

            // Finalize failed or fallback path — drop the card state and
            // continue to markdown reply. cleanup_message_state also drains
            // the tick task.
            self.cleanup_message_state(msg.id).await;
        }

        let robot_code = metadata
            .robot_code
            .as_deref()
            .or(self.config.robot_code.as_deref())
            .unwrap_or_default();

        let is_group = metadata.conversation_type == "2";
        let content = &response.content;

        // Split long responses into chunks (3800 char limit per DingTalk message).
        let chunks = send::split_markdown_chunks(content, send::DEFAULT_CHUNK_LIMIT);
        let (_, base_title) = send::detect_markdown(content);

        // Check whether the session webhook is still valid.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let webhook_valid = metadata
            .session_webhook
            .as_deref()
            .zip(metadata.session_webhook_expired_time)
            .map(|(_, exp)| now_ms < exp)
            .unwrap_or(false);

        for (idx, chunk) in chunks.iter().enumerate() {
            // Title for first chunk is "Reply"; subsequent chunks get (N/M) suffix.
            let chunk_title = if chunks.len() == 1 {
                base_title.clone()
            } else if idx == 0 {
                "Reply".to_string()
            } else {
                format!("Reply ({}/{})", idx + 1, chunks.len())
            };

            if webhook_valid {
                let webhook_url = metadata.session_webhook.as_deref().unwrap_or_default();
                tracing::debug!(
                    chunk = idx + 1,
                    total = chunks.len(),
                    "Sending DingTalk reply via session webhook"
                );
                match send::send_via_webhook(&self.client, webhook_url, &chunk_title, chunk).await {
                    Ok(()) => continue,
                    Err(e) => {
                        // Fall back to Robot API on webhook failure.
                        tracing::debug!(error = %e, "Session webhook failed, falling back to Robot API");
                    }
                }
            }

            // Robot API path (primary when no valid webhook, or webhook fallback).
            let token = self.get_access_token().await?;
            let user_ids_vec = vec![metadata.sender_staff_id.as_str()];
            self.send_markdown(
                &token,
                robot_code,
                if is_group {
                    Some(&metadata.conversation_id)
                } else {
                    None
                },
                if is_group { &[] } else { &user_ids_vec },
                &chunk_title,
                chunk,
            )
            .await?;
        }

        // ── Attachments: upload and send each one as a media message ────────────
        for attachment_path_str in &response.attachments {
            let attachment_path = std::path::Path::new(attachment_path_str);
            let media_type = media::detect_media_type(attachment_path);

            let token = match self.get_access_token().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        error = %e,
                        "DingTalk: failed to get token for attachment upload, skipping"
                    );
                    continue;
                }
            };

            let media_id = match media::upload_media(
                &self.client,
                &token,
                attachment_path,
                media_type,
            )
            .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        error = %e,
                        "DingTalk: attachment upload failed, skipping"
                    );
                    continue;
                }
            };

            // Build the media message body
            let (msg_key, msg_param_value) = match media_type {
                "image" => {
                    let param = serde_json::json!({ "photoURL": format!("@{media_id}") });
                    ("sampleImageMsg", param)
                }
                "voice" => {
                    let param = serde_json::json!({ "mediaId": media_id, "duration": "0" });
                    ("sampleAudioMsg", param)
                }
                "video" => {
                    let param = serde_json::json!({ "videoMediaId": media_id, "videoType": "mp4" });
                    ("sampleVideoMsg", param)
                }
                _ => {
                    // "file" or anything else
                    let filename = attachment_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("attachment");
                    let ext = attachment_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("bin")
                        .to_ascii_lowercase();
                    let param = serde_json::json!({
                        "mediaId": media_id,
                        "fileName": filename,
                        "fileType": ext,
                    });
                    ("sampleFileMsg", param)
                }
            };

            let msg_param_str = match serde_json::to_string(&msg_param_value) {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        error = %e,
                        "DingTalk: failed to serialize media msg param, skipping"
                    );
                    continue;
                }
            };

            let mut body = serde_json::json!({
                "msgKey": msg_key,
                "msgParam": msg_param_str,
                "robotCode": robot_code,
            });

            let media_url = if is_group {
                body["openConversationId"] =
                    serde_json::Value::String(metadata.conversation_id.clone());
                "https://api.dingtalk.com/v1.0/robot/groupMessages/send"
            } else {
                body["userIds"] = serde_json::Value::Array(vec![serde_json::Value::String(
                    metadata.sender_staff_id.clone(),
                )]);
                "https://api.dingtalk.com/v1.0/robot/oToMessages/batchSend"
            };

            match self
                .client
                .post(media_url)
                .header("x-acs-dingtalk-access-token", &token)
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    if let Err(e) = send::ensure_business_success(resp, "media send").await {
                        tracing::debug!(
                            path = %attachment_path_str,
                            error = %e,
                            "DingTalk: attachment send failed, skipping"
                        );
                        continue;
                    }

                    tracing::debug!(
                        path = %attachment_path_str,
                        msg_key,
                        "DingTalk: attachment sent"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        error = %e,
                        "DingTalk: attachment send request failed, skipping"
                    );
                }
            }
        }

        self.cleanup_message_state(msg.id).await;

        tracing::debug!(
            sender = %metadata.sender_nick,
            mode = if is_group { "group" } else { "dm" },
            chunks = chunks.len(),
            attachments = response.attachments.len(),
            "DingTalk reply sent"
        );

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        if !self.card_delivery_enabled() {
            return Ok(());
        }

        // Extract the internal message UUID injected by stream.rs.
        let uuid_str = metadata
            .get("message_id")
            .or_else(|| metadata.get("msg_id"))
            .and_then(|v| v.as_str());
        let Some(uuid_str) = uuid_str else {
            return Ok(());
        };
        let Ok(msg_uuid) = Uuid::parse_str(uuid_str) else {
            return Ok(());
        };
        let message_lock = {
            let mut locks = self.status_locks.lock().await;
            Arc::clone(
                locks
                    .entry(msg_uuid)
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _message_guard = message_lock.lock().await;
        let conversation_id = metadata
            .get("conversation_id")
            .or_else(|| metadata.get("conversationId"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let sender_staff_id = metadata
            .get("sender_staff_id")
            .or_else(|| metadata.get("senderStaffId"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let conversation_key = if !conversation_id.is_empty() {
            conversation_id
        } else {
            sender_staff_id
        };
        let can_activate_card = Self::status_can_activate_card(&status);
        let supports_live_flush = self.status_supports_live_flush(&status);
        let force_live_flush = matches!(
            &status,
            StatusUpdate::Thinking(_)
                | StatusUpdate::ToolStarted { .. }
                | StatusUpdate::ToolCompleted { .. }
        );

        if can_activate_card
            && stream::is_conversation_stopped(&self.stopped_conversations, conversation_key).await
        {
            let state = {
                let mut states = self.card_states.write().await;
                states.remove(&msg_uuid)
            };
            self.reply_targets.write().await.pop(&msg_uuid);
            self.status_locks.lock().await.remove(&msg_uuid);

            if let Some(state) = state {
                if !state.fallback_required && !state.instance_id.is_empty() {
                    let final_content = self
                        .rendered_card_content(&state)
                        .unwrap_or_else(|| state.content_buffer.clone());
                    if let Ok(token) = self.get_access_token().await {
                        if let Err(e) = card_service::finalize_ai_card(
                            &self.client,
                            &self.config,
                            &token,
                            &state.instance_id,
                            &final_content,
                        )
                        .await
                        {
                            tracing::warn!(error = %e, "Failed to finalize stopped AI card");
                        }
                    }
                }
            }

            tracing::debug!(
                msg_id = %msg_uuid,
                conversation = %conversation_key,
                "Skipping DingTalk status update for stopped conversation"
            );
            return Ok(());
        }

        if can_activate_card && !self.ensure_card_ready(msg_uuid).await {
            return Ok(());
        }

        match status {
            StatusUpdate::StreamChunk(chunk) => {
                let mut states = self.card_states.write().await;
                let Some(state) = states.get_mut(&msg_uuid) else {
                    return Ok(());
                };
                if state.fallback_required {
                    return Ok(());
                }
                state.content_buffer.push_str(&chunk);
                state.phase = CardPhase::Inputing;
            }

            StatusUpdate::Status(ref msg) if Self::is_terminal_status_message(msg) => {
                let state = {
                    let mut states = self.card_states.write().await;
                    states.remove(&msg_uuid)
                };

                if let Some(state) = state {
                    if !state.fallback_required && !state.instance_id.is_empty() {
                        let final_content = self
                            .rendered_card_content(&state)
                            .unwrap_or_else(|| state.content_buffer.clone());
                        tracing::info!(
                            instance_id = %state.instance_id,
                            content_len = final_content.len(),
                            "Finalizing DingTalk AI card"
                        );
                        let token = match self.get_access_token().await {
                            Ok(t) => t,
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to get token for card finalize");
                                self.cleanup_message_state(msg_uuid).await;
                                return Ok(());
                            }
                        };

                        if let Err(e) = card_service::finalize_ai_card(
                            &self.client,
                            &self.config,
                            &token,
                            &state.instance_id,
                            &final_content,
                        )
                        .await
                        {
                            tracing::warn!(error = %e, "Failed to finalize AI card");
                        }
                    }
                }

                self.cleanup_message_state(msg_uuid).await;
            }

            StatusUpdate::Thinking(text) => {
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    if state.fallback_required {
                        return Ok(());
                    }
                    // Preserve existing `all`-mode thinking buffer for parity,
                    // but the primary UX signal now comes from
                    // `PhaseChanged(Thinking)` updating state.agent_phase.
                    Self::append_line(&mut state.thinking_buffer, &text);
                    state.agent_phase =
                        crate::channels::dingtalk::types::AgentPhase::Thinking;
                }
            }

            StatusUpdate::PhaseChanged(phase) => {
                use crate::channels::Phase as ChannelPhase;
                use crate::channels::dingtalk::types::AgentPhase;
                let mapped = match phase {
                    ChannelPhase::Thinking => AgentPhase::Thinking,
                    ChannelPhase::UsingTool => AgentPhase::UsingTool,
                    ChannelPhase::Generating => AgentPhase::Generating,
                };
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    if state.fallback_required {
                        return Ok(());
                    }
                    state.agent_phase = mapped;
                    // Clear current_tool when leaving the tool phase.
                    if mapped != AgentPhase::UsingTool {
                        state.current_tool = None;
                    }
                }
            }

            StatusUpdate::ToolStarted {
                ref name,
                ref detail,
                ..
            } => {
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    if state.fallback_required {
                        return Ok(());
                    }
                    state.agent_phase =
                        crate::channels::dingtalk::types::AgentPhase::UsingTool;
                    state.tools_used = state.tools_used.saturating_add(1);
                    // Pre-computed detail from `tool_call_detail` already
                    // runs a light redaction via the StatusUpdate helpers;
                    // we keep it as the base summary, then let the renderer
                    // apply channel_level-aware scrubbing.
                    let fallback_summary = detail.clone().unwrap_or_else(|| name.clone());
                    state.current_tool =
                        Some(crate::channels::dingtalk::types::ToolActivity {
                            name: name.clone(),
                            summary: fallback_summary,
                            started_at: std::time::Instant::now(),
                        });
                }
            }

            StatusUpdate::ToolCompleted {
                success, ref error, ..
            } => {
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    if state.fallback_required {
                        return Ok(());
                    }
                    state.current_tool = None;
                    if !success
                        && let Some(err_msg) = error
                    {
                        // Map to semantic bucket and park it in content_buffer
                        // as a scrubbed single-line footer. Retry behavior is
                        // owned upstream (dispatcher already handles retry
                        // semantics via ToolError classification).
                        let bucket = classify_error_message(err_msg);
                        let bucket_text = bucket_message(bucket);
                        let scrubbed = scrubber::scrub_error_body(bucket_text);
                        Self::append_line(&mut state.content_buffer, scrubbed.as_str());
                    }
                }
            }

            StatusUpdate::ReasoningUpdate {
                ref narrative, ..
            } => {
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    if state.fallback_required {
                        return Ok(());
                    }
                    if state.reasoning_summary_enabled {
                        let scrubbed = scrubber::scrub_reasoning_excerpt(
                            narrative,
                            &state.seen_sensitive,
                        );
                        if !scrubbed.is_empty() {
                            state.reasoning_excerpt =
                                Some(format!("最近思路：{}", scrubbed.as_str()));
                        }
                    }
                }
            }

            _ => {}
        }

        if can_activate_card && supports_live_flush {
            self.flush_card_if_needed(msg_uuid, force_live_flush).await;
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let token = self.get_access_token().await?;
        let robot_code = self.config.robot_code.as_deref().unwrap_or_default();

        self.send_markdown(
            &token,
            robot_code,
            None,
            &[user_id],
            "Notification",
            &response.content,
        )
        .await?;

        tracing::debug!(user_id = %user_id, "DingTalk broadcast sent");
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        // Check if we can get an access token (validates credentials)
        let _ = self.get_access_token().await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        tracing::info!(
            client_id = %self.config.client_id,
            "DingTalk channel shutdown: signaling stream task and awaiting exit"
        );

        // Fire the cooperative stop first. The task's tokio::select! on
        // shutdown_signal.notified() exits its select arm and drops the
        // run_stream_listener future (which in turn closes the WebSocket).
        self.shutdown_signal.notify_waiters();

        // Reclaim the JoinHandle. Await with a short bounded timeout; if
        // the task doesn't exit in time, abort it. This guarantees no
        // zombie WebSocket survives `ChannelManager::hot_add`.
        let handle = self.stream_task.lock().await.take();
        if let Some(handle) = handle {
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(Ok(())) => {
                    tracing::info!(
                        client_id = %self.config.client_id,
                        "DingTalk stream task stopped"
                    );
                }
                Ok(Err(err)) => {
                    // Task panicked or was already cancelled.
                    tracing::warn!(
                        client_id = %self.config.client_id,
                        error = %err,
                        "DingTalk stream task join error on shutdown"
                    );
                }
                Err(_) => {
                    // Cooperative stop ran out of time; we'll have to abort.
                    // Note: handle was consumed by `timeout`; we cannot
                    // abort directly here. The orphaned task will continue
                    // until its own channel closes. Logging at warn so ops
                    // can see it — if this fires regularly, the 5s grace
                    // needs to be tuned.
                    tracing::warn!(
                        client_id = %self.config.client_id,
                        "DingTalk stream task did not stop within grace window; \
                         orphaning it (will self-exit when inject channel closes)"
                    );
                }
            }
        } else {
            tracing::debug!(
                client_id = %self.config.client_id,
                "DingTalk shutdown: no stream task handle (start() not called or already shutdown)"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex, OnceLock};

    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{Method, StatusCode, Uri};
    use axum::response::IntoResponse;
    use axum::routing::any;
    use axum::{Json, Router};
    use secrecy::SecretString;
    use serde_json::{Value, json};

    use super::*;
    use crate::config::{DmPolicy, GroupPolicy};

    #[derive(Clone, Debug)]
    struct RecordedRequest {
        path: String,
        body: Value,
    }

    #[derive(Clone, Debug, Default)]
    struct MockDingTalkBehavior {
        fail_create: bool,
        fail_nonempty_stream: bool,
        fail_finalize_stream: bool,
    }

    #[derive(Clone, Default)]
    struct MockDingTalkState {
        requests: Arc<tokio::sync::Mutex<Vec<RecordedRequest>>>,
        behavior: Arc<tokio::sync::Mutex<MockDingTalkBehavior>>,
        next_card_id: Arc<AtomicUsize>,
    }

    impl MockDingTalkState {
        async fn requests(&self) -> Vec<RecordedRequest> {
            self.requests.lock().await.clone()
        }
    }

    struct ScopedEnvVar {
        key: &'static str,
        original: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            static ENV_MUTEX: OnceLock<StdMutex<()>> = OnceLock::new();
            let guard = ENV_MUTEX
                .get_or_init(|| StdMutex::new(()))
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let original = std::env::var(key).ok();
            // SAFETY: guarded by ENV_MUTEX for test-only process-wide env mutation.
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key,
                original,
                _guard: guard,
            }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            // SAFETY: guarded by ENV_MUTEX for test-only process-wide env mutation.
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    struct MockDingTalkServer {
        state: MockDingTalkState,
        task: tokio::task::JoinHandle<()>,
        _env: ScopedEnvVar,
    }

    impl Drop for MockDingTalkServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn mock_dingtalk_handler(
        State(state): State<MockDingTalkState>,
        method: Method,
        uri: Uri,
        body: Bytes,
    ) -> impl IntoResponse {
        let body_json = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice::<Value>(&body)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()))
        };

        state.requests.lock().await.push(RecordedRequest {
            path: uri.path().to_string(),
            body: body_json.clone(),
        });

        let behavior = state.behavior.lock().await.clone();

        match (method, uri.path()) {
            (Method::POST, "/v1.0/oauth2/accessToken") => (
                StatusCode::OK,
                Json(json!({ "accessToken": "test-token", "expireIn": 7200 })),
            )
                .into_response(),
            (Method::POST, "/v1.0/card/instances/createAndDeliver") => {
                if behavior.fail_create {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "success": false, "message": "create failed" })),
                    )
                        .into_response();
                }

                let id = state.next_card_id.fetch_add(1, Ordering::Relaxed) + 1;
                (
                    StatusCode::OK,
                    Json(json!({
                        "success": true,
                        "result": { "outTrackId": format!("card-{id}") }
                    })),
                )
                    .into_response()
            }
            (Method::PUT, "/v1.0/card/streaming") => {
                let is_finalize = body_json
                    .get("isFinalize")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let content = body_json
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if (is_finalize && behavior.fail_finalize_stream)
                    || (!is_finalize && !content.is_empty() && behavior.fail_nonempty_stream)
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "success": false, "message": "stream failed" })),
                    )
                        .into_response();
                }

                (StatusCode::OK, Json(json!({ "success": true }))).into_response()
            }
            (Method::PUT, "/v1.0/card/instances") => {
                (StatusCode::OK, Json(json!({ "success": true }))).into_response()
            }
            (Method::POST, "/v1.0/robot/oToMessages/batchSend")
            | (Method::POST, "/v1.0/robot/groupMessages/send") => {
                (StatusCode::OK, Json(json!({ "success": true }))).into_response()
            }
            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({ "success": false, "path": uri.path() })),
            )
                .into_response(),
        }
    }

    async fn spawn_mock_dingtalk_server(behavior: MockDingTalkBehavior) -> MockDingTalkServer {
        let state = MockDingTalkState {
            requests: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            behavior: Arc::new(tokio::sync::Mutex::new(behavior)),
            next_card_id: Arc::new(AtomicUsize::new(0)),
        };

        let app = Router::new()
            .route("/{*path}", any(mock_dingtalk_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake dingtalk");
        let addr = listener.local_addr().expect("fake dingtalk addr");
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let env = ScopedEnvVar::set(
            "IRONCLAW_TEST_DINGTALK_API_BASE_URL",
            &format!("http://{addr}"),
        );

        MockDingTalkServer {
            state,
            task,
            _env: env,
        }
    }

    fn test_config(card_stream_mode: CardStreamMode) -> DingTalkConfig {
        DingTalkConfig {
            enabled: true,
            client_id: "test-client".to_string(),
            client_secret: SecretString::from("test-secret"),
            robot_code: Some("robot-code".to_string()),
            message_type: DingTalkMessageType::Card,
            card_template_id: Some("tpl-123".to_string()),
            card_template_key: "content".to_string(),
            card_stream_mode,
            card_stream_interval_ms: 1000,
            ack_reaction: None,
            require_mention: false,
            dm_policy: DmPolicy::Open,
            group_policy: GroupPolicy::Open,
            allow_from: vec![],
            group_allow_from: vec![],
            group_session_scope: Default::default(),
            display_name_resolution: Default::default(),
            max_reconnect_cycles: 10,
            reconnect_deadline_ms: 50_000,
            additional_accounts: vec![],
            status_tick_ms: 2000,
            slow_threshold_secs: (15, 60),
            reasoning_summary_enabled: false,
            max_active_cards: 1000,
        }
    }

    fn test_message() -> IncomingMessage {
        let mut message = IncomingMessage::new("dingtalk", "staff-1", "hello")
            .with_sender_id("staff-1")
            .with_user_name("Alice");
        let msg_id = Uuid::new_v4();
        message.id = msg_id;
        message.metadata = json!({
            "message_id": msg_id.to_string(),
            "conversationId": "conv-1",
            "conversationType": "1",
            "senderStaffId": "staff-1",
            "senderNick": "Alice",
            "msgId": "dt-msg-1",
            "robotCode": "robot-code"
        });
        message
    }

    fn streaming_requests(requests: &[RecordedRequest]) -> Vec<&RecordedRequest> {
        requests
            .iter()
            .filter(|req| req.path == "/v1.0/card/streaming")
            .collect()
    }

    #[tokio::test]
    async fn thinking_creates_card_before_first_chunk() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            requests
                .iter()
                .any(|req| req.path == "/v1.0/card/instances/createAndDeliver"),
            "expected createAndDeliver request, got: {requests:?}"
        );

        let streams = streaming_requests(&requests);
        assert_eq!(streams.len(), 1, "expected only activation stream");
        assert_eq!(streams[0].body["content"], json!(""));
        assert_eq!(streams[0].body["isFinalize"], json!(false));
    }

    #[tokio::test]
    async fn first_stream_chunk_flushes_immediately_after_activation() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::StreamChunk("Hello immediately".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        let streams = streaming_requests(&requests);
        assert_eq!(
            streams.len(),
            2,
            "expected activation + first content stream"
        );
        // Anti-silence UX wraps every PUT body with the phase status line
        // on line 1; the streamed chunk appears as the body below the
        // blank separator. See Unit 5 (render).
        let body = streams[1].body["content"].as_str().unwrap();
        assert!(
            body.ends_with("Hello immediately"),
            "expected body to include streamed chunk, got: {body}"
        );
        assert!(
            body.starts_with("🧠"),
            "expected phase-prefixed status line, got: {body}"
        );
        assert_eq!(streams[1].body["isFinalize"], json!(false));
    }

    #[tokio::test]
    async fn all_mode_flushes_thinking_and_tool_progress() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::All)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "search".to_string(),
                    detail: None,
                    call_id: None,
                },
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        let streams = streaming_requests(&requests);
        assert_eq!(
            streams.len(),
            3,
            "expected activation + thinking + tool flush"
        );
        // Anti-silence render prefixes every content PUT with the phase
        // status line. In `all` mode the thinking buffer remains part of
        // the body (below the status line), and ToolStarted promotes the
        // status line to the 🔧 UsingTool phase.
        let thinking_body = streams[1].body["content"].as_str().unwrap();
        assert!(
            thinking_body.contains("Processing..."),
            "thinking body should include the thinking chunk, got: {thinking_body}"
        );
        let tool_content = streams[2].body["content"]
            .as_str()
            .expect("tool stream content should be string");
        assert!(
            tool_content.contains("调用工具"),
            "expected 🔧 调用工具 in tool status line, got: {tool_content}"
        );
        assert!(
            tool_content.contains("Processing..."),
            "thinking buffer should persist into tool-phase render in `all` mode"
        );
    }

    #[tokio::test]
    async fn create_failure_falls_back_to_markdown_reply() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior {
            fail_create: true,
            ..Default::default()
        })
        .await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .respond(&message, OutgoingResponse::text("final fallback"))
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            requests.iter().any(|req| {
                req.path == "/v1.0/robot/oToMessages/batchSend"
                    && req.body["msgParam"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("final fallback")
            }),
            "expected markdown fallback request, got: {requests:?}"
        );
    }

    #[tokio::test]
    async fn stream_failure_falls_back_to_markdown_reply() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior {
            fail_nonempty_stream: true,
            ..Default::default()
        })
        .await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::StreamChunk("partial".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .respond(
                &message,
                OutgoingResponse::text("final after stream failure"),
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            requests.iter().any(|req| {
                req.path == "/v1.0/robot/oToMessages/batchSend"
                    && req.body["msgParam"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("final after stream failure")
            }),
            "expected markdown fallback after stream failure, got: {requests:?}"
        );
        assert!(
            !streaming_requests(&requests)
                .iter()
                .any(|req| req.body["isFinalize"] == json!(true)),
            "finalize should not run after stream fallback: {requests:?}"
        );
    }

    #[tokio::test]
    async fn finalize_failure_falls_back_to_markdown_reply() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior {
            fail_finalize_stream: true,
            ..Default::default()
        })
        .await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::StreamChunk("partial".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .respond(
                &message,
                OutgoingResponse::text("final after finalize failure"),
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            streaming_requests(&requests)
                .iter()
                .any(|req| req.body["isFinalize"] == json!(true)),
            "expected finalize streaming attempt, got: {requests:?}"
        );
        assert!(
            requests.iter().any(|req| {
                req.path == "/v1.0/robot/oToMessages/batchSend"
                    && req.body["msgParam"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("final after finalize failure")
            }),
            "expected markdown fallback after finalize failure, got: {requests:?}"
        );
    }

    #[tokio::test]
    async fn phase_changed_promotes_status_line_icon() {
        // All mode gives us live flushes on Thinking/Tool events so we can
        // observe the status-line transition without waiting on the tick.
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::All)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        // Thinking phase activates the card.
        channel
            .send_status(
                StatusUpdate::PhaseChanged(crate::channels::Phase::Thinking),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::Thinking("bootstrap".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();

        // Transition to UsingTool with a tool start.
        channel
            .send_status(
                StatusUpdate::PhaseChanged(crate::channels::Phase::UsingTool),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "web_search".to_string(),
                    detail: Some("querying \"ZStack\"".to_string()),
                    call_id: None,
                },
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        let streams = streaming_requests(&requests);

        // Find the last stream with non-empty content — the tool-phase render.
        let tool_phase_body = streams
            .iter()
            .rev()
            .find_map(|s| s.body["content"].as_str())
            .expect("expected at least one stream PUT with content");

        assert!(
            tool_phase_body.contains("🔧 调用工具")
                || tool_phase_body.contains("调用工具"),
            "expected 🔧 调用工具 icon on tool-phase status line, got: {tool_phase_body}"
        );
    }

    #[tokio::test]
    async fn supersede_previous_card_on_new_message_same_user() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();

        // First message seeds reply metadata and activates a card.
        let msg1 = test_message();
        channel.seed_reply_target_for_test(&msg1).await.unwrap();
        channel
            .send_status(
                StatusUpdate::Thinking("first".to_string()),
                &msg1.metadata,
            )
            .await
            .unwrap();

        // Second message from SAME conversation+user — should supersede.
        let mut msg2 = test_message();
        msg2.id = uuid::Uuid::new_v4();
        // Update metadata to carry the new msg_id.
        if let Some(obj) = msg2.metadata.as_object_mut() {
            obj.insert(
                "message_id".to_string(),
                serde_json::Value::String(msg2.id.to_string()),
            );
            obj.insert(
                "msg_id".to_string(),
                serde_json::Value::String(msg2.id.to_string()),
            );
        }
        channel.seed_reply_target_for_test(&msg2).await.unwrap();
        channel
            .send_status(
                StatusUpdate::Thinking("second".to_string()),
                &msg2.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        let streams = streaming_requests(&requests);

        // We should see at least one finalize PUT (supersede of msg1) and
        // at least one non-finalize PUT from msg2's activation.
        let finalize_count = streams
            .iter()
            .filter(|s| s.body["isFinalize"] == json!(true))
            .count();
        assert!(
            finalize_count >= 1,
            "expected at least one finalize (supersede) PUT, got streams: {streams:#?}"
        );

        // A finalize body should carry the supersede terminal marker.
        let found_supersede = streams.iter().any(|s| {
            s.body["isFinalize"] == json!(true)
                && s.body["content"]
                    .as_str()
                    .is_some_and(|c| c.contains("被新问题替代") || c.contains("⏹"))
        });
        assert!(
            found_supersede,
            "expected supersede terminal marker in finalize PUT, got: {streams:#?}"
        );
    }

    #[tokio::test]
    async fn markdown_mode_skips_card_status_updates() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let mut config = test_config(CardStreamMode::Answer);
        config.message_type = DingTalkMessageType::Markdown;
        let channel = DingTalkChannel::new(config).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            !requests
                .iter()
                .any(|req| req.path == "/v1.0/card/instances/createAndDeliver"),
            "markdown mode should not create cards: {requests:?}"
        );
    }
}
