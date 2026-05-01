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

use std::collections::HashMap;
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

/// Process-wide registry of in-flight gate resolution channels.
///
/// One entry per pending in-flight gate. Inserts come from
/// [`BridgeGateController::pause`]; removes come from
/// [`GateResolutions::try_deliver`] (the resolve endpoint).
///
/// Stranded entries from a prior crash do not exist — restarting the
/// process drops the registry. Stale `PendingGate` rows surviving
/// restart are cleaned up by the startup sweep in `router.rs`.
#[derive(Default)]
pub struct GateResolutions {
    inner: Mutex<HashMap<Uuid, oneshot::Sender<GateResolution>>>,
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
        match sender {
            Some(tx) => tx.send(resolution).is_ok(),
            None => false,
        }
    }

    async fn register(&self, request_id: Uuid, sender: oneshot::Sender<GateResolution>) {
        self.inner.lock().await.insert(request_id, sender);
    }

    async fn forget(&self, request_id: Uuid) {
        self.inner.lock().await.remove(&request_id);
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
    per_execution: Mutex<HashMap<ExecutionKey, PerExecutionContext>>,
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
        }
    }

    /// Bind per-execution data for `(user_id, thread_id)`. Call this
    /// before invoking an engine turn that may surface a gate; clear it
    /// with [`Self::clear_execution_context`] when the turn ends.
    pub async fn set_execution_context(
        &self,
        user_id: String,
        thread_id: ThreadId,
        context: PerExecutionContext,
    ) {
        self.per_execution
            .lock()
            .await
            .insert(ExecutionKey { user_id, thread_id }, context);
    }

    /// Drop per-execution data. Idempotent.
    pub async fn clear_execution_context(&self, user_id: &str, thread_id: ThreadId) {
        self.per_execution.lock().await.remove(&ExecutionKey {
            user_id: user_id.to_string(),
            thread_id,
        });
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
    ) -> Option<PerExecutionContext> {
        self.per_execution
            .lock()
            .await
            .get(&ExecutionKey {
                user_id: user_id.to_string(),
                thread_id,
            })
            .cloned()
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
            );
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
        // Inline gate-await currently only handles Approval. Authentication
        // and External resume kinds need state installed *before* the
        // suspended call can succeed (credential in secrets store, callback
        // payload), so they keep the legacy `ThreadOutcome::GatePaused`
        // re-entry path. The engine should not be reaching here for those
        // kinds; if it does, treat as cancellation so the call surfaces a
        // clear error rather than hanging.
        if !matches!(request.resume_kind, ResumeKind::Approval { .. }) {
            debug!(
                kind = %request.resume_kind.kind_name(),
                "BridgeGateController: non-Approval resume kind reached inline await; cancelling",
            );
            return GateResolution::Cancelled;
        }

        let Some(per_exec) = self
            .lookup_per_execution(&request.user_id, request.thread_id)
            .await
        else {
            // No per-execution context registered. This shouldn't happen
            // when invoked through `handle_with_engine`, which always
            // populates it before invoking the engine. If it does, the
            // safe move is to cancel — we have no channel to surface a
            // prompt on.
            debug!(
                user = %request.user_id,
                thread = %request.thread_id,
                "BridgeGateController: no per-execution context registered; cancelling",
            );
            return GateResolution::Cancelled;
        };

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
            // Insert can fail if a gate already exists for this
            // (user, thread). Surface as cancel rather than dropping
            // the prompt silently.
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

        self.emit_gate_prompt(&pending, &per_exec.channel_metadata)
            .await;

        match rx.await {
            Ok(resolution) => resolution,
            Err(_) => {
                // Sender dropped — process shutting down or registry
                // cleared. Treat as cancellation.
                self.resolutions.forget(request_id).await;
                GateResolution::Cancelled
            }
        }
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
            .try_deliver(
                Uuid::new_v4(),
                GateResolution::Approved { always: false },
            )
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
