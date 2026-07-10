// arch-exempt: large_file, loop host integration awaits dependency decomposition, plan #4088
use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures_util::FutureExt;
use ironclaw_events::{DurableEventLog, EventCursor, EventStreamKey, ReadScope, SecurityAuditSink};
use ironclaw_hooks::dispatch::{HookDispatcher, HookDispatcherBuilder};
use ironclaw_hooks::middleware::{
    CapabilityInputResolver as HookCapabilityInputResolver,
    CapabilityProviderResolver as HookCapabilityProviderResolver, HookPromptMaterializationSink,
    HookedLoopCapabilityPort, HookedLoopCheckpointPort, HookedLoopModelPort, HookedLoopPromptPort,
    HookedLoopTranscriptPort,
};
use ironclaw_host_api::{CapabilityId, ExtensionId};
use ironclaw_loop_support::{
    ACTIVE_TASK_COMPACTION_SYSTEM_PROMPT, CapabilityResolveError, CapabilitySurfaceProfileFilter,
    CapabilitySurfaceProfileResolver, EmptyLoopCapabilityPort, EmptyUserProfileSource,
    GuardedSystemInferencePort, HostIdentityContextSource, HostInputQueue, HostManagedModelGateway,
    HostQueueLoopInputPort, HostSkillContextSource, HostUserProfileSource, LoopAttachmentReadPort,
    LoopCapabilityInputResolver, LoopCapabilityPortFactory, ModelGatewayBackedSystemInferencePort,
    RunCancellationFactory, RunCancellationObservationKind, RunStateLoopCancellationPort,
    SubagentLoopPromptPort, SubagentPromptComposer, ThreadBackedLoopContextPort,
    ThreadBackedLoopTranscriptPort, ThreadContextWindowCache, TurnStateRunCancellationFactory,
    active_task_compaction_prompt_id, host_managed_loop_compaction_port_with_prompt_id,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};

use crate::driver_registry::{DriverRequirements, LoopDriverRegistryKey, RequirementLevel};
use crate::hook_gate_refs::HookGateInvocationScopePort;
use crate::model_routes::{ModelRouteError, ModelRouteResolver, ModelSlot};
use crate::planned_driver_factory::is_subagent_planned_run_profile;
use crate::text_loop_driver::{TEXT_ONLY_DRIVER_ID, TEXT_ONLY_DRIVER_VERSION};

mod config;
mod model_gateway;
mod port_adapters;

pub use config::{RebornLoopDriverHostError, RebornLoopDriverHostRequest, TextOnlyLoopHostConfig};
use model_gateway::ThreadResolvingLoopModelGateway;
use port_adapters::{
    HostManagedLoopCheckpointPort, HostManagedLoopProgressPort, NoExtraLoopInputPort,
};

// Legacy text-only driver key used by `is_text_only_driver_key`'s fail-closed
// allowlist. Kept alongside `TEXT_ONLY_DRIVER_ID` so legacy registry entries
// still resolve through the text-only host path. Retire once no callers
// register or persist the `lightweight_loop` key.
const LEGACY_TEXT_ONLY_DRIVER_ID: &str = "lightweight_loop";
const LEGACY_TEXT_ONLY_DRIVER_VERSION: u64 = 1;
const LEGACY_TEXT_ONLY_CHECKPOINT_SCHEMA_ID: &str = "interactive_checkpoint_v1";
const LEGACY_TEXT_ONLY_CHECKPOINT_SCHEMA_VERSION: u64 = 1;
const USER_PROFILE_WARMUP_TIMEOUT: Duration = Duration::from_millis(500);

fn should_fetch_communication_context(context: &LoopRunContext) -> bool {
    !matches!(
        context.product_context.as_ref().map(|ctx| ctx.origin),
        Some(ironclaw_turns::TurnOriginKind::WebUi)
    )
}

fn trace_host_factory_latency_ok(
    operation: &'static str,
    context: &LoopRunContext,
    started_at: Option<Instant>,
) {
    ironclaw_observability::live_latency_trace_ok!(
        "reborn_loop_host_factory",
        operation,
        started_at,
        tenant_id = %context.scope.tenant_id,
        agent_id = context.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = context.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %context.thread_id,
        owner_user_id = context.scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = %context.run_id,
        turn_id = %context.turn_id,
        "reborn loop host factory operation completed",
    );
}

fn trace_host_factory_latency_error<E: ?Sized>(
    operation: &'static str,
    context: &LoopRunContext,
    started_at: Option<Instant>,
    _error: &E,
) {
    // Host errors can include backend-provided details. Keep latency traces
    // cardinality-bounded and non-sensitive; `operation` identifies the stage.
    ironclaw_observability::live_latency_trace_error!(
        "reborn_loop_host_factory",
        operation,
        started_at,
        "host_factory_error",
        tenant_id = %context.scope.tenant_id,
        agent_id = context.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = context.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %context.thread_id,
        owner_user_id = context.scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = %context.run_id,
        turn_id = %context.turn_id,
        "reborn loop host factory operation failed",
    );
}

use ironclaw_turns::{
    CheckpointStateStore, LoopCheckpointStateRef, LoopCheckpointStore, RunProfileId,
    TurnCheckpointId, TurnError, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
    TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef, BeginAssistantDraft,
        CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityInvocation, CapabilityOutcome,
        CommunicationContextProvider, FinalizeAssistantMessage, HookMilestoneSink,
        HostManagedLoopModelPort, HostManagedLoopPromptPort,
        InMemoryInstructionMaterializationStore, InstructionBundleMaterializedMessage,
        InstructionMaterializationStore, InstructionSafetyContext, LoadCheckpointPayloadRequest,
        LoadedCheckpointPayload, LoopCancellationPort, LoopCancellationSignal, LoopCapabilityPort,
        LoopCheckpointPort, LoopCheckpointRequest, LoopCompactionError, LoopCompactionOutcome,
        LoopCompactionPort, LoopCompactionRequest, LoopContextBundle, LoopContextPort,
        LoopContextRequest, LoopHostMilestoneSink, LoopInputAckToken, LoopInputBatch,
        LoopInputCursor, LoopInputPort, LoopModelBudgetAccountant, LoopModelGateway,
        LoopModelPolicyGuard, LoopModelPort, LoopModelRequest, LoopModelResponse,
        LoopProgressEvent, LoopProgressPort, LoopPromptBundle, LoopPromptBundleAuthority,
        LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, LoopRunInfoPort,
        LoopRuntimeContext, LoopTranscriptPort, NoOpBudgetAccountant, NoOpPolicyGuard,
        ProviderToolCall, ProviderToolDefinition, RegisterProviderToolCallRequest,
        RunScopedHookMilestoneSink, StageCheckpointPayloadRequest, SystemInferencePort,
        UpdateAssistantDraft, VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
    runner::ClaimedTurnRun,
};
use tokio::task::JoinHandle;

struct ProfiledCapabilityHostRuntime {
    capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
}

/// Provider resolver that consults the current visible-capability surface
/// to map `capability_id` → `provider: ExtensionId`. Wires the hook
/// middleware to the same surface the inner port already tracks via
/// `SurfaceTrackingLoopCapabilityPort`. Without this, `OwnCapabilities`-
/// scoped hooks never fire because `ctx.provider` stays `None`
/// (henrypark133 Critical #2).
struct SurfaceBackedProviderResolver {
    surface_state: Arc<CapabilitySurfaceState>,
}

#[async_trait]
impl HookCapabilityProviderResolver for SurfaceBackedProviderResolver {
    async fn provider_for(&self, capability_id: &str) -> Option<ExtensionId> {
        let surface = self.surface_state.current().ok().flatten()?;
        surface
            .descriptors
            .iter()
            .find(|d| d.capability_id.as_str() == capability_id)
            .and_then(|d| d.provider.clone())
    }
}

/// Adapter that lets `HookedLoopPromptPort` write hook-emitted
/// `msg:hook.*` content into the host's `InstructionMaterializationStore`
/// so the downstream model resolver can resolve those refs. Captures the
/// `LoopRunContext` at construction time so the hook prompt port doesn't
/// need to know about run-profile types.
///
/// Threat-model + henrypark133 Critical #1: without this adapter wired
/// through the factory, hook prompt patches produce unresolvable refs and
/// the request fails with `model message reference is unavailable`.
struct InstructionStoreBackedHookSink {
    store: Arc<dyn InstructionMaterializationStore>,
    run_context: LoopRunContext,
}

impl HookPromptMaterializationSink for InstructionStoreBackedHookSink {
    fn put(
        &self,
        role: &str,
        content_ref: &ironclaw_turns::LoopMessageRef,
        safe_content: String,
    ) -> Result<(), AgentLoopHostError> {
        self.store.put_materialized_messages(
            &self.run_context,
            vec![InstructionBundleMaterializedMessage {
                role: role.to_string(),
                content_ref: content_ref.clone(),
                model_content: safe_content,
            }],
        )
    }
}

#[derive(Default)]
struct CapabilitySurfaceState {
    current: Mutex<Option<VisibleCapabilitySurface>>,
}

impl CapabilitySurfaceState {
    fn set_current(&self, surface: VisibleCapabilitySurface) -> Result<(), AgentLoopHostError> {
        let mut current = self.current.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability surface state is unavailable",
            )
        })?;
        *current = Some(surface);
        Ok(())
    }

    fn clear_current(&self) -> Result<(), AgentLoopHostError> {
        let mut current = self.current.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "capability surface state is unavailable",
            )
        })?;
        *current = None;
        Ok(())
    }

    fn current(&self) -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError> {
        self.current
            .lock()
            .map(|current| current.clone())
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability surface state is unavailable",
                )
            })
    }
}

struct SurfaceTrackingLoopCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    surface_state: Arc<CapabilitySurfaceState>,
}

impl SurfaceTrackingLoopCapabilityPort {
    fn new(inner: Arc<dyn LoopCapabilityPort>, surface_state: Arc<CapabilitySurfaceState>) -> Self {
        Self {
            inner,
            surface_state,
        }
    }
}

fn capability_may_change_visible_surface(capability_id: &CapabilityId) -> bool {
    matches!(
        capability_id.as_str(),
        "builtin.extension_activate" | "builtin.extension_remove"
    )
}

#[async_trait]
impl LoopCapabilityPort for SurfaceTrackingLoopCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.inner.tool_definitions()
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ironclaw_turns::run_profile::ProviderToolCallCapabilityIds, AgentLoopHostError>
    {
        // MUST delegate to inner. The LoopCapabilityPort default resolves a call by
        // searching `self.tool_definitions()` (the disclosed/advertised surface),
        // which rejects every deferred tool with "outside the visible capability
        // surface" before the inner tool-disclosure forgiving path can resolve it.
        // This is the model gateway's resolvability pre-check, so it must reach
        // inner — same reason validate/register below already delegate.
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<ironclaw_turns::run_profile::CapabilityCallCandidate, AgentLoopHostError> {
        self.inner.register_provider_tool_call(request).await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let surface = self.inner.visible_capabilities(request).await?;
        self.surface_state.set_current(surface.clone())?;
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let may_change_surface = capability_may_change_visible_surface(&request.capability_id);
        let outcome = self.inner.invoke_capability(request).await?;
        if may_change_surface {
            self.surface_state.clear_current()?;
        }
        Ok(outcome)
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let may_change_surface = request
            .invocations
            .iter()
            .any(|invocation| capability_may_change_visible_surface(&invocation.capability_id));
        let outcome = self.inner.invoke_capability_batch(request).await?;
        if may_change_surface {
            self.surface_state.clear_current()?;
        }
        Ok(outcome)
    }
}

// `LoopCapabilityInputResolver` is now defined in `ironclaw_loop_support`
// (workspace-10 refactor); imported above.

/// Default upper bound (in bytes of UTF-8 JSON-serialized form) above which
/// [`HookCapabilityInputResolverAdapter`] refuses to forward resolved input to
/// `before_capability` hook predicates. The hook crate's
/// [`crate::ironclaw_hooks::points::SanitizedArguments`] already caps per-string
/// length and nesting depth, but does not bound total byte size; the adapter
/// rejects oversized payloads up front so an unexpectedly large blob doesn't
/// reach predicate evaluation. The default is intentionally generous (64 KiB)
/// to cover normal capability inputs; callers can tighten it via
/// [`HookCapabilityInputResolverAdapter::with_max_input_bytes`].
pub const DEFAULT_HOOK_CAPABILITY_INPUT_MAX_BYTES: usize = 64 * 1024;

/// Adapter that exposes a [`LoopCapabilityInputResolver`] to the
/// `ironclaw_hooks` middleware as a
/// [`ironclaw_hooks::middleware::CapabilityInputResolver`].
///
/// Production capability dispatch already requires a
/// [`LoopCapabilityInputResolver`] (used by [`HostRuntimeLoopCapabilityPort`]
/// to convert opaque `CapabilityInputRef`s into JSON inputs for the host
/// runtime). This adapter reuses that same resolver — and the same
/// `LoopRunContext` it was built for — to feed sanitized arguments to hook
/// predicate evaluators. Sharing the resolver guarantees that the hook
/// framework and the dispatch path see the same logical input for a given
/// `(run, input_ref)` pair.
///
/// Fail-closed semantics:
///
/// - If the inner resolver returns an error, the adapter returns `None`. The
///   hook framework treats `None` as "unresolved" and `NumericSum`-style
///   predicates fail closed (deny / pause) per the evaluator's existing
///   semantics.
/// - If the resolved JSON value exceeds the configured byte budget once
///   serialized, the adapter returns `None`. The framework's per-string
///   truncation and depth cap (in
///   [`ironclaw_hooks::points::SanitizedArguments`]) apply to predicate
///   evaluation, but the total payload size guard lives here so an oversized
///   body cannot reach the sanitizer at all.
pub struct HookCapabilityInputResolverAdapter {
    inner: Arc<dyn LoopCapabilityInputResolver>,
    run_context: LoopRunContext,
    max_input_bytes: usize,
}

impl HookCapabilityInputResolverAdapter {
    pub fn new(inner: Arc<dyn LoopCapabilityInputResolver>, run_context: LoopRunContext) -> Self {
        Self {
            inner,
            run_context,
            max_input_bytes: DEFAULT_HOOK_CAPABILITY_INPUT_MAX_BYTES,
        }
    }

    /// Override the maximum serialized-byte budget. Inputs whose serialized
    /// JSON exceeds this size resolve to `None` (predicate evaluators that
    /// depend on argument contents fail closed).
    #[must_use]
    pub fn with_max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }
}

#[async_trait]
impl HookCapabilityInputResolver for HookCapabilityInputResolverAdapter {
    async fn resolve(
        &self,
        invocation: &ironclaw_turns::run_profile::CapabilityInvocation,
    ) -> Option<serde_json::Value> {
        let value = match self
            .inner
            .resolve_capability_input(&self.run_context, &invocation.input_ref)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(
                    capability = %invocation.capability_id,
                    input_ref = %invocation.input_ref,
                    kind = ?error.kind,
                    safe_summary = %error.safe_summary,
                    "hook capability input resolution failed; treating as unresolved"
                );
                return None;
            }
        };
        let serialized_len = match serde_json::to_vec(&value) {
            Ok(bytes) => bytes.len(),
            Err(error) => {
                tracing::debug!(
                    capability = %invocation.capability_id,
                    input_ref = %invocation.input_ref,
                    error = %error,
                    "hook capability input could not be re-serialized; treating as unresolved"
                );
                return None;
            }
        };
        if serialized_len > self.max_input_bytes {
            tracing::debug!(
                capability = %invocation.capability_id,
                input_ref = %invocation.input_ref,
                serialized_len,
                max_input_bytes = self.max_input_bytes,
                "hook capability input exceeded byte budget; treating as unresolved"
            );
            return None;
        }
        Some(value)
    }
}

// `LoopCapabilityResultWriter` is now defined in `ironclaw_loop_support`
// (workspace-10 refactor); imported above.

pub type HookDispatcherFactory = Arc<dyn Fn() -> Arc<HookDispatcher> + Send + Sync + 'static>;

/// Per-run hook dispatcher builder factory.
///
/// Returns a freshly-minted [`HookDispatcherBuilder`] per host build, or an
/// error if the per-run install replay fails. The composition path validates
/// every install set once at composition time and replays the identical
/// scratch-validated set per run, so in practice this never errors; the
/// `Result` exists so a future regression surfaces as a build error rather than
/// a panic (CLAUDE.md forbids `.expect()` in production code).
pub type HookDispatcherBuilderFactory = Arc<
    dyn Fn() -> Result<HookDispatcherBuilder, RebornLoopDriverHostError> + Send + Sync + 'static,
>;

/// Default number of durable runtime events read per subscription poll.
pub const DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_BATCH_LIMIT: usize = 64;

/// Default delay between empty subscription polls when the durable log has
/// just been drained. Under sustained idle, the subscription backs off
/// exponentially up to [`DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_MAX_POLL_INTERVAL`]
/// rather than polling at this rate forever (PR #3640 finding C5).
/// Combined with [`DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_BATCH_LIMIT`], this
/// is the first-line throttle for event-triggered dispatch fanout: the
/// subscription processes at most `batch_limit` events per poll, so a storm
/// produced by mutual recursion between two installed hooks cycles at the
/// poll-interval rate rather than unboundedly.
///
/// The full per-hook DoS budget Henry flagged as required for the
/// Installed tier (henrypark133 should-fix #4 on PR #3640, tracked as
/// issue #3689) — a per-hook rate cap with poisoning + milestone on
/// overrun — is tracked as a follow-up. The existing self-trigger guard
/// catches the most common direct-recursion pattern; the throttle here
/// bounds indirect patterns until the proper budget design lands.
pub const DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Maximum effective poll interval under adaptive backoff. When the
/// subscription has seen `N` consecutive empty polls it sleeps for
/// `min(base * 2^N, MAX)` before the next poll, so an idle stream falls
/// back to ~1Hz instead of hammering the durable log at the configured
/// poll interval (PR #3640 finding C5). A non-empty batch resets the
/// streak so a producer burst restores low-latency dispatch immediately.
pub const DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_MAX_POLL_INTERVAL: Duration =
    Duration::from_millis(1_000);

/// Pull-driven durable runtime-event subscription for event-triggered hooks.
///
/// The subscription reads from [`DurableEventLog`] on its own tokio task and
/// dispatches matching hooks through the per-build [`HookDispatcher`]. Because
/// events are pulled from the durable log after append, the runtime event
/// producer does not wait for hook execution or hook backpressure.
/// Configuration for a single event-triggered hook subscription.
///
/// `Clone` is intentionally **not** derived. Cloning and spawning twice
/// would create two consumers reading from the same `start_cursor` and
/// dispatching each hook twice (henrypark133 should-fix #2 on PR
/// #3640). The factory deliberately uses
/// [`Self::clone_for_independent_spawn`] in its one call site so the
/// dual-consumer property is visible at the seam.
///
/// # Replay semantics
///
/// The subscription is **at-least-once**. Restarting from the same
/// `start_cursor` replays every event whose cursor is `>= start_cursor`
/// — including events already dispatched before the prior shutdown.
/// Cursor persistence is the caller's responsibility; for once-only
/// delivery, the caller must commit progress at hook completion and
/// resume from `last_committed_cursor + 1`. This is documented here
/// (henrypark133 should-fix #6 on PR #3640) rather than only in the
/// design doc since it's a load-bearing public-API property.
pub struct EventTriggeredHookSubscription {
    log: Arc<dyn DurableEventLog>,
    stream: EventStreamKey,
    read_scope: ReadScope,
    start_cursor: EventCursor,
    batch_limit: usize,
    poll_interval: Duration,
    /// Upper bound on the adaptive idle poll interval. See
    /// [`DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_MAX_POLL_INTERVAL`].
    max_poll_interval: Duration,
}

impl EventTriggeredHookSubscription {
    /// Cheap deep-copy of the configuration so the factory can mint one
    /// independent background task per host build. Named verbosely
    /// because the alternative (`Clone` derive) would let external
    /// callers create two simultaneous consumers of the same stream by
    /// accident — see the type-level rustdoc above.
    pub(crate) fn clone_for_independent_spawn(&self) -> Self {
        Self {
            log: Arc::clone(&self.log),
            stream: self.stream.clone(),
            read_scope: self.read_scope.clone(),
            start_cursor: self.start_cursor,
            batch_limit: self.batch_limit,
            poll_interval: self.poll_interval,
            max_poll_interval: self.max_poll_interval,
        }
    }

    pub fn new(
        log: Arc<dyn DurableEventLog>,
        stream: EventStreamKey,
        read_scope: ReadScope,
        start_cursor: EventCursor,
    ) -> Self {
        Self {
            log,
            stream,
            read_scope,
            start_cursor,
            batch_limit: DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_BATCH_LIMIT,
            poll_interval: DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_POLL_INTERVAL,
            max_poll_interval: DEFAULT_EVENT_TRIGGERED_SUBSCRIPTION_MAX_POLL_INTERVAL,
        }
    }

    #[must_use]
    pub fn with_batch_limit(mut self, batch_limit: usize) -> Self {
        self.batch_limit = batch_limit.max(1);
        self
    }

    #[must_use]
    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval.max(Duration::from_millis(1));
        // Keep the max never below the base interval — otherwise the
        // adaptive backoff would clamp below the user-configured floor.
        if self.max_poll_interval < self.poll_interval {
            self.max_poll_interval = self.poll_interval;
        }
        self
    }

    #[must_use]
    pub fn with_max_poll_interval(mut self, max_poll_interval: Duration) -> Self {
        self.max_poll_interval = max_poll_interval.max(self.poll_interval);
        self
    }

    /// Verify the subscription's stream key against the host's run scope and
    /// compute the **effective** [`ReadScope`] that the background task must
    /// use for replay. The subscription stream partitions events by
    /// `(tenant, user, agent)`; `ReadScope` filters by
    /// `(project, mission, thread, process)`. If a caller wires a host for
    /// tenant A but supplies a subscription pointing at tenant B's stream (or
    /// to a stream-key that names a foreign user), the host would dispatch B's
    /// hook events into A's dispatcher with A's context tenant — a
    /// cross-tenant trust-boundary break (NOTE(#3640)).
    ///
    /// PR #3640 followup (Bug 1, cross-thread/project leakage): the supplied
    /// `read_scope` is **not trusted as-is**. The effective filter is always
    /// *derived* from the authoritative run/thread scope, so the subscription
    /// can never observe events from other threads/projects/missions that share
    /// the same `(tenant, user, agent)` stream — regardless of what the caller
    /// passed. The derived filter always pins `thread_id` to the run's thread
    /// and pins `project_id`/`mission_id` whenever the run/thread scope carries
    /// them; only `process_id` (a strict narrowing within the thread) is
    /// carried from the caller. Any explicit `Some(want)` the caller supplied
    /// for `project_id`/`mission_id`/`thread_id` must additionally equal the
    /// corresponding run/thread value, so a caller cannot *widen* the scope
    /// below what the run authorizes.
    ///
    /// PR #3931 followup: a permissive `ReadScope::any()` is no longer
    /// special-cased / rejected. Because the read uses the derived scope above
    /// (installed via [`Self::with_read_scope`] before the background task
    /// spawns — never the caller's raw filter), `any()` cannot widen the read:
    /// its `None` fields are simply overwritten by the authoritative
    /// derivation. The rejection was redundant ceremony, not a load-bearing
    /// security check.
    fn effective_read_scope(
        &self,
        run_scope: &ironclaw_turns::TurnScope,
        thread_scope: &ironclaw_threads::ThreadScope,
    ) -> Result<ReadScope, String> {
        if self.stream.tenant_id != run_scope.tenant_id {
            return Err(format!(
                "event subscription stream tenant_id={} does not match run scope tenant_id={}",
                self.stream.tenant_id.as_str(),
                run_scope.tenant_id.as_str(),
            ));
        }
        if self.stream.agent_id != run_scope.agent_id {
            return Err(format!(
                "event subscription stream agent_id={:?} does not match run scope agent_id={:?}",
                self.stream.agent_id.as_ref().map(|a| a.as_str()),
                run_scope.agent_id.as_ref().map(|a| a.as_str()),
            ));
        }
        // The user dimension is carried on the thread scope (owner). Treat a
        // thread without an owner conservatively — refuse to bind the
        // subscription since we cannot verify the stream's user matches.
        let Some(owner) = thread_scope.owner_user_id.as_ref() else {
            return Err(
                "event subscription cannot bind to thread without an owner user".to_string(),
            );
        };
        if &self.stream.user_id != owner {
            return Err(format!(
                "event subscription stream user_id={} does not match thread owner user_id={}",
                self.stream.user_id.as_str(),
                owner.as_str(),
            ));
        }
        // Any explicit Some(want) the caller supplied must equal the
        // corresponding run/thread scope value — a caller may tighten but
        // never widen below the run's authority.
        if let Some(want) = self.read_scope.project_id.as_ref()
            && run_scope.project_id.as_ref() != Some(want)
        {
            return Err(format!(
                "event subscription read_scope.project_id={} does not match run scope project_id={:?}",
                want.as_str(),
                run_scope.project_id.as_ref().map(|p| p.as_str()),
            ));
        }
        if let Some(want) = self.read_scope.mission_id.as_ref()
            && thread_scope.mission_id.as_ref() != Some(want)
        {
            return Err(format!(
                "event subscription read_scope.mission_id={} does not match thread scope mission_id={:?}",
                want.as_str(),
                thread_scope.mission_id.as_ref().map(|m| m.as_str()),
            ));
        }
        if let Some(want) = self.read_scope.thread_id.as_ref()
            && &run_scope.thread_id != want
        {
            return Err(format!(
                "event subscription read_scope.thread_id={} does not match run scope thread_id={}",
                want.as_str(),
                run_scope.thread_id.as_str(),
            ));
        }
        // Derive the effective filter from the authoritative run/thread scope.
        // thread_id is always present on the run scope and is the minimum
        // tightening that prevents cross-thread leakage. project_id/mission_id
        // are pinned whenever the run/thread scope carries them. process_id is
        // preserved from the caller's supplied filter (it is a strictly
        // narrower partition within the thread and the run scope does not own
        // it).
        //
        // PR #3931 M3: when the run carries no project_id (a legitimate
        // tenant/agent-level run), the derived ReadScope.project_id is also
        // None, so the subscription is not pinned on the project dimension —
        // thread_id still pins it to a single thread. This is not fail-closed
        // (project-less runs are legitimate), but make the unpinned dimension
        // observable so it is auditable.
        if run_scope.project_id.is_none() {
            tracing::warn!(
                tenant_id = run_scope.tenant_id.as_str(),
                agent_id = ?run_scope.agent_id.as_ref().map(|a| a.as_str()),
                thread_id = run_scope.thread_id.as_str(),
                "event subscription read scope is not project-pinned (run has no \
                 project_id); subscription remains pinned by thread_id only"
            );
        }
        Ok(ReadScope {
            project_id: run_scope.project_id.clone(),
            mission_id: thread_scope.mission_id.clone(),
            thread_id: Some(run_scope.thread_id.clone()),
            process_id: self.read_scope.process_id,
        })
    }

    /// Override the read scope on a freshly-cloned subscription. Used by the
    /// host build path to install the derived effective scope before the
    /// background task spawns. See [`Self::effective_read_scope`].
    ///
    /// Deliberately module-private (not `pub(crate)`): it has no cross-module
    /// caller, and widening its visibility would let other `ironclaw_reborn`
    /// code install an arbitrary `ReadScope` (e.g. `ReadScope::any()`) and
    /// bypass `effective_read_scope`, reopening the cross-thread/project leak
    /// PR #3931 Bug 1 closed.
    #[must_use]
    fn with_read_scope(mut self, read_scope: ReadScope) -> Self {
        self.read_scope = read_scope;
        self
    }

    async fn spawn(
        self,
        dispatcher: Arc<HookDispatcher>,
        tenant_id: ironclaw_host_api::TenantId,
        run_context: LoopRunContext,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> EventTriggeredHookSubscriptionHandle {
        // PR #3931 (Hole 1): snapshot the replay/live boundary **here**, at
        // subscription start, synchronously before the background task is
        // spawned. Doing it inside the spawned task would re-introduce the
        // race: an event appended between build-return and the task's first
        // poll could land at or below a lazily-read head and be mis-classified
        // as replay. Snapshotting before `tokio::spawn` makes "head-at-startup"
        // mean exactly the head at the instant the host is built.
        let startup_head = match self.log.head_cursor(&self.stream, self.start_cursor).await {
            Ok(head) => head,
            Err(error) => {
                // Fail-closed: without a boundary we cannot classify replay vs
                // live. Don't fail the host build (consistent with the
                // ReplayGap-as-milestone path); spawn a task that only emits
                // the operator-visible terminated milestone so the dead
                // subscription is auditable.
                tracing::error!(
                    error = %error,
                    "event-triggered hook subscription could not snapshot stream head at start; \
                     subscription will not dispatch"
                );
                let sink = Arc::clone(&milestone_sink);
                let context = run_context.clone();
                let task = tokio::spawn(async move {
                    emit_subscription_terminated_note(
                        &sink,
                        &context,
                        "event subscription stopped: head snapshot failed",
                    )
                    .await;
                });
                return EventTriggeredHookSubscriptionHandle { task };
            }
        };
        // henrypark133 should-fix #3 on PR #3640: wrap the background
        // task body in `catch_unwind`. Without it, a panic in `run()`
        // terminates the task silently — no operator-visible signal.
        // On panic, emit the same `EventSubscriptionTerminated`
        // milestone the `ReplayGap` path already emits so audit/SSE
        // consumers learn the subscription died and why.
        let panic_sink = Arc::clone(&milestone_sink);
        let panic_run_context = run_context.clone();
        let task = tokio::spawn(async move {
            let outcome = std::panic::AssertUnwindSafe(self.run(
                dispatcher,
                tenant_id,
                run_context,
                milestone_sink,
                startup_head,
            ))
            .catch_unwind()
            .await;
            if outcome.is_err() {
                tracing::error!(
                    "event-triggered hook subscription task panicked; \
                     emitting EventSubscriptionTerminated milestone"
                );
                emit_subscription_terminated_note(
                    &panic_sink,
                    &panic_run_context,
                    "event subscription stopped: task panic",
                )
                .await;
            }
        });
        EventTriggeredHookSubscriptionHandle { task }
    }

    async fn run(
        self,
        dispatcher: Arc<HookDispatcher>,
        tenant_id: ironclaw_host_api::TenantId,
        run_context: LoopRunContext,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        startup_head: EventCursor,
    ) {
        let mut cursor = self.start_cursor;
        // Adaptive backoff state (PR #3640 finding C5). `empty_streak`
        // counts consecutive polls that returned no entries; the sleep on
        // the next poll is `min(poll_interval << empty_streak, max_poll_interval)`.
        // A non-empty batch resets the streak so producer bursts restore
        // low-latency dispatch immediately. The shift is saturated at the
        // first iteration where the resulting interval reaches the cap,
        // which keeps the streak counter bounded.
        let mut empty_streak: u32 = 0;
        // PR #3640 followup (Bug 2, replay signal): events read while catching
        // up from `start_cursor` to the stream head at subscription time are
        // *replay* — on a reconnect/restart they may have already fired their
        // side effects, so hooks receive `is_replay = true` to dedupe.
        // Everything appended after the subscription started is *live*
        // (`is_replay = false`).
        //
        // PR #3931 (Hole 1, replay/live boundary race): `startup_head` is the
        // stream head snapshotted atomically at subscription start (see
        // `spawn`), NOT "the first poll that returns no entries". The old
        // empty-poll heuristic raced: a live record appended before the first
        // empty poll — or while a continuous backlog drains past the true head
        // — was mis-classified as replay and could be wrongly deduped/skipped
        // by the hook. The catch-up window is exactly the gap from
        // `start_cursor` to head-at-startup: a record is replay iff
        // `cursor <= startup_head`; anything beyond is live.
        loop {
            match self
                .log
                .read_after_cursor(
                    &self.stream,
                    &self.read_scope,
                    Some(cursor),
                    self.batch_limit,
                )
                .await
            {
                Ok(replay) => {
                    if replay.entries.is_empty() {
                        cursor = replay.next_cursor;
                        let sleep_for = adaptive_poll_interval(
                            self.poll_interval,
                            self.max_poll_interval,
                            empty_streak,
                        );
                        empty_streak = empty_streak.saturating_add(1);
                        tokio::time::sleep(sleep_for).await;
                        continue;
                    }
                    empty_streak = 0;
                    // Classify each record against the head snapshotted at
                    // startup. A mixed batch (some at/below `startup_head`,
                    // some above) is split at the boundary cursor-by-cursor —
                    // records `<= startup_head` replay, the rest are live.
                    for entry in replay.entries {
                        if entry.cursor <= startup_head {
                            dispatcher
                                .dispatch_event_triggered_replay_at(
                                    tenant_id.clone(),
                                    entry.cursor,
                                    &entry.record,
                                )
                                .await;
                        } else {
                            dispatcher
                                .dispatch_event_triggered_at(
                                    tenant_id.clone(),
                                    entry.cursor,
                                    &entry.record,
                                )
                                .await;
                        }
                    }
                    cursor = replay.next_cursor;
                }
                Err(ironclaw_events::EventError::ReplayGap {
                    requested,
                    earliest,
                }) => {
                    tracing::error!(
                        ?requested,
                        ?earliest,
                        "event-triggered hook subscription stopped after replay gap"
                    );
                    // NOTE(#3640): replay gaps used to be a silent
                    // warn+break. Surface as an operator-visible milestone
                    // so missing hook deliveries are auditable. Fail-closed:
                    // we break out of the loop after emitting because the
                    // subscription's at-most-once contract is already broken
                    // and resuming from `earliest` would silently lose the
                    // events in the gap.
                    emit_subscription_terminated_note(
                        &milestone_sink,
                        &run_context,
                        "event subscription stopped: replay gap",
                    )
                    .await;
                    break;
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "event-triggered hook subscription poll failed; retrying"
                    );
                    tokio::time::sleep(self.poll_interval).await;
                }
            }
        }
    }
}

/// Compute the next idle sleep duration for the event-triggered subscription
/// under adaptive backoff (PR #3640 finding C5).
///
/// - `base` is the configured `poll_interval`.
/// - `max` is the cap (`max_poll_interval`).
/// - `streak` is the count of consecutive empty polls observed so far.
///
/// Returns `min(base * 2^streak, max)`. Saturates instead of overflowing
/// when `streak` is large.
fn adaptive_poll_interval(base: Duration, max: Duration, streak: u32) -> Duration {
    // Clamp shift to 30 — `1u64 << 30` is ~1B and any `base * 2^30` would
    // dwarf any sensible `max`, so saturating at the cap is correct here.
    let shift = streak.min(30);
    let base_ns = base.as_nanos() as u64;
    let scaled = base_ns.saturating_mul(1u64 << shift);
    let capped = scaled.min(max.as_nanos() as u64);
    Duration::from_nanos(capped)
}

async fn emit_subscription_terminated_note(
    sink: &Arc<dyn LoopHostMilestoneSink>,
    run_context: &LoopRunContext,
    safe_summary: &str,
) {
    let summary = match ironclaw_turns::run_profile::LoopSafeSummary::new(safe_summary) {
        Ok(s) => s,
        Err(_) => {
            // Should never happen for our static strings, but if a future
            // caller passes something the validator rejects, just log and
            // proceed without the milestone rather than panicking inside
            // the background task.
            tracing::error!(
                rejected = safe_summary,
                "subscription-terminated note rejected by LoopSafeSummary::new"
            );
            return;
        }
    };
    let milestone = ironclaw_turns::run_profile::LoopHostMilestone {
        scope: run_context.scope.clone(),
        actor: run_context.actor.clone(),
        turn_id: run_context.turn_id,
        run_id: run_context.run_id,
        loop_driver_id: run_context.loop_driver_id.clone(),
        kind: ironclaw_turns::run_profile::LoopHostMilestoneKind::DriverNote {
            kind: ironclaw_turns::run_profile::LoopDriverNoteKind::EventSubscriptionTerminated,
            safe_summary: summary,
        },
    };
    if let Err(error) = sink.publish_loop_milestone(milestone).await {
        tracing::error!(
            error = %error,
            "failed to emit EventSubscriptionTerminated milestone"
        );
    }
}

struct EventTriggeredHookSubscriptionHandle {
    task: JoinHandle<()>,
}

impl Drop for EventTriggeredHookSubscriptionHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    model_gateway: Arc<G>,
    model_route_resolver: Option<Arc<dyn ModelRouteResolver>>,
    checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    model_accountant: Arc<dyn LoopModelBudgetAccountant>,
    model_policy_guard: Arc<dyn LoopModelPolicyGuard>,
    cancellation_factory: Arc<dyn RunCancellationFactory>,
    config: TextOnlyLoopHostConfig,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    attachment_read_port: Option<Arc<dyn LoopAttachmentReadPort>>,
    /// Optional hook dispatcher factory. When set, the factory invokes the
    /// closure on every `build_text_only_host*` call to obtain a fresh
    /// `HookDispatcher`, wraps it in `Arc`, and then plumbs it through
    /// `HookedLoopCapabilityPort` / `HookedLoopPromptPort`. Building a fresh
    /// dispatcher per host build means **dispatcher-local** state —
    /// slot-poisoning and any registry mutations done while a run is active —
    /// does not leak into the next run.
    ///
    /// This per-run freshness applies to the dispatcher/registry only. It does
    /// **not** apply to predicate counter state: rate-limit / value-cap
    /// counters are keyed `(hook, tenant, capability)` with no `run_id` and are
    /// deliberately TENANT-scoped, so they are shared across every run within a
    /// tenant (a run-scoped rate limit would reset each run and enforce
    /// nothing). The composition root captures one `PredicateEvaluator` per
    /// tenant and shares it across the dispatchers this factory mints — see
    /// `ironclaw_reborn_composition::hooks` ("Predicate counter scoping").
    /// Default behavior (no factory) is unchanged from the pre-hooks shape.
    hook_dispatcher_factory: Option<HookDispatcherFactory>,
    /// Per-build builder factory. Preferred over `hook_dispatcher_factory`
    /// because it lets the host factory attach the run-scoped milestone
    /// sink internally (henrypark133 Critical #4). Exactly one of these
    /// two should be set; if both are, the builder factory wins.
    hook_dispatcher_builder_factory: Option<HookDispatcherBuilderFactory>,
    /// Optional hook security-audit sink. Applied only on the builder-factory
    /// path, where the dispatcher is still mutable before Arc wrapping.
    hook_security_audit_sink: Option<Arc<dyn SecurityAuditSink>>,
    /// Optional capability-input resolver. When the hook dispatcher is set
    /// and a resolver is configured, the factory wraps it in a
    /// [`HookCapabilityInputResolverAdapter`] (bound to the current
    /// `LoopRunContext`) and threads it into `HookedLoopCapabilityPort` so
    /// argument-dependent predicates (e.g., `NumericSum`) evaluate against
    /// real capability arguments instead of failing closed.
    capability_input_resolver: Option<Arc<dyn LoopCapabilityInputResolver>>,
    /// Optional gate-ref factory for hook `PauseApproval` / `PauseAuth`
    /// decisions. Default behavior (no factory) is fail-closed — the hook
    /// suspension surfaces as `Denied`. Production deployments must install
    /// a factory that talks to the host's approval/auth router; tests can
    /// install `UuidHookGateRefFactory` to exercise the affirmative path.
    /// See [`Self::with_hook_gate_ref_factory`].
    hook_gate_ref_factory: Option<Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory>>,
    /// Per-build hook-gate-factory builder. See
    /// [`Self::with_hook_gate_ref_factory_builder`].
    hook_gate_ref_factory_builder: Option<HookGateRefFactoryBuilder>,
    /// Optional durable runtime-event subscription for event-triggered hooks.
    /// The subscription starts only when a hook dispatcher is also installed.
    event_subscription: Option<EventTriggeredHookSubscription>,
    safety_context: InstructionSafetyContext,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    /// Per-run user agent-context profile source. Started once at loop start
    /// and stamped into `LoopRuntimeContext.user_profile` only if it resolves
    /// before prompt wiring reaches the profile handoff. Defaults to
    /// `EmptyUserProfileSource` (returns `None`) so callers that do not wire a
    /// profile source degrade gracefully rather than failing.
    user_profile_source: Arc<dyn HostUserProfileSource>,
    communication_context_provider: Option<Arc<dyn CommunicationContextProvider>>,
    input_queue: Option<Arc<dyn HostInputQueue>>,
    profiled_capabilities: Option<ProfiledCapabilityHostRuntime>,
    subagent_prompt_composer: Option<SubagentPromptComposer>,
    driver_requirements: HashMap<LoopDriverRegistryKey, DriverRequirements>,
}

/// Per-host-build callback that produces a fresh hook-gate factory bound
/// to the active run. Preferred over a shared `Arc<dyn HookGateRefFactory>`
/// because it eliminates the stale-`LoopRunContext` capture risk
/// serrrfirat MEDIUM (5-15 review on PR #3633) flagged: a single shared
/// factory carries whatever run context it was constructed against, and
/// the host reuses the same `Arc` across every host it produces — so a
/// second host build could mint gate refs against the first build's run
/// context.
pub type HookGateRefFactoryBuilder = Arc<
    dyn Fn(&LoopRunContext) -> Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory>
        + Send
        + Sync,
>;

impl<S, G> RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    // arch-exempt: too_many_args, needs LoopHostDependencies aggregation, plan #4088
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        model_gateway: Arc<G>,
        checkpoint_state_store: Arc<dyn CheckpointStateStore>,
        turn_state_store: Arc<dyn TurnStateStore>,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
        config: TextOnlyLoopHostConfig,
        safety_context: InstructionSafetyContext,
    ) -> Self {
        let cancellation_factory: Arc<dyn RunCancellationFactory> = Arc::new(
            TurnStateRunCancellationFactory::new(Arc::clone(&turn_state_store)),
        );
        Self {
            thread_service,
            thread_scope,
            model_gateway,
            model_route_resolver: None,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
            model_accountant: Arc::new(NoOpBudgetAccountant),
            model_policy_guard: Arc::new(NoOpPolicyGuard),
            cancellation_factory,
            config,
            skill_context_source: None,
            attachment_read_port: None,
            hook_dispatcher_factory: None,
            hook_dispatcher_builder_factory: None,
            hook_security_audit_sink: None,
            capability_input_resolver: None,
            hook_gate_ref_factory: None,
            hook_gate_ref_factory_builder: None,
            event_subscription: None,
            safety_context,
            identity_context_source: None,
            user_profile_source: Arc::new(EmptyUserProfileSource),
            communication_context_provider: None,
            input_queue: None,
            profiled_capabilities: None,
            subagent_prompt_composer: None,
            driver_requirements: HashMap::new(),
        }
    }

    pub fn with_cancellation_factory(mut self, factory: Arc<dyn RunCancellationFactory>) -> Self {
        self.cancellation_factory = factory;
        self
    }

    pub fn cancellation_observation_kind(&self) -> RunCancellationObservationKind {
        self.cancellation_factory.observation_kind()
    }

    /// Per-run thread scope: the factory's installation scope
    /// (tenant/agent/project) with `owner_user_id` re-pointed at the
    /// run's authenticated actor. This is what makes the WebChat v2
    /// surface multi-user — each caller's thread reads and writes land
    /// in their own `owners/<user>` subtree (via
    /// [`ThreadScope::to_resource_scope`]), matching the owner the
    /// product facade already creates threads under
    /// (`reborn_services::create_thread` scopes to `caller.user_id`).
    ///
    /// The re-scoping applies only when the factory is owner-scoped (it
    /// declares an owner, as the serve path does by pinning the runtime
    /// owner). An owner-less factory keeps its shared/system slot, so
    /// owner-agnostic flows and runs without an actor (background tasks,
    /// triggers) are unaffected.
    fn effective_thread_scope(&self, run_context: &LoopRunContext) -> ThreadScope {
        crate::thread_scope::ThreadScopeResolver::resolve_for_turn(
            &self.thread_scope,
            &run_context.scope,
            run_context.actor(),
        )
    }

    fn build_compaction_ports(&self, run_context: &LoopRunContext) -> Arc<dyn LoopCompactionPort> {
        // Two arms (not a shared `Arc<dyn _>` helper) because the host's
        // `model_gateway: Arc<G>` is `G: ?Sized`: the resolved test gateway is
        // `Arc<dyn HostManagedModelGateway>` while the production fallback keeps
        // the host's own `Arc<G>`, and `Arc<G>` cannot be coerced to
        // `Arc<dyn _>` for a `?Sized` G (E0277). Unifying would require dropping
        // the gateway generic (the S1 seam the plan deliberately rejected).
        let direct_system_inference: Arc<dyn SystemInferencePort> =
            if let Some(gw) = self.model_gateway.resolve_for_scope(&run_context.scope) {
                Arc::new(ModelGatewayBackedSystemInferencePort::new(
                    gw,
                    run_context.clone(),
                ))
            } else {
                Arc::new(ModelGatewayBackedSystemInferencePort::new(
                    Arc::clone(&self.model_gateway),
                    run_context.clone(),
                ))
            };
        let system_inference: Arc<dyn SystemInferencePort> =
            Arc::new(GuardedSystemInferencePort::new(
                direct_system_inference,
                run_context.clone(),
                Arc::clone(&self.model_accountant),
                Arc::clone(&self.model_policy_guard),
            ));
        host_managed_loop_compaction_port_with_prompt_id(
            system_inference,
            Arc::clone(&self.thread_service),
            self.effective_thread_scope(run_context),
            active_task_compaction_prompt_id(),
            ACTIVE_TASK_COMPACTION_SYSTEM_PROMPT,
        )
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
        self
    }

    pub fn with_attachment_read_port(mut self, port: Arc<dyn LoopAttachmentReadPort>) -> Self {
        self.attachment_read_port = Some(port);
        self
    }

    /// Install a hook dispatcher factory closure. The closure is invoked once
    /// on every `build_text_only_host*` call to mint a fresh
    /// [`HookDispatcher`], which the factory then wraps in `Arc` and threads
    /// through `HookedLoopCapabilityPort` / `HookedLoopPromptPort`.
    ///
    /// This is the recommended hook installation path: per-build construction
    /// gives each host its own dispatcher, so slot poisoning, registry
    /// mutations, and any other dispatcher-owned state are scoped to a single
    /// run rather than shared across every host the factory ever produces.
    ///
    /// **Hook telemetry**: to surface hook dispatch in the host's milestone
    /// stream, the closure itself should attach a
    /// [`ironclaw_turns::run_profile::HookMilestoneSink`] (typically a
    /// [`ironclaw_turns::run_profile::RunScopedHookMilestoneSink`] wrapping
    /// the factory's `LoopHostMilestoneSink`) before returning the
    /// dispatcher. The wrapping happens inside the closure so each run gets a
    /// dispatcher already configured for telemetry. Hook activity is
    /// invisible to observers when no sink is attached.
    pub fn with_hook_dispatcher_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn() -> Arc<HookDispatcher> + Send + Sync + 'static,
    {
        self.hook_dispatcher_factory = Some(Arc::new(factory));
        self
    }

    /// Install a capability-input resolver for hook predicate evaluation.
    /// When set alongside a hook dispatcher, hook predicates that depend on
    /// argument contents (`ValueOrRateBound::NumericSum`, etc.) see real,
    /// sanitized input values; otherwise they fail closed because the hooks
    /// middleware defaults to the framework's `NullCapabilityInputResolver`.
    ///
    /// The resolver is the same trait used by [`HostRuntimeLoopCapabilityPort`]
    /// to convert opaque capability input refs into JSON arguments; production
    /// callers typically share a single implementation between dispatch and
    /// hook evaluation so both observe the same logical input.
    /// Install a hook dispatcher *builder* factory. **Preferred over
    /// [`Self::with_hook_dispatcher_factory`]** because the host factory
    /// can attach a `RunScopedHookMilestoneSink` keyed to the current
    /// `LoopRunContext` *internally*, before the builder is sealed —
    /// guaranteeing hook telemetry carries the right run/thread scope
    /// without caller bookkeeping (henrypark133 Critical #4).
    ///
    /// The closure is invoked once per `build_text_only_host*` call. It
    /// should construct a clean builder (no pre-attached milestone sink —
    /// the host wires one). Manifest-driven hook installations happen
    /// inside the closure exactly as with the legacy factory.
    pub fn with_hook_dispatcher_builder_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn() -> Result<HookDispatcherBuilder, RebornLoopDriverHostError> + Send + Sync + 'static,
    {
        self.hook_dispatcher_builder_factory = Some(Arc::new(factory));
        self
    }

    pub fn with_hook_security_audit_sink(mut self, sink: Arc<dyn SecurityAuditSink>) -> Self {
        self.hook_security_audit_sink = Some(sink);
        self
    }

    pub fn with_capability_input_resolver(
        mut self,
        resolver: Arc<dyn LoopCapabilityInputResolver>,
    ) -> Self {
        self.capability_input_resolver = Some(resolver);
        self
    }

    /// Install a `HookGateRefFactory` for hook-emitted `PauseApproval` /
    /// `PauseAuth` decisions. The default (no factory) is fail-closed: the
    /// suspension surfaces as `Denied` so the loop doesn't park on an
    /// unresolvable ref.
    ///
    /// Production: install a factory that reserves a gate through the
    /// host's real approval/auth router so the ref carries lease + one-shot
    /// semantics. Tests/dev: install `UuidHookGateRefFactory` to exercise
    /// the affirmative `ApprovalRequired { gate_ref }` shape (the refs are
    /// locally unique but not router-registered — production must not use
    /// this).
    /// **Deprecated for production use.** A single shared factory captures
    /// its `LoopRunContext` (typically through the `Fn() ->
    /// HookGateReservationContext` closure passed to
    /// `RouterBackedHookGateRefFactory::try_new`) at construction time
    /// and reuses it for every host build. The host factory itself
    /// produces many hosts with different run contexts, so a later
    /// build can mint a gate ref against a stale run — exactly the
    /// integration footgun serrrfirat MEDIUM on PR #3633 (5-15 review)
    /// flagged. Use [`Self::with_hook_gate_ref_factory_builder`]
    /// instead, which receives the current `LoopRunContext` per build
    /// so the factory can carry the right run identity by construction.
    #[deprecated(
        since = "0.1.0",
        note = "shared factory captures stale LoopRunContext across host \
                builds. Use `with_hook_gate_ref_factory_builder(|run_ctx| ...)` \
                so the factory is constructed fresh per host with the right \
                run context."
    )]
    pub fn with_hook_gate_ref_factory(
        mut self,
        factory: Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory>,
    ) -> Self {
        self.hook_gate_ref_factory = Some(factory);
        self
    }

    /// Install a per-build hook-gate-factory builder. The closure is
    /// invoked once per `build_text_only_host*` call with the current
    /// `LoopRunContext`; the returned factory is wired into that host's
    /// `HookedLoopCapabilityPort`. This is the preferred path for
    /// router-backed factories that need to bind the active run/actor
    /// at mint time (serrrfirat MEDIUM on PR #3633 5-15 review).
    pub fn with_hook_gate_ref_factory_builder<F>(mut self, factory_builder: F) -> Self
    where
        F: Fn(&LoopRunContext) -> Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory>
            + Send
            + Sync
            + 'static,
    {
        self.hook_gate_ref_factory_builder = Some(Arc::new(factory_builder));
        self
    }

    /// Install a pull-driven durable event subscription for event-triggered
    /// hooks. The event producer path is unchanged: this consumer polls the
    /// durable log on a background task and dispatches observer-only hooks
    /// outside the loop's inline tick.
    pub fn with_event_subscription(mut self, subscription: EventTriggeredHookSubscription) -> Self {
        self.event_subscription = Some(subscription);
        self
    }

    /// Install a shared [`HookDispatcher`] that wraps the capability and
    /// prompt ports for every host built by this factory.
    ///
    /// **Deprecated for production use.** This is preserved as a thin
    /// backward-compat wrapper that adapts a single `Arc<HookDispatcher>`
    /// into a factory closure cloning the same instance on every build. As a
    /// result, dispatcher-owned mutable state (poisoned slots, predicate
    /// counters, registry mutations) is **shared across every run** the
    /// factory produces — a hook poisoned in run N stays poisoned for runs
    /// N+1, N+2, …
    ///
    /// New callers should prefer [`Self::with_hook_dispatcher_factory`],
    /// which mints a fresh dispatcher per host build and isolates
    /// dispatcher-local state (slot-poisoning, registry mutations) per run.
    /// Predicate counter state is the exception: it is tenant-scoped and shared
    /// across runs by design (see [`Self::with_hook_dispatcher_factory`] and
    /// `ironclaw_reborn_composition::hooks`, "Predicate counter scoping").
    pub fn with_hook_dispatcher(self, dispatcher: Arc<HookDispatcher>) -> Self {
        // Single-instance Arc cloning preserves the legacy shared-state shape
        // so existing call sites and tests behave identically. New code paths
        // should reach for `with_hook_dispatcher_factory` instead.
        self.with_hook_dispatcher_factory(move || Arc::clone(&dispatcher))
    }

    /// **DEPRECATED.** Earlier docs claimed this would defer `.build_arc()`
    /// until the host factory finalizes wiring, but the implementation
    /// always called it eagerly and routed through the legacy shared-
    /// dispatcher adapter (serrrfirat P2 #3 on PR #3573). Callers using
    /// this path lost per-run dispatcher isolation and the run-scoped
    /// milestone sink. Use [`Self::with_hook_dispatcher_builder_factory`]
    /// — pass a closure that builds a fresh builder per host — for the
    /// behavior this method's name implied, or
    /// [`Self::with_hook_dispatcher`] if you actually want the shared
    /// dispatcher.
    #[deprecated(
        since = "0.1.0",
        note = "this method always called build_arc() eagerly and routed \
                through the shared-dispatcher adapter, contradicting the \
                doc-claimed deferred semantics. Use \
                `with_hook_dispatcher_builder_factory(|| builder())` for \
                per-build isolation, or `with_hook_dispatcher(...)` if you \
                meant the shared adapter explicitly."
    )]
    pub fn with_hook_dispatcher_builder(self, builder: HookDispatcherBuilder) -> Self {
        self.with_hook_dispatcher(builder.build_arc())
    }

    // Queue ownership follows the same factory used for capability/context
    // ports. PlannedDriver delegates fully to the host for input port
    // construction.
    pub fn with_input_queue(mut self, queue: Arc<dyn HostInputQueue>) -> Self {
        self.input_queue = Some(queue);
        self
    }

    pub fn with_identity_context_source(
        mut self,
        source: Arc<dyn HostIdentityContextSource>,
    ) -> Self {
        self.identity_context_source = Some(source);
        self
    }

    /// Installs the per-user profile source. Resolution starts once at loop
    /// start, but chat prompt construction only consumes it if it is already
    /// available by the prompt-port handoff; otherwise the advisory profile
    /// read continues in the background and the turn proceeds without it. When
    /// not called the factory defaults to `EmptyUserProfileSource`.
    pub fn with_user_profile_source(mut self, source: Arc<dyn HostUserProfileSource>) -> Self {
        self.user_profile_source = source;
        self
    }

    pub fn with_communication_context_provider(
        mut self,
        provider: Arc<dyn CommunicationContextProvider>,
    ) -> Self {
        self.communication_context_provider = Some(provider);
        self
    }

    pub fn with_profiled_capability_port_factory(
        mut self,
        capability_factory: Arc<dyn LoopCapabilityPortFactory>,
        surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    ) -> Self {
        self.profiled_capabilities = Some(ProfiledCapabilityHostRuntime {
            capability_factory,
            surface_resolver,
        });
        self
    }

    pub fn with_subagent_prompt_composer(mut self, composer: SubagentPromptComposer) -> Self {
        self.subagent_prompt_composer = Some(composer);
        self
    }

    pub fn with_driver_requirements(
        mut self,
        driver_requirements: HashMap<LoopDriverRegistryKey, DriverRequirements>,
    ) -> Self {
        self.driver_requirements = driver_requirements;
        self
    }

    pub fn with_model_route_resolver(mut self, resolver: Arc<dyn ModelRouteResolver>) -> Self {
        self.model_route_resolver = Some(resolver);
        self
    }

    pub fn with_model_budget_accountant(
        mut self,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
    ) -> Self {
        self.model_accountant = accountant;
        self
    }

    pub fn with_model_policy_guard(mut self, policy_guard: Arc<dyn LoopModelPolicyGuard>) -> Self {
        self.model_policy_guard = policy_guard;
        self
    }

    pub async fn build_text_only_host(
        &self,
        request: RebornLoopDriverHostRequest,
    ) -> Result<RebornLoopDriverHost, RebornLoopDriverHostError> {
        self.build_text_only_host_with_capabilities(request, Arc::new(EmptyLoopCapabilityPort))
            .await
    }

    pub async fn build_text_only_host_with_profiled_capabilities(
        &self,
        request: RebornLoopDriverHostRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
        surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    ) -> Result<RebornLoopDriverHost, RebornLoopDriverHostError> {
        validate_claimed_run_context(&request.claimed_run, &request.loop_run_context)?;
        validate_thread_scope(
            &self.effective_thread_scope(&request.loop_run_context),
            &request.loop_run_context,
        )?;
        let allow_set = Arc::new(
            surface_resolver
                .resolve(&request.loop_run_context)
                .await
                .map_err(capability_resolve_error_to_host_error)?,
        );
        let capabilities: Arc<dyn LoopCapabilityPort> =
            Arc::new(CapabilitySurfaceProfileFilter::new(capabilities, allow_set));
        self.build_text_only_host_with_capabilities(request, capabilities)
            .await
    }

    pub async fn build_text_only_host_with_capabilities(
        &self,
        request: RebornLoopDriverHostRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<RebornLoopDriverHost, RebornLoopDriverHostError> {
        validate_claimed_run_context(&request.claimed_run, &request.loop_run_context)?;
        // Resolve the per-caller thread scope once; every thread read/write
        // port below uses it so a logged-in user's turn touches only their
        // own `owners/<user>` subtree.
        let effective_scope = self.effective_thread_scope(&request.loop_run_context);
        validate_thread_scope(&effective_scope, &request.loop_run_context)?;

        let max_messages = self.config.max_messages.max(1);
        let run_context = self.attach_model_route_snapshot(request.loop_run_context)?;

        // Kick off advisory communication-context fetches only for origins that
        // can use delivery/channel context. WebUI chat renders its origin without
        // touching the provider, so plain chat prompts avoid unrelated reads.
        let communication_fetch = if should_fetch_communication_context(&run_context) {
            self.communication_context_provider
                .as_ref()
                .map(|provider| {
                    provider.begin_communication_context(
                        run_context.scope.clone(),
                        run_context.actor().cloned(),
                    )
                })
        } else {
            None
        };
        let user_profile_source = Arc::clone(&self.user_profile_source);
        let user_profile_run_context = run_context.clone();
        let user_profile_fetch = tokio::spawn(async move {
            tokio::time::timeout(
                USER_PROFILE_WARMUP_TIMEOUT,
                user_profile_source.resolve_user_profile(&user_profile_run_context),
            )
            .await
            .ok()
            .flatten()
        });
        // Build the live cancellation handle in parallel with the rest of host
        // setup. The claimed run state is already validated by the scheduler, so
        // the turn-state-backed factory can avoid a synchronous durable seed
        // read on the host construction path while still registering for wake
        // flips and the polling fallback.
        let cancellation_factory = Arc::clone(&self.cancellation_factory);
        let cancellation_state = request.claimed_run.state.clone();
        let cancellation_handle_fetch = tokio::spawn(async move {
            cancellation_factory
                .handle_for_claimed_run(&cancellation_state)
                .await
        });

        let context_window_cache = Arc::new(ThreadContextWindowCache::default());
        let mut context_adapter = ThreadBackedLoopContextPort::new(
            Arc::clone(&self.thread_service),
            effective_scope.clone(),
            run_context.clone(),
            max_messages,
        )
        .with_context_window_cache(Arc::clone(&context_window_cache));
        if let Some(source) = self.skill_context_source.as_ref() {
            context_adapter = context_adapter.with_skill_context_source(source.clone());
        }
        if let Some(source) = self.identity_context_source.as_ref() {
            context_adapter = context_adapter.with_identity_context_source(source.clone());
        }
        context_adapter = context_adapter.with_milestone_sink(Arc::clone(&self.milestone_sink));
        let context: Arc<dyn LoopContextPort> = Arc::new(context_adapter);
        // Mint a fresh dispatcher per build when a factory is installed. This
        // localizes dispatcher-owned state (slot poisoning, registry edits) to
        // this one host so it cannot leak into the next run that shares this
        // factory. NOTE: predicate counter state is NOT localized here — the
        // evaluator/backend is captured once per tenant by the composition root
        // and shared across runs by design (tenant-scoped rate/value caps; see
        // `ironclaw_reborn_composition::hooks`, "Predicate counter scoping").
        //
        // Builder-factory path (preferred): the factory hands back a
        // mutable builder; we attach a `RunScopedHookMilestoneSink` keyed
        // to *this* run's `LoopRunContext` *before* sealing. Hook
        // telemetry then carries the correct run/thread scope without
        // depending on the closure capturing the right context (which
        // would silently misattribute across reuses — henrypark133
        // Critical #4).
        let dispatcher_started_at = ironclaw_observability::live_latency_started_at();
        let per_build_dispatcher = match (
            self.hook_dispatcher_builder_factory.as_ref(),
            self.hook_dispatcher_factory.as_ref(),
        ) {
            (Some(builder_factory), _) => {
                let mut builder = builder_factory()?;
                if let Some(sink) = self.hook_security_audit_sink.as_ref() {
                    builder = builder.with_security_audit_sink(Arc::clone(sink));
                }
                let run_scoped: Arc<dyn HookMilestoneSink> =
                    Arc::new(RunScopedHookMilestoneSink::new(
                        run_context.clone(),
                        Arc::clone(&self.milestone_sink) as _,
                    ));
                Some(builder.with_milestone_sink(run_scoped).build_arc())
            }
            (None, Some(factory)) => Some(factory()),
            (None, None) => None,
        };
        trace_host_factory_latency_ok("dispatcher_setup", &run_context, dispatcher_started_at);

        let event_subscription_started_at = ironclaw_observability::live_latency_started_at();
        let event_subscription = match (
            per_build_dispatcher.as_ref(),
            self.event_subscription.as_ref(),
        ) {
            (Some(dispatcher), Some(subscription)) => {
                // NOTE(#3640): bind subscription stream/read-scope to
                // this host's run scope. Otherwise a
                // caller wiring tenant A's host with tenant B's stream
                // would silently dispatch B's hook events into A's
                // dispatcher with A's context tenant. Bug 1 followup: the
                // effective read scope is *derived* from the run/thread scope
                // so the subscription can never observe other threads/projects
                // in the shared stream — the caller's filter is not trusted.
                let effective_read_scope = subscription
                    .effective_read_scope(&run_context.scope, &self.thread_scope)
                    .map_err(|reason| RebornLoopDriverHostError::ScopeMismatch { reason })?;
                Some(
                    subscription
                        .clone_for_independent_spawn()
                        .with_read_scope(effective_read_scope)
                        .spawn(
                            Arc::clone(dispatcher),
                            run_context.scope.tenant_id.clone(),
                            run_context.clone(),
                            Arc::clone(&self.milestone_sink) as _,
                        )
                        .await,
                )
            }
            _ => None,
        };
        trace_host_factory_latency_ok(
            "event_subscription_setup",
            &run_context,
            event_subscription_started_at,
        );
        let instruction_materialization_store: Arc<dyn InstructionMaterializationStore> =
            Arc::new(InMemoryInstructionMaterializationStore::default());
        let surface_state = Arc::new(CapabilitySurfaceState::default());
        let mut capabilities: Arc<dyn LoopCapabilityPort> = Arc::new(
            SurfaceTrackingLoopCapabilityPort::new(capabilities, Arc::clone(&surface_state)),
        );
        if let Some(dispatcher) = per_build_dispatcher.as_ref() {
            // Wire a surface-backed provider resolver so OwnCapabilities-
            // scoped hooks can see `ctx.provider` (henrypark133 Critical #2).
            // Without this, the middleware keeps NullCapabilityProviderResolver
            // and every OwnCapabilities hook is inert because provider stays
            // None.
            let provider_resolver: Arc<dyn HookCapabilityProviderResolver> =
                Arc::new(SurfaceBackedProviderResolver {
                    surface_state: Arc::clone(&surface_state),
                });
            let mut hooked = HookedLoopCapabilityPort::new(
                Arc::clone(&capabilities),
                Arc::clone(dispatcher),
                run_context.scope.tenant_id.clone(),
            )
            .with_provider_resolver(provider_resolver);
            // Prefer the per-build builder (serrrfirat MEDIUM on PR
            // #3633 5-15 review): each host build invokes the closure
            // with its own `LoopRunContext`, so the resulting factory
            // can't capture stale run/actor identity. Fall back to the
            // deprecated shared factory if only the older API is wired.
            let per_build_gate_factory = self
                .hook_gate_ref_factory_builder
                .as_ref()
                .map(|builder| builder(&run_context))
                .or_else(|| self.hook_gate_ref_factory.clone());
            let has_gate_factory = per_build_gate_factory.is_some();
            if let Some(factory) = per_build_gate_factory {
                hooked = hooked.with_gate_ref_factory(factory);
            }
            if let Some(input_resolver) = self.capability_input_resolver.as_ref() {
                let adapter: Arc<dyn HookCapabilityInputResolver> =
                    Arc::new(HookCapabilityInputResolverAdapter::new(
                        Arc::clone(input_resolver),
                        run_context.clone(),
                    ));
                hooked = hooked.with_resolver(adapter);
            }
            let hooked: Arc<dyn LoopCapabilityPort> = Arc::new(hooked);
            capabilities = if has_gate_factory {
                Arc::new(HookGateInvocationScopePort::new(hooked))
            } else {
                hooked
            };
        }
        let visible_surface_started_at = ironclaw_observability::live_latency_started_at();
        let visible_surface_result = capabilities
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: error.safe_summary,
            });
        let visible_surface = match visible_surface_result {
            Ok(surface) => {
                trace_host_factory_latency_ok(
                    "visible_capabilities",
                    &run_context,
                    visible_surface_started_at,
                );
                surface
            }
            Err(error) => {
                trace_host_factory_latency_error(
                    "visible_capabilities",
                    &run_context,
                    visible_surface_started_at,
                    &error,
                );
                return Err(error);
            }
        };
        // The rendered guidance names BOTH the lister and the setter; require both
        // capabilities to be visible before setting the flag, so we never prompt
        // the model to call a tool that is not actually in the surface.
        let delivery_tools_visible = {
            let ids: std::collections::HashSet<&str> = visible_surface
                .descriptors
                .iter()
                .map(|d| d.capability_id.as_str())
                .collect();
            ids.contains("builtin.outbound_delivery_target_set")
                && ids.contains("builtin.outbound_delivery_targets_list")
        };
        // Join the fetch started at loop entry and stamp the surface-derived
        // `delivery_tools_visible` flag. In the common case the fetch has already
        // resolved during the work above, so this adds no critical-path latency.
        let communication_started_at = ironclaw_observability::live_latency_started_at();
        let communication = match communication_fetch {
            Some(fetch) => fetch.resolve(delivery_tools_visible).await,
            None => None,
        };
        trace_host_factory_latency_ok(
            "communication_context",
            &run_context,
            communication_started_at,
        );
        // Use the per-user profile only if the loop-start fetch has already
        // finished. It is advisory context, so cold profile filesystem reads
        // should warm the cache in the background instead of spending any
        // prompt critical-path budget.
        let user_profile_started_at = ironclaw_observability::live_latency_started_at();
        tokio::task::yield_now().await;
        let user_profile = match user_profile_fetch.now_or_never() {
            Some(Ok(profile)) => profile,
            Some(Err(error)) => {
                tracing::debug!(
                    error = %error,
                    "user profile task failed; continuing without profile"
                );
                None
            }
            None => {
                tracing::debug!(
                    "user profile resolution was not ready at prompt handoff; continuing without profile"
                );
                None
            }
        };
        trace_host_factory_latency_ok("user_profile", &run_context, user_profile_started_at);
        let prompt_authority = LoopPromptBundleAuthority::shared();
        let prompt_ports_started_at = ironclaw_observability::live_latency_started_at();
        let surface_state_for_prompt = Arc::clone(&surface_state);
        let prompt_port = HostManagedLoopPromptPort::new(
            run_context.clone(),
            Arc::clone(&context),
            Arc::clone(&self.milestone_sink),
        )
        .with_prompt_bundle_authority(prompt_authority.clone())
        .with_default_message_limit(max_messages)
        .with_current_surface_lookup(move || surface_state_for_prompt.current())
        .with_instruction_materialization_store(Arc::clone(&instruction_materialization_store))
        .with_safety_context(self.safety_context.clone())
        // Stamped once per loop spawn; a resume creates a new host and restamps.
        .with_runtime_context(LoopRuntimeContext {
            loop_started_at_utc: chrono::Utc::now(),
            communication,
            product_context: run_context.product_context.clone(),
            user_profile,
        });
        let mut prompt: Arc<dyn LoopPromptPort> = Arc::new(prompt_port);
        if let Some(dispatcher) = per_build_dispatcher.as_ref() {
            // Pass a sink backed by the host's instruction materialization
            // store so hook-emitted `msg:hook.*` refs are resolvable by the
            // downstream model resolver. Without this the resolver fails
            // the request with `model message reference is unavailable`
            // (henrypark133 review Critical #1).
            let sink: Arc<dyn HookPromptMaterializationSink> =
                Arc::new(InstructionStoreBackedHookSink {
                    store: Arc::clone(&instruction_materialization_store),
                    run_context: run_context.clone(),
                });
            prompt = Arc::new(
                HookedLoopPromptPort::new(
                    Arc::clone(&prompt),
                    Arc::clone(dispatcher),
                    run_context.scope.tenant_id.clone(),
                )
                .with_materialization_sink(sink)
                .with_bundle_authority(prompt_authority.clone(), run_context.clone()),
            );
        }
        trace_host_factory_latency_ok("prompt_ports", &run_context, prompt_ports_started_at);
        if is_subagent_planned_run_profile(&run_context) {
            let Some(composer) = self.subagent_prompt_composer.clone() else {
                return Err(RebornLoopDriverHostError::InvalidRequest {
                    reason: "subagent prompt composer is required for subagent run profile"
                        .to_string(),
                });
            };
            prompt = Arc::new(SubagentLoopPromptPort::new(
                prompt,
                run_context.clone(),
                composer,
            ));
        }
        let input: Arc<dyn LoopInputPort> = match self.input_queue.as_ref() {
            Some(queue) => Arc::new(HostQueueLoopInputPort::new(
                queue.clone(),
                run_context.clone(),
            )),
            None => Arc::new(NoExtraLoopInputPort::new(run_context.clone())),
        };
        // Resolve a scope-specific gateway when available (test harnesses override
        // resolve_for_scope to route per-thread scripted gateways). Production
        // gateways inherit the default-None impl → falls to Arc::clone, byte-identical.
        // Two arms (rather than one literal over a shared gateway binding) because
        // `host_gateway` is the resolved `Arc<dyn _>` in the Some arm but the host's
        // own `Arc<G>` (G: ?Sized, not coercible to `Arc<dyn _>`) in the fallback —
        // see `build_compaction_ports` above. Each arm moves its owned fields.
        let model_gateway_ports_started_at = ironclaw_observability::live_latency_started_at();
        let model_gateway: Arc<dyn LoopModelGateway> =
            if let Some(gw) = self.model_gateway.resolve_for_scope(&run_context.scope) {
                Arc::new(ThreadResolvingLoopModelGateway {
                    thread_service: Arc::clone(&self.thread_service),
                    thread_scope: effective_scope.clone(),
                    host_gateway: gw,
                    max_messages,
                    skill_context_source: self.skill_context_source.clone(),
                    identity_context_source: self.identity_context_source.clone(),
                    instruction_materialization_store: Some(Arc::clone(
                        &instruction_materialization_store,
                    )),
                    capabilities: Some(Arc::clone(&capabilities)),
                    prompt_authority,
                    context_window_cache: Some(context_window_cache),
                    attachment_read_port: self.attachment_read_port.clone(),
                })
            } else {
                Arc::new(ThreadResolvingLoopModelGateway {
                    thread_service: Arc::clone(&self.thread_service),
                    thread_scope: effective_scope.clone(),
                    host_gateway: Arc::clone(&self.model_gateway),
                    max_messages,
                    skill_context_source: self.skill_context_source.clone(),
                    identity_context_source: self.identity_context_source.clone(),
                    instruction_materialization_store: Some(Arc::clone(
                        &instruction_materialization_store,
                    )),
                    capabilities: Some(Arc::clone(&capabilities)),
                    prompt_authority,
                    context_window_cache: Some(context_window_cache),
                    attachment_read_port: self.attachment_read_port.clone(),
                })
            };
        let mut model: Arc<dyn LoopModelPort> = Arc::new(HostManagedLoopModelPort::with_guards(
            run_context.clone(),
            model_gateway,
            Arc::clone(&self.milestone_sink),
            Arc::clone(&self.model_accountant),
            Arc::clone(&self.model_policy_guard),
        ));
        let mut checkpoint: Arc<dyn LoopCheckpointPort> =
            Arc::new(HostManagedLoopCheckpointPort::new(
                run_context.clone(),
                Arc::clone(&self.checkpoint_state_store),
                Arc::clone(&self.loop_checkpoint_store),
                Arc::clone(&self.milestone_sink),
            ));
        let mut transcript: Arc<dyn LoopTranscriptPort> =
            Arc::new(ThreadBackedLoopTranscriptPort::with_milestone_sink(
                Arc::clone(&self.thread_service),
                effective_scope.clone(),
                run_context.clone(),
                Arc::clone(&self.milestone_sink),
            ));
        if let Some(dispatcher) = per_build_dispatcher.as_ref() {
            model = Arc::new(HookedLoopModelPort::new(
                Arc::clone(&model),
                Arc::clone(dispatcher),
                run_context.scope.tenant_id.clone(),
            ));
            transcript = Arc::new(HookedLoopTranscriptPort::new(
                Arc::clone(&transcript),
                Arc::clone(dispatcher),
                run_context.scope.tenant_id.clone(),
            ));
            checkpoint = Arc::new(HookedLoopCheckpointPort::new(
                Arc::clone(&checkpoint),
                Arc::clone(dispatcher),
                run_context.scope.tenant_id.clone(),
            ));
        }
        trace_host_factory_latency_ok(
            "model_checkpoint_transcript_ports",
            &run_context,
            model_gateway_ports_started_at,
        );
        let progress: Arc<dyn LoopProgressPort> = Arc::new(HostManagedLoopProgressPort::new(
            run_context.clone(),
            Arc::clone(&self.milestone_sink),
        ));
        let compaction = self.build_compaction_ports(&run_context);
        let cancellation_started_at = ironclaw_observability::live_latency_started_at();
        let cancellation_handle_result = cancellation_handle_fetch
            .await
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: format!("cancellation handle task failed: {error}"),
            })?
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: error.safe_summary,
            });
        let cancellation_handle = match cancellation_handle_result {
            Ok(handle) => {
                trace_host_factory_latency_ok(
                    "cancellation_handle",
                    &run_context,
                    cancellation_started_at,
                );
                handle
            }
            Err(error) => {
                trace_host_factory_latency_error(
                    "cancellation_handle",
                    &run_context,
                    cancellation_started_at,
                    &error,
                );
                return Err(error);
            }
        };
        let cancellation: Arc<dyn LoopCancellationPort> =
            Arc::new(RunStateLoopCancellationPort::new(cancellation_handle));

        Ok(RebornLoopDriverHost {
            run_context,
            context,
            prompt,
            input,
            model,
            checkpoint,
            capabilities,
            surface_state,
            transcript,
            progress,
            compaction,
            cancellation,
            _event_subscription: event_subscription,
        })
    }

    fn attach_model_route_snapshot(
        &self,
        run_context: LoopRunContext,
    ) -> Result<LoopRunContext, RebornLoopDriverHostError> {
        if let Some(snapshot) = &run_context.resolved_model_route {
            snapshot
                .validate()
                .map_err(|reason| RebornLoopDriverHostError::InvalidRequest { reason })?;
            let Some(resolver) = &self.model_route_resolver else {
                return Err(RebornLoopDriverHostError::InvalidRequest {
                    reason: "model route resolver is required for this host".to_string(),
                });
            };
            let slot = slot_for_model_profile(&run_context)?;
            let route = crate::model_routes::ModelRoute::new(
                snapshot.provider_id.clone(),
                snapshot.model_id.clone(),
            )
            .map_err(model_route_error_to_host_error)?;
            resolver
                .validate_model_route(slot, &route)
                .map_err(model_route_error_to_host_error)?;
            return Ok(run_context);
        }
        let Some(resolver) = &self.model_route_resolver else {
            if self.config.require_model_route_snapshot {
                return Err(RebornLoopDriverHostError::InvalidRequest {
                    reason: "model route resolver is required for this host".to_string(),
                });
            }
            return Ok(run_context);
        };
        let slot = slot_for_model_profile(&run_context)?;
        let snapshot = resolver
            .resolve_model_route(slot)
            .map_err(model_route_error_to_host_error)?;
        Ok(run_context.with_resolved_model_route(snapshot.to_loop_model_route_snapshot()))
    }
}

impl<S, G> TurnRunWakeNotifier for RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    fn notify_queued_run(&self, wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        self.cancellation_factory.notify_run_wake(&wake);
        Ok(())
    }
}

pub struct RebornLoopDriverHost {
    run_context: LoopRunContext,
    context: Arc<dyn LoopContextPort>,
    prompt: Arc<dyn LoopPromptPort>,
    input: Arc<dyn LoopInputPort>,
    model: Arc<dyn LoopModelPort>,
    checkpoint: Arc<dyn LoopCheckpointPort>,
    capabilities: Arc<dyn LoopCapabilityPort>,
    surface_state: Arc<CapabilitySurfaceState>,
    transcript: Arc<dyn LoopTranscriptPort>,
    progress: Arc<dyn LoopProgressPort>,
    compaction: Arc<dyn LoopCompactionPort>,
    cancellation: Arc<dyn LoopCancellationPort>,
    _event_subscription: Option<EventTriggeredHookSubscriptionHandle>,
}

impl fmt::Debug for RebornLoopDriverHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RebornLoopDriverHost")
            .field("scope", &self.run_context.scope)
            .field("turn_id", &self.run_context.turn_id)
            .field("run_id", &self.run_context.run_id)
            .field("loop_driver_id", &self.run_context.loop_driver_id)
            .finish()
    }
}

impl LoopRunInfoPort for RebornLoopDriverHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopCancellationPort for RebornLoopDriverHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        self.cancellation.observe_cancellation()
    }

    async fn cancellation_requested(&self) -> LoopCancellationSignal {
        self.cancellation.cancellation_requested().await
    }
}

#[async_trait]
impl LoopContextPort for RebornLoopDriverHost {
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        self.context.load_loop_context(request).await
    }
}

#[async_trait]
impl LoopPromptPort for RebornLoopDriverHost {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.prompt.build_prompt_bundle(request).await
    }
}

#[async_trait]
impl LoopInputPort for RebornLoopDriverHost {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        self.input.poll_inputs(after, limit).await
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        self.input.ack_inputs(tokens).await
    }
}

#[async_trait]
impl LoopModelPort for RebornLoopDriverHost {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        self.model.stream_model(request).await
    }
}

#[async_trait]
impl LoopCapabilityPort for RebornLoopDriverHost {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.capabilities.tool_definitions()
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.capabilities.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<ironclaw_turns::run_profile::CapabilityCallCandidate, AgentLoopHostError> {
        self.capabilities.register_provider_tool_call(request).await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.capabilities.visible_capabilities(request).await
    }

    fn current_visible_capabilities(
        &self,
    ) -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError> {
        self.surface_state.current()
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.capabilities.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.capabilities.invoke_capability_batch(request).await
    }
}

#[async_trait]
impl LoopTranscriptPort for RebornLoopDriverHost {
    async fn begin_assistant_draft(
        &self,
        request: BeginAssistantDraft,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.begin_assistant_draft(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        self.transcript.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.finalize_assistant_message(request).await
    }

    async fn append_capability_result_ref(
        &self,
        request: AppendCapabilityResultRef,
    ) -> Result<ironclaw_turns::LoopMessageRef, AgentLoopHostError> {
        self.transcript.append_capability_result_ref(request).await
    }
}

#[async_trait]
impl LoopCheckpointPort for RebornLoopDriverHost {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        self.checkpoint.checkpoint(request).await
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        self.checkpoint.stage_checkpoint_payload(request).await
    }

    async fn load_checkpoint_payload(
        &self,
        request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        self.checkpoint.load_checkpoint_payload(request).await
    }
}

#[async_trait]
impl LoopProgressPort for RebornLoopDriverHost {
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError> {
        self.progress.emit_loop_progress(event).await
    }
}

#[async_trait]
impl LoopCompactionPort for RebornLoopDriverHost {
    async fn compact_loop_context(
        &self,
        request: LoopCompactionRequest,
    ) -> Result<LoopCompactionOutcome, LoopCompactionError> {
        self.compaction.compact_loop_context(request).await
    }
}

fn validate_claimed_run_context(
    claimed_run: &ClaimedTurnRun,
    run_context: &LoopRunContext,
) -> Result<(), RebornLoopDriverHostError> {
    if claimed_run.state.status != TurnStatus::Running {
        return Err(RebornLoopDriverHostError::InvalidRequest {
            reason: "claimed run must be running".to_string(),
        });
    }
    if claimed_run.state.scope != run_context.scope
        || claimed_run.state.turn_id != run_context.turn_id
        || claimed_run.state.run_id != run_context.run_id
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run state does not match loop run context".to_string(),
        });
    }
    if claimed_run.resolved_run_profile != run_context.resolved_run_profile {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run profile does not match loop run context".to_string(),
        });
    }
    match (
        &claimed_run.state.resolved_model_route,
        &run_context.resolved_model_route,
    ) {
        (Some(expected), Some(actual)) if expected != actual => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context model route does not match claimed run".to_string(),
            });
        }
        (Some(_), None) => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context is missing claimed run model route".to_string(),
            });
        }
        (None, Some(_)) => {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "loop run context model route was not persisted on claimed run".to_string(),
            });
        }
        _ => {}
    }
    let expected_profile_id = persisted_profile_id(&run_context.resolved_run_profile.profile_id);
    if claimed_run.state.resolved_run_profile_id != expected_profile_id
        || claimed_run.state.resolved_run_profile_version
            != run_context.resolved_run_profile.profile_version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "claimed run persisted profile identity does not match loop run context"
                .to_string(),
        });
    }
    if run_context.loop_driver_id != run_context.resolved_run_profile.loop_driver.id
        || run_context.loop_driver_version != run_context.resolved_run_profile.loop_driver.version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop driver identity does not match resolved profile".to_string(),
        });
    }
    if run_context.thread_id != run_context.scope.thread_id {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop run context thread does not match scope thread".to_string(),
        });
    }
    if run_context.checkpoint_schema_id != run_context.resolved_run_profile.checkpoint_schema_id
        || run_context.checkpoint_schema_version
            != run_context.resolved_run_profile.checkpoint_schema_version
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "loop run context checkpoint identity does not match resolved profile"
                .to_string(),
        });
    }
    Ok(())
}

#[async_trait]
impl<S, G> crate::turn_runner::HostFactory for RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    async fn create_host(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<
        Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>,
        crate::turn_runner::HostFactoryError,
    > {
        let mut loop_run_context = LoopRunContext::new(
            claimed.state.scope.clone(),
            claimed.state.turn_id,
            claimed.state.run_id,
            claimed.resolved_run_profile.clone(),
        )
        .with_accepted_message_ref(claimed.state.accepted_message_ref.clone());
        if let Some(actor) = claimed.state.actor.clone() {
            loop_run_context = loop_run_context.with_actor(actor);
        }
        if let Some(snapshot) = claimed.state.resolved_model_route.clone() {
            loop_run_context = loop_run_context.with_resolved_model_route(snapshot);
        }
        if let Some(product_context) = claimed.state.product_context.clone() {
            loop_run_context = loop_run_context.with_product_context(product_context);
        }
        let request = RebornLoopDriverHostRequest {
            claimed_run: claimed.clone(),
            loop_run_context,
        };
        let capability_requirement = self.capability_requirement(claimed)?;
        let host_result = if capability_requirement.requires_profiled_capabilities() {
            let Some(profiled) = self.profiled_capabilities.as_ref() else {
                return Err(crate::turn_runner::HostFactoryError::new(
                    "profiled capability port factory is required for capability-required driver host",
                ));
            };
            let capabilities = profiled
                .capability_factory
                .create_capability_port(&request.loop_run_context)
                .await
                .map_err(|error| crate::turn_runner::HostFactoryError::new(error.safe_summary))?;
            self.build_text_only_host_with_profiled_capabilities(
                request,
                capabilities,
                Arc::clone(&profiled.surface_resolver),
            )
            .await
        } else {
            self.build_text_only_host(request).await
        };
        host_result
            .map(|host| {
                Box::new(host)
                    as Box<dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync>
            })
            .map_err(|error| crate::turn_runner::HostFactoryError::new(error.to_string()))
    }
}

impl<S, G> RebornLoopDriverHostFactory<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync + 'static,
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    fn capability_requirement(
        &self,
        claimed: &ClaimedTurnRun,
    ) -> Result<DriverCapabilityRequirement, crate::turn_runner::HostFactoryError> {
        let key = LoopDriverRegistryKey::from_descriptor(&claimed.resolved_run_profile.loop_driver)
            .map_err(|reason| {
                crate::turn_runner::HostFactoryError::new(format!(
                    "invalid loop driver descriptor: {reason}"
                ))
            })?;
        let Some(requirements) = self.driver_requirements.get(&key) else {
            // Older text-only factory paths predate driver requirement snapshots.
            // Keep only those known descriptors on the no-capability host path.
            if is_text_only_driver_key(&key) {
                return Ok(DriverCapabilityRequirement::ExplicitlyTextOnly);
            }
            return Err(crate::turn_runner::HostFactoryError::new(
                "loop driver requirements metadata is unavailable; cannot determine capability requirements",
            ));
        };
        Ok(DriverCapabilityRequirement::from_requirements(requirements))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriverCapabilityRequirement {
    ExplicitlyTextOnly,
    ProfiledCapabilitiesRequired,
    ProfiledCapabilitiesNotRequired,
}

impl DriverCapabilityRequirement {
    fn from_requirements(requirements: &DriverRequirements) -> Self {
        if matches!(requirements.capabilities, RequirementLevel::Required) {
            Self::ProfiledCapabilitiesRequired
        } else {
            Self::ProfiledCapabilitiesNotRequired
        }
    }

    fn requires_profiled_capabilities(self) -> bool {
        matches!(self, Self::ProfiledCapabilitiesRequired)
    }
}

fn is_text_only_driver_key(key: &LoopDriverRegistryKey) -> bool {
    is_reborn_text_only_driver_key(key) || is_legacy_text_only_driver_key(key)
}

fn is_reborn_text_only_driver_key(key: &LoopDriverRegistryKey) -> bool {
    key.id.as_str() == TEXT_ONLY_DRIVER_ID
        && key.version.as_u64() == TEXT_ONLY_DRIVER_VERSION
        && key.checkpoint_schema_id.is_none()
        && key.checkpoint_schema_version.is_none()
}

fn is_legacy_text_only_driver_key(key: &LoopDriverRegistryKey) -> bool {
    key.id.as_str() == LEGACY_TEXT_ONLY_DRIVER_ID
        && key.version.as_u64() == LEGACY_TEXT_ONLY_DRIVER_VERSION
        && key
            .checkpoint_schema_id
            .as_ref()
            .is_some_and(|schema_id| schema_id.as_str() == LEGACY_TEXT_ONLY_CHECKPOINT_SCHEMA_ID)
        && key
            .checkpoint_schema_version
            .is_some_and(|version| version.as_u64() == LEGACY_TEXT_ONLY_CHECKPOINT_SCHEMA_VERSION)
}

fn model_route_error_to_host_error(error: ModelRouteError) -> RebornLoopDriverHostError {
    tracing::warn!(
        component = "model_route",
        operation = "resolve_route",
        error = %error,
        error_debug = ?error,
        "model route error mapped to safe host error"
    );
    RebornLoopDriverHostError::InvalidRequest {
        reason: format!("model route resolution failed: {}", error.kind().as_str()),
    }
}

fn capability_resolve_error_to_host_error(
    error: CapabilityResolveError,
) -> RebornLoopDriverHostError {
    tracing::warn!(
        component = "capability_surface_profile",
        operation = "resolve_profile",
        error = %error,
        error_debug = ?error,
        "capability surface resolution error mapped to safe host error"
    );
    let reason = match error {
        CapabilityResolveError::Unavailable { .. } => "capability surface profile is unavailable",
        CapabilityResolveError::Internal { .. } => {
            "capability surface profile could not be resolved"
        }
        _ => "capability surface profile resolution failed",
    };
    RebornLoopDriverHostError::InvalidRequest {
        reason: reason.to_string(),
    }
}

fn slot_for_model_profile(
    run_context: &LoopRunContext,
) -> Result<ModelSlot, RebornLoopDriverHostError> {
    ModelSlot::from_model_profile_id(&run_context.resolved_run_profile.model_profile_id).ok_or_else(
        || RebornLoopDriverHostError::InvalidRequest {
            reason: "model profile is not supported by the model route resolver".to_string(),
        },
    )
}

fn persisted_profile_id(profile_id: &RunProfileId) -> RunProfileId {
    if profile_id.is_interactive_default() {
        RunProfileId::default_profile()
    } else {
        profile_id.clone()
    }
}

fn validate_thread_scope(
    thread_scope: &ThreadScope,
    run_context: &LoopRunContext,
) -> Result<(), RebornLoopDriverHostError> {
    // Reborn text-only hosts currently wrap `ironclaw_threads::ThreadScope`,
    // whose production transcript boundary is agent-scoped. Agentless turn
    // scopes are rejected here until that lower thread boundary grows an
    // explicit agentless thread scope.
    if run_context.scope.agent_id.as_ref() != Some(&thread_scope.agent_id) {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "text-only loop host requires a matching agent-scoped thread".to_string(),
        });
    }
    if thread_scope.tenant_id != run_context.scope.tenant_id
        || thread_scope.project_id != run_context.scope.project_id
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "thread scope does not match loop run scope".to_string(),
        });
    }
    // The thread store keys threads by `owner_user_id` (via the MountView in
    // `ThreadScope::to_resource_scope`), but that axis is not part of the
    // on-disk thread path, so a wrong owner silently reads an empty subtree.
    // Actor-fallback turns still require owner=actor. Explicit-owner turns
    // intentionally allow actor/subject divergence for shared Slack/team
    // routes, but the explicit subject must match the resolved thread owner.
    if run_context.scope.has_explicit_thread_owner() {
        if run_context.scope.explicit_owner_user_id() != thread_scope.owner_user_id.as_ref() {
            return Err(RebornLoopDriverHostError::ScopeMismatch {
                reason: "thread scope owner does not match the explicit loop run subject"
                    .to_string(),
            });
        }
    } else if let (Some(thread_owner), Some(actor)) =
        (thread_scope.owner_user_id.as_ref(), run_context.actor())
        && thread_owner != &actor.user_id
    {
        return Err(RebornLoopDriverHostError::ScopeMismatch {
            reason: "thread scope owner does not match the loop run actor".to_string(),
        });
    }
    Ok(())
}

fn turn_error_to_host_error(error: TurnError) -> AgentLoopHostError {
    match &error {
        TurnError::Unauthorized => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "access",
            AgentLoopHostErrorKind::Unauthorized,
            "checkpoint state access was unauthorized",
            &error,
        ),
        TurnError::InvalidRequest { .. } => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "request",
            AgentLoopHostErrorKind::InvalidInvocation,
            "checkpoint state request is invalid",
            &error,
        ),
        TurnError::Unavailable { .. } => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "store",
            AgentLoopHostErrorKind::Unavailable,
            "checkpoint state store is unavailable",
            &error,
        ),
        TurnError::ScopeNotFound => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "scope_lookup",
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state scope was not found for this loop run",
            &error,
        ),
        TurnError::Conflict { .. } | TurnError::RunNotRetryable { .. } => {
            ironclaw_loop_support::raw_agent_loop_host_error(
                "checkpoint_state",
                "write",
                AgentLoopHostErrorKind::CheckpointRejected,
                "checkpoint state write conflicted with current turn state",
                &error,
            )
        }
        TurnError::CapacityExceeded { .. } => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "write",
            AgentLoopHostErrorKind::Unavailable,
            "checkpoint state store capacity was exceeded",
            &error,
        ),
        TurnError::InvalidTransition { .. } => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "write",
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write was invalid for current turn state",
            &error,
        ),
        TurnError::LeaseMismatch => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "write",
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write lease no longer matches current run",
            &error,
        ),
        TurnError::ThreadBusy(_) | TurnError::AdmissionRejected(_) => {
            ironclaw_loop_support::raw_agent_loop_host_error(
                "checkpoint_state",
                "admission",
                AgentLoopHostErrorKind::Unavailable,
                "checkpoint state store returned unsupported turn admission status",
                &error,
            )
        }
        TurnError::InvalidRunOriginAdapter => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "request",
            AgentLoopHostErrorKind::InvalidInvocation,
            "checkpoint state request contains an invalid run origin adapter",
            &error,
        ),
    }
}

#[cfg(test)]
mod hook_resolver_adapter_tests {
    //! Unit coverage for [`HookCapabilityInputResolverAdapter`]. These tests
    //! drive the adapter directly (not through the factory) so they can
    //! exercise every error branch without standing up a full Reborn host.

    use super::*;
    use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInputRef, CapabilityInvocation,
        CapabilitySurfaceVersion,
    };
    use ironclaw_turns::{
        InMemoryRunProfileResolver, RunProfileResolutionRequest, RunProfileResolver, TurnId,
        TurnRunId, TurnScope,
    };
    use std::sync::Mutex;

    fn tenant() -> TenantId {
        TenantId::new("hook-resolver-tests").expect("tenant id literal valid")
    }

    async fn run_context() -> LoopRunContext {
        let tenant_id = tenant();
        let agent_id = AgentId::new("agent-hook-resolver").expect("agent id literal valid");
        let project_id = ProjectId::new("project-hook-resolver").expect("project id literal valid");
        let thread_id = ThreadId::new("thread-hook-resolver").expect("thread id literal valid");
        let scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("interactive default run profile resolves");
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    fn invocation(input_ref: &str) -> CapabilityInvocation {
        CapabilityInvocation {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: CapabilitySurfaceVersion::new("v1")
                .expect("surface version literal valid"),
            capability_id: CapabilityId::new("cap.test").expect("capability id literal valid"),
            input_ref: CapabilityInputRef::new(input_ref).expect("input ref literal valid"),
            approval_resume: None,
            auth_resume: None,
        }
    }

    /// Test double for [`LoopCapabilityInputResolver`] that returns a queued
    /// `Result` per call; lets us cover both Ok and Err branches.
    struct StubInputResolver {
        responses: Mutex<Vec<Result<serde_json::Value, AgentLoopHostError>>>,
    }

    impl StubInputResolver {
        fn new(responses: Vec<Result<serde_json::Value, AgentLoopHostError>>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl LoopCapabilityInputResolver for StubInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            self.responses
                .lock()
                .expect("stub responses mutex not poisoned")
                .remove(0)
        }
    }

    #[tokio::test]
    async fn adapter_extracts_json_body_when_inner_resolves() {
        let inner = Arc::new(StubInputResolver::new(vec![Ok(serde_json::json!({
            "amount": "50"
        }))]));
        let adapter = HookCapabilityInputResolverAdapter::new(inner, run_context().await);

        let resolved = adapter.resolve(&invocation("input:cap.test")).await;
        let value = resolved.expect("adapter returns Some when inner resolves");
        assert_eq!(value, serde_json::json!({"amount": "50"}));
    }

    #[tokio::test]
    async fn adapter_returns_none_when_inner_errors() {
        let inner = Arc::new(StubInputResolver::new(vec![Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "input ref is unknown",
        ))]));
        let adapter = HookCapabilityInputResolverAdapter::new(inner, run_context().await);

        let resolved = adapter.resolve(&invocation("input:missing")).await;
        assert!(
            resolved.is_none(),
            "adapter must fail closed when inner resolver returns an error"
        );
    }

    #[tokio::test]
    async fn adapter_passes_through_non_object_json_unchanged() {
        // The trait already returns `serde_json::Value`, so "non-JSON-shaped"
        // input cannot reach the adapter — anything the inner returns is
        // already typed JSON. The fail-closed case for non-JSON bodies lives
        // in the inner resolver's implementation (it surfaces an
        // `AgentLoopHostError` on decode failure, covered by the
        // `returns_none_when_inner_errors` test above). This test pins down
        // the adapter's contract for non-object inputs: it must forward them
        // verbatim so predicate evaluators see the raw shape and decide for
        // themselves whether to fail closed.
        let inner = Arc::new(StubInputResolver::new(vec![Ok(serde_json::Value::String(
            "not-an-object".to_string(),
        ))]));
        let adapter = HookCapabilityInputResolverAdapter::new(inner, run_context().await);

        let resolved = adapter.resolve(&invocation("input:non-object")).await;
        assert_eq!(
            resolved,
            Some(serde_json::Value::String("not-an-object".to_string())),
            "adapter forwards non-object JSON verbatim"
        );
    }

    #[tokio::test]
    async fn adapter_returns_none_when_body_exceeds_byte_budget() {
        // Build a value whose serialized form deliberately exceeds the
        // adapter's configured budget. The hooks crate's
        // `SanitizedArguments::from_json` caps individual string lengths and
        // nesting depth, but does not bound total payload bytes — this guard
        // is the adapter's defense in depth.
        let oversized_string = "x".repeat(2048);
        let inner = Arc::new(StubInputResolver::new(vec![Ok(serde_json::json!({
            "blob": oversized_string,
        }))]));
        let adapter = HookCapabilityInputResolverAdapter::new(inner, run_context().await)
            .with_max_input_bytes(512);

        let resolved = adapter.resolve(&invocation("input:oversized")).await;
        assert!(
            resolved.is_none(),
            "adapter must refuse payloads above the configured byte budget"
        );
    }
}

#[cfg(test)]
#[path = "loop_driver_host/tests.rs"]
mod port_adapter_tests;

#[cfg(test)]
#[path = "loop_driver_host/compaction_tests.rs"]
mod compaction_tests;

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_turns::{
        InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryRunProfileResolver,
        PutLoopCheckpointRequest, RunProfileResolver, TurnActor, TurnCheckpointId, TurnId,
        TurnRunId, TurnScope,
        run_profile::{
            AgentLoopHostErrorKind, CheckpointSchemaId, InMemoryLoopHostMilestoneSink,
            LoadCheckpointPayloadRequest, LoopCheckpointKind, LoopCheckpointRequest,
            LoopRunContext, RunProfileResolutionRequest, StageCheckpointPayloadRequest,
        },
    };

    async fn test_run_context() -> LoopRunContext {
        let tenant_id = TenantId::new("tenant-surf-prompt-test").unwrap();
        let agent_id = AgentId::new("agent-surf-prompt-test").unwrap();
        let project_id = ProjectId::new("project-surf-prompt-test").unwrap();
        let thread_id = ThreadId::new("thread-surf-prompt-test").unwrap();
        let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    #[tokio::test]
    async fn owner_scoped_thread_scope_isolates_each_caller() {
        let base = ThreadScope {
            tenant_id: TenantId::new("tenant-scope-test").unwrap(),
            agent_id: AgentId::new("agent-scope-test").unwrap(),
            project_id: None,
            owner_user_id: Some(UserId::new("operator").unwrap()),
            mission_id: None,
        };

        // This drives the resolution exactly as the host does — through
        // `run_context.actor()` — so it locks the host's call pattern, not
        // just the resolver rule (which has its own tests in
        // `crate::thread_scope`).
        use crate::thread_scope::ThreadScopeResolver;

        // No actor → the factory's owner is kept (background / system runs).
        let ctx = test_run_context().await;
        assert_eq!(
            ThreadScopeResolver::resolve(&base, ctx.actor()).owner_user_id,
            Some(UserId::new("operator").unwrap()),
        );

        // Distinct actors → distinct owners: this is the per-caller thread
        // isolation. user-a's run can only ever touch owners/user-a.
        let ctx_a = test_run_context()
            .await
            .with_actor(TurnActor::new(UserId::new("user-a").unwrap()));
        let ctx_b = test_run_context()
            .await
            .with_actor(TurnActor::new(UserId::new("user-b").unwrap()));
        assert_eq!(
            ThreadScopeResolver::resolve(&base, ctx_a.actor()).owner_user_id,
            Some(UserId::new("user-a").unwrap()),
        );
        assert_eq!(
            ThreadScopeResolver::resolve(&base, ctx_b.actor()).owner_user_id,
            Some(UserId::new("user-b").unwrap()),
        );

        // Owner-less base → unchanged even with an actor (shared/system slot).
        let ownerless = ThreadScope {
            owner_user_id: None,
            ..base.clone()
        };
        assert_eq!(
            ThreadScopeResolver::resolve(&ownerless, ctx_a.actor()).owner_user_id,
            None,
        );
    }

    fn test_checkpoint_port(
        context: LoopRunContext,
    ) -> (
        HostManagedLoopCheckpointPort,
        Arc<InMemoryCheckpointStateStore>,
        Arc<InMemoryLoopCheckpointStore>,
    ) {
        let state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
        let port = HostManagedLoopCheckpointPort::new(
            context,
            state_store.clone(),
            checkpoint_store.clone(),
            milestone_sink,
        );
        (port, state_store, checkpoint_store)
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_roundtrips_staged_payload() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let expected_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
        let payload = br#"{"iteration":3}"#.to_vec();

        let state_ref = port
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: LoopCheckpointKind::BeforeSideEffect,
                schema_id: expected_schema_id.as_str().to_string(),
                payload: payload.clone(),
            })
            .await
            .expect("stage checkpoint payload");
        let checkpoint_id = port
            .checkpoint(LoopCheckpointRequest {
                kind: LoopCheckpointKind::BeforeSideEffect,
                state_ref,
                gate_ref: None,
            })
            .await
            .expect("write checkpoint metadata");

        let loaded = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id,
                expected_schema_id: expected_schema_id.clone(),
                expected_schema_version,
            })
            .await
            .expect("load checkpoint payload");

        assert_eq!(loaded.kind, LoopCheckpointKind::BeforeSideEffect);
        assert_eq!(loaded.schema_id, expected_schema_id);
        assert_eq!(loaded.schema_version, expected_schema_version);
        assert_eq!(loaded.payload.as_bytes(), payload.as_slice());
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_rejects_schema_mismatch() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let expected_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
        let state_ref = port
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: LoopCheckpointKind::BeforeModel,
                schema_id: expected_schema_id.as_str().to_string(),
                payload: b"{}".to_vec(),
            })
            .await
            .expect("stage checkpoint payload");
        let checkpoint_id = port
            .checkpoint(LoopCheckpointRequest {
                kind: LoopCheckpointKind::BeforeModel,
                state_ref,
                gate_ref: None,
            })
            .await
            .expect("write checkpoint metadata");

        let error = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id,
                expected_schema_id: CheckpointSchemaId::new("different_checkpoint_schema")
                    .expect("valid schema"),
                expected_schema_version,
            })
            .await
            .expect_err("schema mismatch must reject");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Invalid);
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_rejects_schema_version_mismatch() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let stored_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);
        let state_ref = port
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: LoopCheckpointKind::BeforeModel,
                schema_id: expected_schema_id.as_str().to_string(),
                payload: b"{}".to_vec(),
            })
            .await
            .expect("stage checkpoint payload");
        let checkpoint_id = port
            .checkpoint(LoopCheckpointRequest {
                kind: LoopCheckpointKind::BeforeModel,
                state_ref,
                gate_ref: None,
            })
            .await
            .expect("write checkpoint metadata");

        // Load with a bumped schema version — stored = N, expected = N+1.
        let bumped_version =
            ironclaw_turns::RunProfileVersion::new(stored_schema_version.as_u64() + 1);

        let error = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id,
                expected_schema_id,
                expected_schema_version: bumped_version,
            })
            .await
            .expect_err("schema version mismatch must reject");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Invalid);
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_missing_metadata_is_unavailable() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let expected_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, _checkpoint_store) = test_checkpoint_port(context);

        let error = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id: TurnCheckpointId::new(),
                expected_schema_id,
                expected_schema_version,
            })
            .await
            .expect_err("missing metadata must reject");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    #[tokio::test]
    async fn checkpoint_port_load_payload_missing_state_record_is_unavailable() {
        let context = test_run_context().await;
        let expected_schema_id = context.checkpoint_schema_id.clone();
        let expected_schema_version = context.checkpoint_schema_version;
        let (port, _state_store, checkpoint_store) = test_checkpoint_port(context.clone());
        let missing_state_ref =
            LoopCheckpointStateRef::for_run(&context, "missing-state").expect("valid ref");
        let metadata = checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: context.scope.clone(),
                turn_id: context.turn_id,
                run_id: context.run_id,
                state_ref: missing_state_ref,
                schema_id: expected_schema_id.clone(),
                schema_version: expected_schema_version,
                kind: LoopCheckpointKind::BeforeBlock,
                gate_ref: None,
            })
            .await
            .expect("write checkpoint metadata");

        let error = port
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id: metadata.checkpoint_id,
                expected_schema_id,
                expected_schema_version,
            })
            .await
            .expect_err("missing state payload must reject");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    #[test]
    fn adaptive_poll_interval_doubles_per_streak_until_cap() {
        // PR #3640 finding C5: empty-poll backoff should double up to a
        // configurable cap. Streak 0 returns the base; each subsequent
        // empty poll doubles until clamped.
        let base = Duration::from_millis(50);
        let cap = Duration::from_millis(1_000);

        assert_eq!(
            adaptive_poll_interval(base, cap, 0),
            Duration::from_millis(50)
        );
        assert_eq!(
            adaptive_poll_interval(base, cap, 1),
            Duration::from_millis(100)
        );
        assert_eq!(
            adaptive_poll_interval(base, cap, 2),
            Duration::from_millis(200)
        );
        assert_eq!(
            adaptive_poll_interval(base, cap, 3),
            Duration::from_millis(400)
        );
        assert_eq!(
            adaptive_poll_interval(base, cap, 4),
            Duration::from_millis(800)
        );
        // Streak 5 would compute 1600ms but the cap clamps it to 1000ms.
        assert_eq!(adaptive_poll_interval(base, cap, 5), cap);
        assert_eq!(adaptive_poll_interval(base, cap, 100), cap);
        // Huge streak must not overflow.
        assert_eq!(adaptive_poll_interval(base, cap, u32::MAX), cap);
    }

    #[test]
    fn adaptive_poll_interval_respects_cap_below_base() {
        // If `max < base` somehow slips through (it shouldn't — `with_*`
        // builders normalize — but defense in depth), the function still
        // returns the cap rather than overshooting.
        let base = Duration::from_millis(200);
        let cap = Duration::from_millis(50);
        assert_eq!(adaptive_poll_interval(base, cap, 0), cap);
        assert_eq!(adaptive_poll_interval(base, cap, 10), cap);
    }
}

/// PR #3640 followup (Bug 1, cross-thread/project event leakage via permissive
/// ReadScope). Unit coverage for [`EventTriggeredHookSubscription::effective_read_scope`].
#[cfg(test)]
mod event_subscription_scope_tests {
    use super::*;
    use ironclaw_events::{InMemoryDurableEventLog, RuntimeEvent};
    use ironclaw_host_api::{
        AgentId, CapabilityId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
    };
    use ironclaw_threads::ThreadScope;
    use ironclaw_turns::TurnScope;

    fn ids() -> (TenantId, AgentId, UserId, ProjectId, ThreadId) {
        (
            TenantId::new("tenant-bug1").expect("tenant"),
            AgentId::new("agent-bug1").expect("agent"),
            UserId::new("user-bug1").expect("user"),
            ProjectId::new("project-bug1").expect("project"),
            ThreadId::new("thread-bug1").expect("thread"),
        )
    }

    fn run_scope(
        tenant: &TenantId,
        agent: &AgentId,
        project: &ProjectId,
        thread: &ThreadId,
    ) -> TurnScope {
        TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            Some(project.clone()),
            thread.clone(),
        )
    }

    fn thread_scope(
        tenant: &TenantId,
        agent: &AgentId,
        user: &UserId,
        project: &ProjectId,
    ) -> ThreadScope {
        ThreadScope {
            tenant_id: tenant.clone(),
            agent_id: agent.clone(),
            project_id: Some(project.clone()),
            owner_user_id: Some(user.clone()),
            mission_id: None,
        }
    }

    fn stream_key(tenant: &TenantId, user: &UserId, agent: &AgentId) -> EventStreamKey {
        EventStreamKey::new(tenant.clone(), user.clone(), Some(agent.clone()))
    }

    fn subscription(
        stream: EventStreamKey,
        read_scope: ReadScope,
    ) -> EventTriggeredHookSubscription {
        let log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
        EventTriggeredHookSubscription::new(log, stream, read_scope, EventCursor::origin())
    }

    /// PR #3931 followup: `ReadScope::any()` is no longer rejected. A
    /// permissive caller filter is accepted, but the *derived* scope (the only
    /// scope ever used for the read) still pins the authoritative
    /// thread/project from the run/thread scope — so `any()` cannot widen the
    /// read or reopen the cross-thread/project leak.
    #[test]
    fn event_subscription_any_scope_is_constrained_by_derived_scope() {
        let (tenant, agent, user, project, thread) = ids();
        let sub = subscription(stream_key(&tenant, &user, &agent), ReadScope::any());
        let effective = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect("ReadScope::any() is accepted; the derived scope constrains the read");
        // The permissive `any()` does NOT pass through — the derivation pins
        // the authoritative thread/project regardless of the caller's filter.
        assert_eq!(effective.thread_id, Some(thread));
        assert_eq!(effective.project_id, Some(project));
        assert_ne!(
            effective,
            ReadScope::any(),
            "derived scope must not remain permissive"
        );
    }

    #[test]
    fn effective_read_scope_pins_thread_and_project_from_run_scope() {
        let (tenant, agent, user, project, thread) = ids();
        // Caller supplies a thread-pinned (non-any) scope.
        let supplied = ReadScope {
            thread_id: Some(thread.clone()),
            ..ReadScope::any()
        };
        let sub = subscription(stream_key(&tenant, &user, &agent), supplied);
        let effective = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect("valid scope derives");
        assert_eq!(effective.thread_id, Some(thread));
        assert_eq!(effective.project_id, Some(project));
        assert_eq!(effective.mission_id, None);
    }

    #[test]
    fn effective_read_scope_rejects_tenant_mismatch() {
        let (tenant, agent, user, project, thread) = ids();
        let other_tenant = TenantId::new("tenant-OTHER").expect("tenant");
        let sub = subscription(stream_key(&other_tenant, &user, &agent), ReadScope::any());

        let err = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect_err("tenant mismatch must be rejected");

        assert!(
            err.contains("tenant_id"),
            "err names the tenant axis: {err}"
        );
    }

    #[test]
    fn effective_read_scope_rejects_agent_mismatch() {
        let (tenant, agent, user, project, thread) = ids();
        let other_agent = AgentId::new("agent-OTHER").expect("agent");
        let sub = subscription(stream_key(&tenant, &user, &other_agent), ReadScope::any());

        let err = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect_err("agent mismatch must be rejected");

        assert!(err.contains("agent_id"), "err names the agent axis: {err}");
    }

    #[test]
    fn effective_read_scope_rejects_thread_without_owner() {
        let (tenant, agent, user, project, thread) = ids();
        let mut owner_less = thread_scope(&tenant, &agent, &user, &project);
        owner_less.owner_user_id = None;
        let sub = subscription(stream_key(&tenant, &user, &agent), ReadScope::any());

        let err = sub
            .effective_read_scope(&run_scope(&tenant, &agent, &project, &thread), &owner_less)
            .expect_err("thread without owner must be rejected");

        assert!(
            err.contains("without an owner"),
            "err names the missing owner: {err}"
        );
    }

    #[test]
    fn effective_read_scope_rejects_user_mismatch() {
        let (tenant, agent, user, project, thread) = ids();
        let other_user = UserId::new("user-OTHER").expect("user");
        let sub = subscription(stream_key(&tenant, &other_user, &agent), ReadScope::any());

        let err = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect_err("user mismatch must be rejected");

        assert!(err.contains("user_id"), "err names the user axis: {err}");
    }

    #[test]
    fn effective_read_scope_rejects_project_widening() {
        let (tenant, agent, user, project, thread) = ids();
        let other_project = ProjectId::new("project-OTHER").expect("project");
        let supplied = ReadScope {
            project_id: Some(other_project),
            ..ReadScope::any()
        };
        let sub = subscription(stream_key(&tenant, &user, &agent), supplied);

        let err = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect_err("divergent explicit project_id must be rejected");

        assert!(
            err.contains("project_id"),
            "err names the project axis: {err}"
        );
    }

    #[test]
    fn effective_read_scope_rejects_mission_widening() {
        let (tenant, agent, user, project, thread) = ids();
        let foreign_mission = ironclaw_host_api::MissionId::new("mission-OTHER").expect("mission");
        let supplied = ReadScope {
            mission_id: Some(foreign_mission),
            ..ReadScope::any()
        };
        let sub = subscription(stream_key(&tenant, &user, &agent), supplied);

        let err = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect_err("divergent explicit mission_id must be rejected");

        assert!(
            err.contains("mission_id"),
            "err names the mission axis: {err}"
        );
    }

    #[test]
    fn effective_read_scope_rejects_thread_widening() {
        let (tenant, agent, user, project, thread) = ids();
        let other_thread = ThreadId::new("thread-OTHER").expect("thread");
        let supplied = ReadScope {
            thread_id: Some(other_thread),
            ..ReadScope::any()
        };
        let sub = subscription(stream_key(&tenant, &user, &agent), supplied);

        let err = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect_err("divergent explicit thread_id must be rejected");

        assert!(
            err.contains("thread_id"),
            "err names the thread axis: {err}"
        );
    }

    /// End-to-end leakage proof: the derived effective scope, applied by the
    /// durable log's read filter, must hide events from a *different* project
    /// that share the same `(tenant, user, agent)` stream.
    #[tokio::test]
    async fn event_subscription_does_not_observe_other_project_events() {
        let (tenant, agent, user, project, thread) = ids();
        let other_project = ProjectId::new("project-OTHER").expect("project");

        let our_scope = ResourceScope {
            tenant_id: tenant.clone(),
            user_id: user.clone(),
            agent_id: Some(agent.clone()),
            project_id: Some(project.clone()),
            mission_id: None,
            thread_id: Some(thread.clone()),
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let foreign_scope = ResourceScope {
            project_id: Some(other_project.clone()),
            invocation_id: ironclaw_host_api::InvocationId::new(),
            ..our_scope.clone()
        };

        let log = InMemoryDurableEventLog::new();
        let cap = CapabilityId::new("bug1.cap").expect("cap");
        // Foreign-project event appended first.
        log.append(RuntimeEvent::hook_failed(
            foreign_scope,
            cap.clone(),
            "deadbeef",
            "panic",
            "fail_isolated",
            None,
        ))
        .await
        .expect("append foreign");
        // Our-project event.
        log.append(RuntimeEvent::hook_failed(
            our_scope.clone(),
            cap,
            "deadbeef",
            "panic",
            "fail_isolated",
            None,
        ))
        .await
        .expect("append ours");

        let supplied = ReadScope {
            thread_id: Some(thread.clone()),
            ..ReadScope::any()
        };
        let sub = subscription(EventStreamKey::from_scope(&our_scope), supplied);
        let effective = sub
            .effective_read_scope(
                &run_scope(&tenant, &agent, &project, &thread),
                &thread_scope(&tenant, &agent, &user, &project),
            )
            .expect("derive effective scope");

        let replay = log
            .read_after_cursor(
                &EventStreamKey::from_scope(&our_scope),
                &effective,
                Some(EventCursor::origin()),
                16,
            )
            .await
            .expect("read with effective scope");

        assert_eq!(
            replay.entries.len(),
            1,
            "effective scope must observe only our project's event, not the foreign one"
        );
        assert_eq!(
            replay.entries[0].record.scope.project_id.as_ref(),
            Some(&project),
            "the single observed event must belong to our project"
        );
    }

    /// A `DurableEventLog` whose `head_cursor` always fails. Used to exercise
    /// the fail-closed subscription-spawn path.
    struct HeadCursorFailingLog;

    #[async_trait::async_trait]
    impl DurableEventLog for HeadCursorFailingLog {
        async fn append(
            &self,
            _event: RuntimeEvent,
        ) -> Result<ironclaw_events::EventLogEntry<RuntimeEvent>, ironclaw_events::EventError>
        {
            unreachable!("append is not exercised by the spawn-failure test")
        }

        async fn read_after_cursor(
            &self,
            _stream: &EventStreamKey,
            _filter: &ReadScope,
            _after: Option<EventCursor>,
            _limit: usize,
        ) -> Result<ironclaw_events::EventReplay<RuntimeEvent>, ironclaw_events::EventError>
        {
            unreachable!("read_after_cursor is not reached once head_cursor fails")
        }

        async fn head_cursor(
            &self,
            _stream: &EventStreamKey,
            _after: EventCursor,
        ) -> Result<EventCursor, ironclaw_events::EventError> {
            Err(ironclaw_events::EventError::DurableLog {
                reason: "stub head_cursor failure".to_string(),
            })
        }
    }

    /// PR #3931 (Hole 1) fail-closed contract: when `head_cursor` fails at
    /// subscription start the host cannot classify replay vs live, so it must
    /// not silently disappear. It spawns a task that emits the operator-visible
    /// `EventSubscriptionTerminated` milestone and returns a dead handle that
    /// never dispatches.
    #[tokio::test]
    async fn event_triggered_head_cursor_failure_emits_terminated_milestone() {
        use ironclaw_turns::run_profile::{
            InMemoryLoopHostMilestoneSink, InMemoryRunProfileResolver, LoopDriverNoteKind,
            LoopHostMilestoneKind, LoopRunContext, RunProfileResolutionRequest, RunProfileResolver,
        };
        use ironclaw_turns::{TurnId, TurnRunId};

        let (tenant, agent, user, project, thread) = ids();
        let sub = EventTriggeredHookSubscription::new(
            Arc::new(HeadCursorFailingLog),
            stream_key(&tenant, &user, &agent),
            ReadScope::any(),
            EventCursor::origin(),
        );

        let dispatcher = ironclaw_hooks::dispatch::HookDispatcherBuilder::new(
            ironclaw_hooks::HookRegistry::new(),
        )
        .build_arc();

        let turn_scope = run_scope(&tenant, &agent, &project, &thread);
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("resolve run profile");
        let run_context =
            LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved);
        let _ = &user;

        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
        let sink: Arc<dyn LoopHostMilestoneSink> = milestone_sink.clone();

        // Keep the handle alive: its `Drop` aborts the spawned task, so the
        // terminated-note task must finish before `_handle` is dropped.
        let _handle = sub
            .spawn(dispatcher, tenant.clone(), run_context, sink)
            .await;

        // The terminated note is emitted from a spawned task; yield until it
        // lands (bounded) rather than racing the assertion.
        let mut milestones = milestone_sink.milestones();
        for _ in 0..1000 {
            if !milestones.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
            milestones = milestone_sink.milestones();
        }
        assert_eq!(
            milestones.len(),
            1,
            "fail-closed spawn must emit exactly one milestone, got {milestones:?}"
        );
        assert!(
            matches!(
                &milestones[0].kind,
                LoopHostMilestoneKind::DriverNote {
                    kind: LoopDriverNoteKind::EventSubscriptionTerminated,
                    ..
                }
            ),
            "expected EventSubscriptionTerminated milestone, got {:?}",
            milestones[0].kind
        );
    }
}

#[cfg(test)]
mod turn_error_to_host_error_tests {
    use super::*;
    use ironclaw_turns::{TurnCapacityResource, TurnError, TurnRunId};

    #[test]
    fn capacity_exceeded_maps_to_unavailable() {
        let error = turn_error_to_host_error(TurnError::capacity_exceeded(
            TurnCapacityResource::SpawnTreeDescendants,
            3,
        ));
        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    #[test]
    fn conflict_maps_to_checkpoint_rejected() {
        let error = turn_error_to_host_error(TurnError::Conflict {
            reason: "checkpoint conflict".to_string(),
        });
        assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
    }

    #[test]
    fn run_not_retryable_maps_to_checkpoint_rejected() {
        let error = turn_error_to_host_error(TurnError::RunNotRetryable {
            run_id: TurnRunId::new(),
        });
        assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
    }

    #[test]
    fn scope_not_found_maps_to_checkpoint_rejected() {
        let error = turn_error_to_host_error(TurnError::ScopeNotFound);
        assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
    }

    #[test]
    fn invalid_transition_maps_to_checkpoint_rejected() {
        use ironclaw_turns::TurnStatus;
        let error = turn_error_to_host_error(TurnError::InvalidTransition {
            from: TurnStatus::Running,
            to: TurnStatus::Completed,
        });
        assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
    }
}
