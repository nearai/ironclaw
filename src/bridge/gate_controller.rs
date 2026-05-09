//! Inline gate-await bridge controller.
//!
//! Implements [`ironclaw_engine::GateController`] for the bridge layer.
//! When the engine hits an `Approval` gate inside a live execution
//! (Tier 0 batch or Tier 1 CodeAct VM), it calls
//! [`BridgeGateController::pause`] which:
//!
//! 1. Builds and persists a [`PendingGate`] (existing UI machinery
//!    discovers the prompt through the same store / SSE / channel
//!    flow as before).
//! 2. Registers a [`oneshot::Sender`] keyed by `request_id` in a
//!    process-wide registry shared with the resolve endpoint.
//! 3. Awaits the receiver. The future stays parked here, holding the
//!    engine's call stack open, until the user resolves the gate.
//!
//! On the resolve side, [`GateResolutions::try_deliver`] looks up the
//! sender by `request_id` and hands the [`GateResolution`] back into
//! the suspended engine. The engine continues from the exact
//! suspension point — no re-entry, no replay, no double-execution of
//! prior side effects in the same step.
//!
//! ## Single instance, per-thread context
//!
//! The controller is a single shared instance (held by `EngineState`,
//! attached to `ThreadManager` at boot). Per-execution data
//! (conversation id, channel metadata, original message, scope thread
//! id) lives in a `HashMap` keyed by `(user_id, thread_id)`. The
//! bridge populates an entry before invoking
//! `ConversationManager::handle_user_message`; if a gate fires during
//! that execution, the controller looks up the entry to construct the
//! `PendingGate`. Stale entries (from a turn that completed without
//! gating) are removed by the bridge after the call.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_common::AppEvent;
use ironclaw_common::ExternalThreadId;
use ironclaw_engine::{
    ConversationId, GateController, GatePauseRequest, GateResolution, ResumeKind, ThreadId,
};
use serde_json::Value as JsonValue;
use tokio::sync::{Mutex, oneshot};
use tracing::debug;
use uuid::Uuid;

use crate::auth::extension::AuthManager;
use crate::channels::ChannelManager;
use crate::channels::StatusUpdate;
use crate::channels::web::sse::SseManager;
use crate::extensions::ExtensionManager;
use crate::gate::pending::PendingGate;
use crate::gate::store::PendingGateStore;
use crate::tools::ToolRegistry;

/// Per-execution data the controller needs to build a `PendingGate`.
/// Populated by the bridge before invoking the engine for a turn,
/// removed after.
#[derive(Debug, Clone)]
pub struct PerExecutionContext {
    pub conversation_id: ConversationId,
    pub source_channel: String,
    pub scope_thread_id: Option<ExternalThreadId>,
    pub channel_metadata: JsonValue,
    pub original_message: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct ExecutionKey {
    user_id: String,
    thread_id: ThreadId,
}

/// Pre-execution registry key. Keyed by `(user_id, conversation_id)`
/// so two concurrent conversations for the same user (e.g. two browser
/// tabs) don't clobber each other's pre-execution slot before each
/// turn has been promoted to its own `(user_id, thread_id)` entry.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct PreExecKey {
    user_id: String,
    conversation_id: ConversationId,
}

/// Process-wide registry of in-flight gate resolution channels.
///
/// One entry per pending in-flight gate. Inserts come from
/// [`BridgeGateController::pause`]; removes come from
/// [`GateResolutions::try_deliver`] (the resolve endpoint).
///
/// Authentication gates additionally register their `request_id`
/// under the `(user_id, credential_name)` pair they are waiting on,
/// so the OAuth callback path can wake the parked VM by credential
/// name without having to know the engine-internal request_id —
/// scoped per user so a credential write under one account never
/// wakes a parked gate from a different account that happens to share
/// the same credential name.
///
/// Stranded entries from a prior crash do not exist — restarting the
/// process drops the registry. Stale `PendingGate` rows surviving
/// restart are cleaned up by the startup sweep in `router.rs`.
#[derive(Default)]
pub struct GateResolutions {
    inner: Mutex<HashMap<Uuid, oneshot::Sender<GateResolution>>>,
    /// Secondary index: `(user_id, credential_name)` → request_ids
    /// parked on it. Used by [`Self::deliver_for_credential`] so the
    /// OAuth-callback path can wake a Tier 0/Tier 1 inline-await for
    /// the credential that was just written. The user_id component
    /// keeps multi-tenant deployments from cross-waking — only the
    /// owning user's parked gates fire.
    by_credential: Mutex<HashMap<(String, String), HashSet<Uuid>>>,
}

impl GateResolutions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deliver a resolution to the suspended caller. Returns `true` if
    /// a sender was registered for `request_id` (engine was waiting),
    /// `false` if not (no live VM — fall through to legacy re-entry).
    pub async fn try_deliver(&self, request_id: Uuid, resolution: GateResolution) -> bool {
        let sender = self.inner.lock().await.remove(&request_id);
        // Best-effort: drop any credential-index entries that point at
        // this request_id (we don't know which credential without
        // tracking the reverse mapping; keep the index lazy by sweeping
        // sets that contain the id).
        {
            let mut idx = self.by_credential.lock().await;
            idx.retain(|_, set| {
                set.remove(&request_id);
                !set.is_empty()
            });
        }
        match sender {
            Some(tx) => tx.send(resolution).is_ok(),
            None => false,
        }
    }

    /// Deliver `Approved` to every parked Authentication gate that was
    /// waiting on `(user_id, credential_name)`. Returns the count of
    /// waiters woken. Used by the OAuth-callback path: when a
    /// credential is written, every paused tool call (Tier 0 or
    /// Tier 1, foreground or mission child thread) belonging to
    /// `user_id` that was blocked on that credential can resume
    /// inline and retry the action against the now-present secret.
    /// Other users' parked gates on the same credential name are
    /// left untouched.
    pub async fn deliver_for_credential(&self, user_id: &str, credential_name: &str) -> usize {
        let request_ids: Vec<Uuid> = {
            let mut idx = self.by_credential.lock().await;
            idx.remove(&(user_id.to_string(), credential_name.to_string()))
                .map(|set| set.into_iter().collect())
                .unwrap_or_default()
        };
        let mut delivered = 0;
        for request_id in request_ids {
            if self
                .try_deliver(request_id, GateResolution::Approved { always: false })
                .await
            {
                delivered += 1;
            }
        }
        delivered
    }

    async fn register(&self, request_id: Uuid, sender: oneshot::Sender<GateResolution>) {
        self.inner.lock().await.insert(request_id, sender);
    }

    /// Register `request_id` against `(user_id, credential_name)` so
    /// an OAuth completion can wake it by credential name later,
    /// scoped to the owning user.
    async fn register_credential(
        &self,
        user_id: String,
        credential_name: String,
        request_id: Uuid,
    ) {
        self.by_credential
            .lock()
            .await
            .entry((user_id, credential_name))
            .or_default()
            .insert(request_id);
    }

    async fn forget(&self, request_id: Uuid) {
        self.inner.lock().await.remove(&request_id);
        let mut idx = self.by_credential.lock().await;
        idx.retain(|_, set| {
            set.remove(&request_id);
            !set.is_empty()
        });
    }
}

/// Single shared controller. Threaded through every
/// `ThreadExecutionContext` the engine builds for a live execution.
pub struct BridgeGateController {
    pending_gates: Arc<PendingGateStore>,
    sse: Option<Arc<SseManager>>,
    tools: Arc<ToolRegistry>,
    auth_manager: Option<Arc<AuthManager>>,
    extension_manager: Option<Arc<ExtensionManager>>,
    channels: Arc<ChannelManager>,
    resolutions: Arc<GateResolutions>,
    /// Per-(user, thread) registry. Populated once the bridge knows
    /// which thread the engine spawned for a turn. The lookup here
    /// wins when both this map and `pre_execution` carry an entry —
    /// it's the more specific key.
    per_execution: Mutex<HashMap<ExecutionKey, PerExecutionContext>>,
    /// Pre-execution registry, populated *before* `handle_user_message`
    /// returns the thread_id. Closes the race where a fast tool gate
    /// reaches `pause()` before the bridge has had a chance to register
    /// the (user, thread)-keyed entry. Keyed by `(user_id,
    /// conversation_id)` so concurrent conversations for the same user
    /// (e.g. two browser tabs) don't clobber each other — each turn's
    /// `pause()` matches its own conversation's slot via
    /// `GatePauseRequest::conversation_id`.
    pre_execution: Mutex<HashMap<PreExecKey, PerExecutionContext>>,
    /// Per-(user, thread) serialization lock for `pause()`. Holding
    /// this across the `PendingGateStore::insert` + select-await window
    /// guarantees only one inline gate per `(user, thread)` is in
    /// flight at a time. Without it, a parallel batch where two tool
    /// calls both gate concurrently would have the second insert hit
    /// the (user, thread) uniqueness check and silently surface as
    /// `GateResolution::Cancelled`. With it, the second `pause()`
    /// queues until the first resolves.
    gate_locks: Mutex<HashMap<ExecutionKey, Arc<Mutex<()>>>>,
    /// Per-`ThreadId` registry of in-flight pause request_ids.
    /// `pause()` adds its `request_id` here on entry and removes it on
    /// exit. `cancel_thread()` walks this set and delivers
    /// `GateResolution::Cancelled` to each — wiring `stop_thread()`
    /// through to the parked future so a stop request promptly wakes
    /// the engine task instead of waiting on the 30-minute gate
    /// expiry.
    ///
    /// A `HashSet<Uuid>` (rather than `Option<Uuid>`) is correct even
    /// though the current `gate_locks` serializes one pause per
    /// `(user, thread)`: a future change that loosens that
    /// serialization (e.g. per-action gates instead of per-thread)
    /// would otherwise drop pending request_ids on the floor.
    active_pauses: Mutex<HashMap<ThreadId, HashSet<Uuid>>>,
}

impl BridgeGateController {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pending_gates: Arc<PendingGateStore>,
        sse: Option<Arc<SseManager>>,
        tools: Arc<ToolRegistry>,
        auth_manager: Option<Arc<AuthManager>>,
        extension_manager: Option<Arc<ExtensionManager>>,
        channels: Arc<ChannelManager>,
        resolutions: Arc<GateResolutions>,
    ) -> Self {
        Self {
            pending_gates,
            sse,
            tools,
            auth_manager,
            extension_manager,
            channels,
            resolutions,
            per_execution: Mutex::new(HashMap::new()),
            pre_execution: Mutex::new(HashMap::new()),
            gate_locks: Mutex::new(HashMap::new()),
            active_pauses: Mutex::new(HashMap::new()),
        }
    }

    async fn track_active_pause(&self, thread_id: ThreadId, request_id: Uuid) {
        self.active_pauses
            .lock()
            .await
            .entry(thread_id)
            .or_default()
            .insert(request_id);
    }

    async fn untrack_active_pause(&self, thread_id: ThreadId, request_id: Uuid) {
        let mut map = self.active_pauses.lock().await;
        if let Some(set) = map.get_mut(&thread_id) {
            set.remove(&request_id);
            if set.is_empty() {
                map.remove(&thread_id);
            }
        }
    }

    /// Look up (or create) the per-(user, thread) gate-serialization
    /// lock. The returned Arc is cloned out so callers can drop the
    /// outer registry lock before contending on the inner lock.
    async fn gate_lock_for(&self, key: &ExecutionKey) -> Arc<Mutex<()>> {
        let mut map = self.gate_locks.lock().await;
        map.entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Bind per-execution data for `(user_id, conversation_id)` BEFORE
    /// the engine spawns the thread. Closes the race window where a
    /// fast tool gate reaches `pause()` before the (user, thread)-keyed
    /// entry has been written.
    ///
    /// Keying by `conversation_id` (rather than `user_id` alone) keeps
    /// concurrent conversations for the same user — multiple browser
    /// tabs, background missions firing alongside a foreground turn —
    /// from clobbering each other's slot. Each turn's `pause()` matches
    /// its own conversation via `GatePauseRequest::conversation_id`.
    pub async fn set_pre_execution_context(
        &self,
        user_id: String,
        conversation_id: ConversationId,
        context: PerExecutionContext,
    ) {
        self.pre_execution.lock().await.insert(
            PreExecKey {
                user_id,
                conversation_id,
            },
            context,
        );
    }

    /// Bind per-execution data for `(user_id, thread_id)` once the
    /// engine has allocated a thread_id. Call after
    /// [`Self::set_pre_execution_context`]; supersedes the
    /// (user, conversation_id)-keyed entry for subsequent lookups.
    pub async fn set_execution_context(
        &self,
        user_id: String,
        thread_id: ThreadId,
        context: PerExecutionContext,
    ) {
        let conv_id = context.conversation_id;
        self.per_execution.lock().await.insert(
            ExecutionKey {
                user_id: user_id.clone(),
                thread_id,
            },
            context,
        );
        // Remove the (user, conversation)-keyed pre-execution entry —
        // the (user, thread)-keyed entry is now the source of truth.
        self.pre_execution.lock().await.remove(&PreExecKey {
            user_id,
            conversation_id: conv_id,
        });
    }

    /// Drop the pre-execution `(user, conversation)`-keyed entry
    /// without touching any `(user, thread)`-keyed entry. Used on the
    /// bridge error path when `handle_user_message` failed before
    /// allocating a thread_id — without this the slot would leak and
    /// could mis-route the next gate prompt for the same conversation.
    pub async fn clear_pre_execution_context(
        &self,
        user_id: &str,
        conversation_id: ConversationId,
    ) {
        self.pre_execution.lock().await.remove(&PreExecKey {
            user_id: user_id.to_string(),
            conversation_id,
        });
    }

    /// Drop per-execution data. Idempotent. `conversation_id` is the
    /// originating conversation for this turn so any leftover
    /// pre-execution slot (e.g. when the bridge bailed before
    /// promotion) gets cleared too.
    pub async fn clear_execution_context(
        &self,
        user_id: &str,
        thread_id: ThreadId,
        conversation_id: ConversationId,
    ) {
        let key = ExecutionKey {
            user_id: user_id.to_string(),
            thread_id,
        };
        self.per_execution.lock().await.remove(&key);
        // Defensive: clear any leftover pre-execution entry too. In
        // the happy path `set_execution_context` already removed it,
        // but if the bridge bailed before that promotion (engine spawn
        // failed) the entry would otherwise leak.
        self.pre_execution.lock().await.remove(&PreExecKey {
            user_id: user_id.to_string(),
            conversation_id,
        });
        // Drop the per-(user, thread) gate-serialization lock entry.
        // By the time the bridge clears execution context, all `pause`
        // futures for this thread have resolved, so the inner lock is
        // idle and removing the registry entry simply bounds the map.
        self.gate_locks.lock().await.remove(&key);
    }

    /// Forward a resolution into the inline-await registry. Returns
    /// `true` if the engine was actively awaiting it.
    pub async fn try_deliver(&self, request_id: Uuid, resolution: GateResolution) -> bool {
        self.resolutions.try_deliver(request_id, resolution).await
    }

    async fn lookup_per_execution(
        &self,
        user_id: &str,
        thread_id: ThreadId,
        conversation_id: Option<ConversationId>,
    ) -> Option<PerExecutionContext> {
        // Most specific match first: (user, thread). Falls back to
        // the (user, conversation)-keyed pre-execution entry so a
        // gate firing before `set_execution_context` lands still
        // finds its context. The fallback requires the request to
        // carry `conversation_id`; gates from threads with no
        // originating conversation (background missions) only match
        // via the (user, thread) entry.
        if let Some(ctx) = self.per_execution.lock().await.get(&ExecutionKey {
            user_id: user_id.to_string(),
            thread_id,
        }) {
            return Some(ctx.clone());
        }
        if let Some(conv_id) = conversation_id {
            return self
                .pre_execution
                .lock()
                .await
                .get(&PreExecKey {
                    user_id: user_id.to_string(),
                    conversation_id: conv_id,
                })
                .cloned();
        }
        None
    }

    async fn build_pending_gate(
        &self,
        request_id: Uuid,
        per_exec: &PerExecutionContext,
        user_id: &str,
        thread_id: ThreadId,
        req: &GatePauseRequest,
    ) -> PendingGate {
        let display_parameters = match self.tools.get(&req.action_name).await {
            Some(tool) => Some(crate::tools::redact_params(
                &req.parameters,
                tool.sensitive_params(),
            )),
            None => Some(req.parameters.clone()),
        };

        PendingGate {
            request_id,
            gate_name: req.gate_name.clone(),
            user_id: user_id.to_string(),
            thread_id,
            scope_thread_id: per_exec.scope_thread_id.clone(),
            conversation_id: per_exec.conversation_id,
            source_channel: per_exec.source_channel.clone(),
            action_name: req.action_name.clone(),
            call_id: req.call_id.clone(),
            parameters: req.parameters.clone(),
            display_parameters,
            description: format!(
                "Tool '{}' requires {} (gate: {})",
                req.action_name,
                req.resume_kind.kind_name(),
                req.gate_name
            ),
            resume_kind: req.resume_kind.clone(),
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
            original_message: per_exec.original_message.clone(),
            resume_output: None,
            paused_lease: None,
            approval_already_granted: false,
        }
    }

    async fn emit_gate_prompt(&self, pending: &PendingGate, channel_metadata: &JsonValue) {
        let extension_name = crate::bridge::router::resolve_auth_gate_extension_name(
            self.auth_manager.as_deref(),
            self.extension_manager.as_deref(),
            self.tools.as_ref(),
            pending,
        )
        .await;

        let display_parameters = crate::bridge::router::gate_display_parameters(pending);

        if let Some(ref sse) = self.sse {
            sse.broadcast_for_user(
                &pending.user_id,
                AppEvent::GateRequired {
                    request_id: pending.request_id.to_string(),
                    gate_name: pending.gate_name.clone(),
                    tool_name: pending.action_name.clone(),
                    description: pending.description.clone(),
                    parameters: serde_json::to_string_pretty(&display_parameters)
                        .unwrap_or_else(|_| display_parameters.to_string()),
                    extension_name: extension_name.clone(),
                    resume_kind: serde_json::to_value(&pending.resume_kind).unwrap_or_default(),
                    thread_id: Some(pending.effective_wire_thread_id()),
                },
            ); // projection-exempt: bridge dispatcher, inline-await gate prompt for live VM waiting on user input
        }

        if let ResumeKind::Approval { allow_always } = &pending.resume_kind {
            let _ = self
                .channels
                .send_status(
                    &pending.source_channel,
                    StatusUpdate::ApprovalNeeded {
                        request_id: pending.request_id.to_string(),
                        tool_name: pending.action_name.clone(),
                        description: pending.description.clone(),
                        parameters: display_parameters,
                        allow_always: *allow_always,
                    },
                    channel_metadata,
                )
                .await;
        }
    }
}

#[async_trait]
impl GateController for BridgeGateController {
    async fn pause(&self, request: GatePauseRequest) -> GateResolution {
        // Inline gate-await handles Approval and Authentication.
        // External resume kinds keep the legacy
        // `ThreadOutcome::GatePaused` re-entry path because their
        // resolution installs callback-payload state that can't be
        // handed back to the suspended call without unwinding. Surface
        // External as Cancelled so the call returns a clean error
        // instead of hanging.
        if matches!(request.resume_kind, ResumeKind::External { .. }) {
            debug!(
                kind = %request.resume_kind.kind_name(),
                "BridgeGateController: External resume kind reached inline await; cancelling",
            );
            return GateResolution::Cancelled;
        }

        let Some(per_exec) = self
            .lookup_per_execution(&request.user_id, request.thread_id, request.conversation_id)
            .await
        else {
            // No per-execution context registered. This shouldn't happen
            // when invoked through `handle_with_engine`, which always
            // populates it before invoking the engine. Mission /
            // background threads also reach here today; they fall
            // through to the legacy `ThreadOutcome::GatePaused` unwind
            // path so `process_mission_outcome_and_notify` (#3133
            // half-1) transitions the mission to Paused. The half-2
            // mission auto-resume path (`resume_paused_for_credential`)
            // resumes the mission after OAuth completes. Cancelling
            // here is the right wire for that flow — it produces the
            // legacy unwind that the mission state machine consumes.
            debug!(
                user = %request.user_id,
                thread = %request.thread_id,
                kind = %request.resume_kind.kind_name(),
                "BridgeGateController: no per-execution context — cancelling (mission/background path)"
            );
            return GateResolution::Cancelled;
        };

        // Serialize concurrent inline gates per (user, thread). A
        // parallel batch where two tool calls both gate would otherwise
        // race on `PendingGateStore::insert` — the first wins, the
        // second hits the (user, thread) uniqueness check and silently
        // becomes `Cancelled` without ever prompting the user. Holding
        // this lock across insert + select-await queues subsequent
        // gates behind the current one so each gets its own prompt.
        //
        // TODO(#3157 follow-up — design-doc item): bound live inline
        // gate awaits per user / globally with a typed semaphore so an
        // authenticated user opening many threads each with an
        // unresolved approval gate cannot accumulate parked engine
        // tasks/pending rows past a budget. The current implicit bound
        // is one pending gate per (user, thread) × the existing
        // thread-creation budget × the 30-min expiry; that is enough
        // to ship the first inline-await slice but not enough as a
        // long-term DoS guard. Track in a separate issue once the
        // semaphore design (cap UX, fairness, rejection error shape)
        // is settled rather than hand-rolling it inside this
        // controller.
        let exec_key = ExecutionKey {
            user_id: request.user_id.clone(),
            thread_id: request.thread_id,
        };
        let gate_lock = self.gate_lock_for(&exec_key).await;
        let _gate_guard = gate_lock.lock().await;

        let request_id = Uuid::new_v4();
        let pending = self
            .build_pending_gate(
                request_id,
                &per_exec,
                &request.user_id,
                request.thread_id,
                &request,
            )
            .await;

        if let Err(e) = self.pending_gates.insert(pending.clone()).await {
            // With the per-(user, thread) gate lock held above, a
            // legitimate concurrent collision can't happen. An insert
            // failure here means a stale row from a prior turn hadn't
            // been cleaned up. Surface as cancel.
            debug!(
                user = %request.user_id,
                thread = %request.thread_id,
                error = %e,
                "BridgeGateController: pending_gates.insert rejected; treating as cancelled",
            );
            return GateResolution::Cancelled;
        }

        let (tx, rx) = oneshot::channel();
        self.resolutions.register(request_id, tx).await;
        // For Authentication gates, also index this request_id by
        // the credential name we're waiting on so the OAuth callback
        // path can wake us by credential without having to know the
        // request_id. Forget on exit cleans the index either way.
        if let ResumeKind::Authentication {
            ref credential_name,
            ..
        } = request.resume_kind
        {
            self.resolutions
                .register_credential(
                    request.user_id.clone(),
                    credential_name.as_str().to_string(),
                    request_id,
                )
                .await;
        }

        // Track this in-flight pause so `cancel_thread()` can wake it
        // promptly on `ThreadManager::stop_thread()`. Without this,
        // a stop request against a thread parked here would have to
        // wait for the user (or the 30-min expiry) before the engine
        // task observed the stop signal.
        self.track_active_pause(request.thread_id, request_id).await;

        self.emit_gate_prompt(&pending, &per_exec.channel_metadata)
            .await;

        // Bound the await on `pending.expires_at`. Without this, a user
        // who ignores the prompt past expiry strands the engine: the
        // pending DB row expires, but the oneshot stays open and the
        // VM keeps running until something else (process restart,
        // join_thread timeout) tears it down. Race the receiver against
        // a sleep; whichever resolves first wins.
        let expires_at = pending.expires_at;
        let now = chrono::Utc::now();
        let timeout_dur = (expires_at - now)
            .to_std()
            .unwrap_or(std::time::Duration::ZERO);
        let pending_key = pending.key();
        let resolution = tokio::select! {
            biased;
            received = rx => match received {
                Ok(resolution) => resolution,
                Err(_) => {
                    // Sender dropped — process shutting down or registry
                    // cleared. Discard the pending row so the UI doesn't
                    // keep showing a stranded prompt and a future
                    // (user, thread) gate isn't blocked by the
                    // duplicate-insert guard. Same cleanup as the
                    // expiry branch below.
                    self.resolutions.forget(request_id).await;
                    let _ = self.pending_gates.discard(&pending_key).await;
                    GateResolution::Cancelled
                }
            },
            _ = tokio::time::sleep(timeout_dur) => {
                // Expiry hit before the user resolved. Drop the
                // registry entry and the pending row so a late
                // resolve_gate call can't double-deliver, and surface
                // as Cancelled to wake the VM.
                self.resolutions.forget(request_id).await;
                let _ = self.pending_gates.discard(&pending_key).await;
                debug!(
                    user = %request.user_id,
                    thread = %request.thread_id,
                    request_id = %request_id,
                    "BridgeGateController: pause expired before resolution; cancelling",
                );
                GateResolution::Cancelled
            }
        };
        // Always untrack on exit. Idempotent — `cancel_thread` may
        // have already removed our entry while delivering the
        // cancellation that woke us; re-removing is a no-op.
        self.untrack_active_pause(request.thread_id, request_id)
            .await;
        resolution
    }

    async fn cancel_thread(&self, thread_id: ThreadId) {
        // Snapshot the in-flight request_ids and pending keys, then
        // release the lock before delivering. Holding `active_pauses`
        // while calling `try_deliver` (which takes its own lock) and
        // `pending_gates.discard` (DB I/O) would gratuitously serialize
        // unrelated stops.
        let request_ids: Vec<Uuid> = {
            let map = self.active_pauses.lock().await;
            map.get(&thread_id)
                .map(|set| set.iter().copied().collect())
                .unwrap_or_default()
        };
        if request_ids.is_empty() {
            return;
        }
        debug!(
            thread = %thread_id,
            count = request_ids.len(),
            "BridgeGateController::cancel_thread: waking parked gates",
        );
        for request_id in request_ids {
            // Deliver Cancelled to the parked future. Returns false if
            // the future has already woken (resolution arrived between
            // our snapshot and try_deliver) — that's fine, we just
            // skip the discard for the same reason.
            let _ = self
                .resolutions
                .try_deliver(request_id, GateResolution::Cancelled)
                .await;
        }
        // Discard any pending DB rows for this thread so the UI doesn't
        // keep showing stranded prompts. We don't have the
        // `pending_key` here (only request_id), so use the thread-level
        // discard helper if one exists — otherwise this is the cost of
        // not threading the key through. The pause() future will run
        // its own cleanup when it wakes; this branch is a defence in
        // depth for the case where a row was committed but the
        // resolution channel was closed.
        let _removed = self.pending_gates.discard_for_thread(thread_id).await;
        // Clear the active set for this thread now that all parked
        // pauses have been notified.
        self.active_pauses.lock().await.remove(&thread_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `try_deliver` returns `false` for an unknown request_id (the
    /// engine isn't waiting). The resolve endpoint uses this to decide
    /// whether to fall through to the legacy re-entry path.
    #[tokio::test]
    async fn try_deliver_unknown_request_returns_false() {
        let resolutions = GateResolutions::new();
        let delivered = resolutions
            .try_deliver(Uuid::new_v4(), GateResolution::Approved { always: false })
            .await;
        assert!(!delivered, "unknown request_id must report false");
    }

    /// Round-trip: register a sender, hand it to a spawned task that
    /// awaits the receiver, then deliver. The task must observe the
    /// resolution and `try_deliver` must report `true`.
    #[tokio::test]
    async fn try_deliver_routes_to_registered_receiver() {
        let resolutions = Arc::new(GateResolutions::new());
        let request_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        resolutions.register(request_id, tx).await;

        let receiver_task = tokio::spawn(async move { rx.await.ok() });

        let delivered = resolutions
            .try_deliver(request_id, GateResolution::Denied { reason: None })
            .await;
        assert!(delivered, "registered request_id must report true");

        let received = receiver_task.await.expect("task panicked");
        assert!(matches!(received, Some(GateResolution::Denied { .. })));
    }

    /// `cancel_thread()` wakes a `pause()` future parked on the given
    /// thread with `GateResolution::Cancelled`. Without this hook,
    /// `ThreadManager::stop_thread()` would have to wait up to the
    /// 30-minute gate expiry before the engine task observed the stop.
    #[tokio::test]
    async fn cancel_thread_wakes_parked_pause_with_cancelled_resolution() {
        use ironclaw_engine::GateController;
        use ironclaw_engine::ResumeKind;
        use ironclaw_engine::ThreadId;

        let controller = Arc::new(BridgeGateController::new(
            Arc::new(crate::gate::store::PendingGateStore::in_memory()),
            None,
            Arc::new(crate::tools::ToolRegistry::new()),
            None,
            None,
            Arc::new(crate::channels::ChannelManager::new()),
            Arc::new(GateResolutions::new()),
        ));
        let thread_id = ThreadId::new();
        let user_id = "stop-during-wait-user".to_string();
        let conversation_id = ironclaw_engine::ConversationId::new();

        // Bridge populates the per-execution context before invoking
        // the engine. Without it `pause()` cancels immediately
        // (no per-execution lookup hit), which would also pass the
        // test for the wrong reason.
        controller
            .set_execution_context(
                user_id.clone(),
                thread_id,
                PerExecutionContext {
                    conversation_id,
                    source_channel: "test".into(),
                    scope_thread_id: None,
                    channel_metadata: serde_json::json!({}),
                    original_message: None,
                },
            )
            .await;

        // Park a pause() future in a spawned task. It will block on the
        // approval-resolution oneshot (and the 30-min sleep) until
        // cancel_thread wakes it.
        let controller_clone = controller.clone();
        let user_clone = user_id.clone();
        let pause_task = tokio::spawn(async move {
            controller_clone
                .pause(GatePauseRequest {
                    thread_id,
                    user_id: user_clone,
                    gate_name: "approval".into(),
                    action_name: "test_tool".into(),
                    call_id: "call_stop_test".into(),
                    parameters: serde_json::json!({}),
                    resume_kind: ResumeKind::Approval { allow_always: true },
                    conversation_id: Some(conversation_id),
                })
                .await
        });

        // Let pause() reach the select await before we cancel.
        // The track_active_pause + register happen synchronously after
        // the pending_gates.insert; one tokio yield is sufficient on
        // current_thread runtime because pause() yields at .await
        // points before reaching the select.
        for _ in 0..10 {
            tokio::task::yield_now().await;
            if !controller
                .active_pauses
                .lock()
                .await
                .get(&thread_id)
                .map(|s| s.is_empty())
                .unwrap_or(true)
            {
                break;
            }
        }
        assert!(
            !controller
                .active_pauses
                .lock()
                .await
                .get(&thread_id)
                .map(|s| s.is_empty())
                .unwrap_or(true),
            "pause must register an active entry before we cancel"
        );

        // Now stop the thread. The pause future should resolve
        // promptly (well under the 30-minute expiry).
        controller.cancel_thread(thread_id).await;

        let resolution = tokio::time::timeout(std::time::Duration::from_secs(2), pause_task)
            .await
            .expect("cancel_thread must wake parked pause within 2s")
            .expect("pause task did not panic");
        assert!(
            matches!(resolution, GateResolution::Cancelled),
            "stop must surface as Cancelled; got {resolution:?}"
        );

        // Active set is cleared.
        assert!(
            !controller
                .active_pauses
                .lock()
                .await
                .contains_key(&thread_id),
            "active_pauses must be cleared after cancel_thread"
        );
    }

    /// `cancel_thread()` is a no-op when no pause is parked on the
    /// thread. `ThreadManager::stop_thread()` always calls it; the
    /// happy path (no inline-await waiter) must not panic or block.
    #[tokio::test]
    async fn cancel_thread_with_no_active_pause_is_a_no_op() {
        use ironclaw_engine::GateController;
        use ironclaw_engine::ThreadId;

        let controller = BridgeGateController::new(
            Arc::new(crate::gate::store::PendingGateStore::in_memory()),
            None,
            Arc::new(crate::tools::ToolRegistry::new()),
            None,
            None,
            Arc::new(crate::channels::ChannelManager::new()),
            Arc::new(GateResolutions::new()),
        );
        // Should return immediately.
        controller.cancel_thread(ThreadId::new()).await;
    }

    /// `try_deliver` returns `false` when the receiver was dropped
    /// before delivery. The resolve endpoint then falls through, and
    /// the corresponding `PendingGate` is treated as stale.
    #[tokio::test]
    async fn try_deliver_returns_false_when_receiver_dropped() {
        let resolutions = GateResolutions::new();
        let request_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        resolutions.register(request_id, tx).await;
        drop(rx);

        let delivered = resolutions
            .try_deliver(request_id, GateResolution::Approved { always: false })
            .await;
        assert!(!delivered, "dropped receiver must report false");
    }

    /// A second `try_deliver` for the same request_id returns `false`
    /// — the entry was consumed by the first delivery.
    #[tokio::test]
    async fn try_deliver_is_one_shot() {
        let resolutions = Arc::new(GateResolutions::new());
        let request_id = Uuid::new_v4();
        let (tx, _rx) = oneshot::channel();
        resolutions.register(request_id, tx).await;

        let first = resolutions
            .try_deliver(request_id, GateResolution::Approved { always: false })
            .await;
        let second = resolutions
            .try_deliver(request_id, GateResolution::Approved { always: false })
            .await;
        // Note: `first` may be `false` because we dropped rx — but the
        // entry is still consumed, so `second` must always be `false`.
        assert!(!second, "second delivery must report false");
        let _ = first;
    }
}
