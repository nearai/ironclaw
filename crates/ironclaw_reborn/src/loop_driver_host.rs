use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use futures_util::FutureExt;
use ironclaw_events::{DurableEventLog, EventCursor, EventStreamKey, ReadScope};
use ironclaw_hooks::dispatch::{HookDispatcher, HookDispatcherBuilder};
use ironclaw_hooks::middleware::{
    CapabilityInputResolver as HookCapabilityInputResolver,
    CapabilityProviderResolver as HookCapabilityProviderResolver, HookPromptMaterializationSink,
    HookedLoopCapabilityPort, HookedLoopCheckpointPort, HookedLoopModelPort, HookedLoopPromptPort,
    HookedLoopTranscriptPort,
};
use ironclaw_host_api::ExtensionId;
use ironclaw_loop_support::{
    CapabilityResolveError, CapabilitySurfaceProfileFilter, CapabilitySurfaceProfileResolver,
    EmptyLoopCapabilityPort, GuardedSystemInferencePort, HostIdentityContextSource, HostInputQueue,
    HostManagedModelGateway, HostQueueLoopInputPort, HostSkillContextSource,
    LoopCapabilityInputResolver, ModelGatewayBackedSystemInferencePort, RunCancellationFactory,
    RunCancellationObservationKind, RunStateLoopCancellationPort, SubagentLoopPromptPort,
    SubagentPromptComposer, ThreadBackedLoopContextPort, ThreadBackedLoopTranscriptPort,
    TurnStateRunCancellationFactory, default_host_managed_loop_compaction_port,
};
use ironclaw_threads::{SessionThreadService, ThreadScope};

use crate::driver_registry::{DriverRequirements, LoopDriverRegistryKey, RequirementLevel};
use crate::hook_gate_refs::HookGateInvocationScopePort;
use crate::model_routes::{ModelRouteError, ModelRouteResolver, ModelSlot};
use crate::planned_driver_factory::SUBAGENT_PLANNED_PROFILE_ID;
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

use ironclaw_turns::{
    CheckpointStateStore, LoopCheckpointStateRef, LoopCheckpointStore, RunProfileId,
    TurnCheckpointId, TurnError, TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError,
    TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef, BeginAssistantDraft,
        CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityInvocation, CapabilityOutcome,
        FinalizeAssistantMessage, HookMilestoneSink, HostManagedLoopModelPort,
        HostManagedLoopPromptPort, InMemoryInstructionMaterializationStore,
        InstructionBundleMaterializedMessage, InstructionMaterializationStore,
        InstructionSafetyContext, LoadCheckpointPayloadRequest, LoadedCheckpointPayload,
        LoopCancellationPort, LoopCancellationSignal, LoopCapabilityPort, LoopCheckpointPort,
        LoopCheckpointRequest, LoopCompactionError, LoopCompactionPort, LoopCompactionRequest,
        LoopCompactionResponse, LoopContextBundle, LoopContextPort, LoopContextRequest,
        LoopHostMilestoneSink, LoopInputAckToken, LoopInputBatch, LoopInputCursor, LoopInputPort,
        LoopModelBudgetAccountant, LoopModelPolicyGuard, LoopModelPort, LoopModelRequest,
        LoopModelResponse, LoopProgressEvent, LoopProgressPort, LoopPromptBundle,
        LoopPromptBundleAuthority, LoopPromptBundleRequest, LoopPromptPort, LoopRunContext,
        LoopRunInfoPort, LoopTranscriptPort, NoOpBudgetAccountant, NoOpPolicyGuard,
        ProviderToolCall, ProviderToolDefinition, RunScopedHookMilestoneSink,
        StageCheckpointPayloadRequest, SystemInferencePort, UpdateAssistantDraft,
        VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
    runner::ClaimedTurnRun,
};
use tokio::task::JoinHandle;

#[async_trait]
pub trait LoopCapabilityPortFactory: Send + Sync {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError>;
}

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

#[async_trait]
impl LoopCapabilityPort for SurfaceTrackingLoopCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.inner.tool_definitions()
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<ironclaw_turns::run_profile::CapabilityCallCandidate, AgentLoopHostError> {
        self.inner.register_provider_tool_call(tool_call).await
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
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.inner.invoke_capability_batch(request).await
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

pub type HookDispatcherBuilderFactory =
    Arc<dyn Fn() -> HookDispatcherBuilder + Send + Sync + 'static>;

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

    /// Verify that this subscription's stream key and read-scope filter are
    /// consistent with the host's run scope. The subscription stream
    /// partitions events by `(tenant, user, agent)`; `ReadScope` filters by
    /// `(project, mission, thread, process)`. If a caller wires a host for
    /// tenant A but supplies a subscription pointing at tenant B's stream
    /// (or to a stream-key that names a foreign user), the host would
    /// dispatch B's hook events into A's dispatcher with A's context tenant
    /// — a cross-tenant trust-boundary break (NOTE(#3640)). Same for
    /// `ReadScope` filter dimensions: any `Some(want)` in
    /// the filter must match the corresponding value in the run scope, so
    /// the subscription never observes events outside the host's authority.
    fn validate_against_run_scope(
        &self,
        run_scope: &ironclaw_turns::TurnScope,
        thread_scope: &ironclaw_threads::ThreadScope,
    ) -> Result<(), String> {
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
        // ReadScope tightens the stream; any Some(want) must equal the
        // corresponding run/thread scope value. None is permissive and is
        // acceptable (the run scope owns the dimension authoritatively).
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
        Ok(())
    }

    fn spawn(
        self,
        dispatcher: Arc<HookDispatcher>,
        tenant_id: ironclaw_host_api::TenantId,
        run_context: LoopRunContext,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> EventTriggeredHookSubscriptionHandle {
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
                    for entry in replay.entries {
                        dispatcher
                            .dispatch_event_triggered_at(
                                tenant_id.clone(),
                                entry.cursor,
                                &entry.record,
                            )
                            .await;
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
    /// Optional hook dispatcher factory. When set, the factory invokes the
    /// closure on every `build_text_only_host*` call to obtain a fresh
    /// `HookDispatcher`, wraps it in `Arc`, and then plumbs it through
    /// `HookedLoopCapabilityPort` / `HookedLoopPromptPort`. Building a fresh
    /// dispatcher per host build means slot-poisoning state, the per-tenant
    /// predicate counter, and any registry mutations done while a run is
    /// active do not leak into the next run. Default behavior (no factory) is
    /// unchanged from the pre-hooks shape.
    hook_dispatcher_factory: Option<HookDispatcherFactory>,
    /// Per-build builder factory. Preferred over `hook_dispatcher_factory`
    /// because it lets the host factory attach the run-scoped milestone
    /// sink internally (henrypark133 Critical #4). Exactly one of these
    /// two should be set; if both are, the builder factory wins.
    hook_dispatcher_builder_factory: Option<HookDispatcherBuilderFactory>,
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
    safety_context: Option<InstructionSafetyContext>,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
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
            hook_dispatcher_factory: None,
            hook_dispatcher_builder_factory: None,
            capability_input_resolver: None,
            hook_gate_ref_factory: None,
            hook_gate_ref_factory_builder: None,
            event_subscription: None,
            safety_context: None,
            identity_context_source: None,
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

    fn build_compaction_ports(&self, run_context: &LoopRunContext) -> Arc<dyn LoopCompactionPort> {
        let direct_system_inference: Arc<dyn SystemInferencePort> =
            Arc::new(ModelGatewayBackedSystemInferencePort::new(
                Arc::clone(&self.model_gateway),
                run_context.clone(),
            ));
        let system_inference: Arc<dyn SystemInferencePort> =
            Arc::new(GuardedSystemInferencePort::new(
                direct_system_inference,
                run_context.clone(),
                Arc::clone(&self.model_accountant),
                Arc::clone(&self.model_policy_guard),
            ));
        default_host_managed_loop_compaction_port(
            system_inference,
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            include_str!("../../ironclaw_loop_support/prompts/compaction_summarizer_fresh.md"),
        )
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
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
        F: Fn() -> HookDispatcherBuilder + Send + Sync + 'static,
    {
        self.hook_dispatcher_builder_factory = Some(Arc::new(factory));
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
    /// which mints a fresh dispatcher per host build and provides full
    /// per-run isolation of hook state.
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

    pub fn with_safety_context(mut self, safety_context: InstructionSafetyContext) -> Self {
        self.safety_context = Some(safety_context);
        self
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
        validate_thread_scope(&self.thread_scope, &request.loop_run_context)?;
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
        validate_thread_scope(&self.thread_scope, &request.loop_run_context)?;

        let max_messages = self.config.max_messages.max(1);
        let run_context = self.attach_model_route_snapshot(request.loop_run_context)?;
        let mut context_adapter = ThreadBackedLoopContextPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            run_context.clone(),
            max_messages,
        );
        if let Some(source) = self.skill_context_source.as_ref() {
            context_adapter = context_adapter.with_skill_context_source(source.clone());
        }
        if let Some(source) = self.identity_context_source.as_ref() {
            context_adapter = context_adapter.with_identity_context_source(source.clone());
        }
        context_adapter = context_adapter.with_milestone_sink(Arc::clone(&self.milestone_sink));
        let context: Arc<dyn LoopContextPort> = Arc::new(context_adapter);
        // Mint a fresh dispatcher per build when a factory is installed. This
        // localizes dispatcher-owned state (slot poisoning, registry edits,
        // predicate counters) to this one host so it cannot leak into the
        // next run that shares this factory.
        //
        // Builder-factory path (preferred): the factory hands back a
        // mutable builder; we attach a `RunScopedHookMilestoneSink` keyed
        // to *this* run's `LoopRunContext` *before* sealing. Hook
        // telemetry then carries the correct run/thread scope without
        // depending on the closure capturing the right context (which
        // would silently misattribute across reuses — henrypark133
        // Critical #4).
        let per_build_dispatcher = match (
            self.hook_dispatcher_builder_factory.as_ref(),
            self.hook_dispatcher_factory.as_ref(),
        ) {
            (Some(builder_factory), _) => {
                let builder = builder_factory();
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
        let event_subscription = match (
            per_build_dispatcher.as_ref(),
            self.event_subscription.as_ref(),
        ) {
            (Some(dispatcher), Some(subscription)) => {
                // NOTE(#3640): bind subscription stream/read-scope to
                // this host's run scope. Otherwise a
                // caller wiring tenant A's host with tenant B's stream
                // would silently dispatch B's hook events into A's
                // dispatcher with A's context tenant.
                subscription
                    .validate_against_run_scope(&run_context.scope, &self.thread_scope)
                    .map_err(|reason| RebornLoopDriverHostError::ScopeMismatch { reason })?;
                Some(subscription.clone_for_independent_spawn().spawn(
                    Arc::clone(dispatcher),
                    run_context.scope.tenant_id.clone(),
                    run_context.clone(),
                    Arc::clone(&self.milestone_sink) as _,
                ))
            }
            _ => None,
        };
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
        capabilities
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: error.safe_summary,
            })?;
        let prompt_authority = LoopPromptBundleAuthority::shared();
        let surface_state_for_prompt = Arc::clone(&surface_state);
        let mut prompt_port = HostManagedLoopPromptPort::new(
            run_context.clone(),
            Arc::clone(&context),
            Arc::clone(&self.milestone_sink),
        )
        .with_prompt_bundle_authority(prompt_authority.clone())
        .with_default_message_limit(max_messages)
        .with_current_surface_lookup(move || surface_state_for_prompt.current())
        .with_instruction_materialization_store(Arc::clone(&instruction_materialization_store));
        if let Some(safety_context) = self.safety_context.clone() {
            prompt_port = prompt_port.with_safety_context(safety_context);
        }
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
        if run_context.resolved_run_profile.profile_id.as_str() == SUBAGENT_PLANNED_PROFILE_ID {
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
        let model_gateway = Arc::new(ThreadResolvingLoopModelGateway {
            thread_service: Arc::clone(&self.thread_service),
            thread_scope: self.thread_scope.clone(),
            host_gateway: Arc::clone(&self.model_gateway),
            max_messages,
            skill_context_source: self.skill_context_source.clone(),
            identity_context_source: self.identity_context_source.clone(),
            instruction_materialization_store: Some(Arc::clone(&instruction_materialization_store)),
            capabilities: Some(Arc::clone(&capabilities)),
            prompt_authority,
        });
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
                self.thread_scope.clone(),
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
        let progress: Arc<dyn LoopProgressPort> = Arc::new(HostManagedLoopProgressPort::new(
            run_context.clone(),
            Arc::clone(&self.milestone_sink),
        ));
        let compaction = self.build_compaction_ports(&run_context);
        let cancellation_handle = self
            .cancellation_factory
            .handle_for_run(&run_context.scope, run_context.run_id)
            .await
            .map_err(|error| RebornLoopDriverHostError::InvalidRequest {
                reason: error.safe_summary,
            })?;
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
        tool_call: ProviderToolCall,
    ) -> Result<ironclaw_turns::run_profile::CapabilityCallCandidate, AgentLoopHostError> {
        self.capabilities
            .register_provider_tool_call(tool_call)
            .await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.capabilities.visible_capabilities(request).await
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
    ) -> Result<LoopCompactionResponse, LoopCompactionError> {
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
        TurnError::Conflict { .. } => ironclaw_loop_support::raw_agent_loop_host_error(
            "checkpoint_state",
            "write",
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write conflicted with current turn state",
            &error,
        ),
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
            surface_version: CapabilitySurfaceVersion::new("v1")
                .expect("surface version literal valid"),
            capability_id: CapabilityId::new("cap.test").expect("capability id literal valid"),
            input_ref: CapabilityInputRef::new(input_ref).expect("input ref literal valid"),
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
mod tests {
    use super::*;

    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::{
        InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryRunProfileResolver,
        PutLoopCheckpointRequest, RunProfileResolver, TurnCheckpointId, TurnId, TurnRunId,
        TurnScope,
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

#[cfg(test)]
mod turn_error_to_host_error_tests {
    use super::*;
    use ironclaw_turns::{TurnCapacityResource, TurnError};

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
