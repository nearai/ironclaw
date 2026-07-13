use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, ProjectId, ProviderToolName, TenantId, ThreadId,
};
use ironclaw_loop_support::{
    CapabilityResultWrite, DurablePersistence, LoopCapabilityPortDecorator,
    LoopCapabilityResultWriter,
};
use ironclaw_turns::{
    CapabilityActivityId, TurnId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityCallCandidate, CapabilityFailure, CapabilityFailureKind,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilityProgress,
        CapabilityResultMessage, CapabilitySurfaceVersion, LoopCapabilityPort, LoopRunContext,
        ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolCallReplay,
        ProviderToolDefinition, RegisterProviderToolCallRequest, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
};
use serde_json::{Value, json};
use tracing::debug;

use crate::tool_disclosure::{
    ActiveSet, CapabilityCatalog, DisclosureCaps, PromotedSet, TOOL_CALL_NAME, TOOL_DESCRIBE_NAME,
    TOOL_SEARCH_NAME, bridge_tool_definitions, canonicalize_json, definition_matches_provider_name,
    is_bridge_capability_id, is_bridge_name, select_active_set, tool_search_rank,
};

const DISCLOSURE_INPUT_PREFIX: &str = "input:tool-disclosure:";

/// Internal bridge name for an auto-loaded schema (describe-first) response.
///
/// NOT a real provider tool name, so it can never collide with a catalog tool or
/// trip `is_bridge_name`. When the model calls a deferred tool whose schema it
/// has not loaded this turn with arguments that fail pre-dispatch validation, the
/// register path routes the call to this synthetic bridge instead of dispatching
/// blind; `invoke_describe_first` then returns the tool's parameter schema so the
/// model's retry can carry the required fields. See `register_describe_first`.
const DESCRIBE_FIRST_BRIDGE_NAME: &str = "tool_disclosure:auto_schema";

/// Provider tool name of the loop's `capability_info` inspector (mirrors
/// `ironclaw_loop_support::capability_info::TOOL_NAME`). Inspecting a deferred
/// tool via `capability_info` is treated as intent to use it: the target is
/// disclosed + promoted so it becomes directly callable — the `tool_search` →
/// `capability_info` → direct-call discovery path.
const CAPABILITY_INFO_NAME: &str = "capability_info";

pub(crate) struct ToolDisclosureCapabilityDecorator {
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
    caps: DisclosureCaps,
}

impl ToolDisclosureCapabilityDecorator {
    pub(crate) fn new(result_writer: Arc<dyn LoopCapabilityResultWriter>) -> Self {
        Self {
            result_writer,
            promoted_by_scope: Arc::new(Mutex::new(HashMap::new())),
            caps: DisclosureCaps::default(),
        }
    }
}

impl LoopCapabilityPortDecorator for ToolDisclosureCapabilityDecorator {
    fn decorate(
        &self,
        run_context: &LoopRunContext,
        inner: Arc<dyn LoopCapabilityPort>,
    ) -> Arc<dyn LoopCapabilityPort> {
        Arc::new(ToolDisclosureCapabilityPort {
            inner,
            run_context: run_context.clone(),
            result_writer: Arc::clone(&self.result_writer),
            promoted_by_scope: Arc::clone(&self.promoted_by_scope),
            caps: self.caps,
            turn_state: Mutex::new(None),
            bridge_inputs: Mutex::new(BTreeMap::new()),
            tool_call_target_inputs: Mutex::new(BTreeMap::new()),
        })
    }
}

struct ToolDisclosureCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
    caps: DisclosureCaps,
    turn_state: Mutex<Option<ToolDisclosureTurnState>>,
    bridge_inputs: Mutex<BTreeMap<String, BridgeInvocation>>,
    tool_call_target_inputs: Mutex<BTreeMap<String, CapabilityId>>,
}

#[derive(Debug, Clone)]
struct ToolDisclosureTurnState {
    turn_id: TurnId,
    /// Fingerprint of the inner tool surface the catalog was built from. The
    /// catalog is rebuilt when this changes so tools that become available
    /// mid-turn (an activated extension, a completed OAuth connect) enter the
    /// disclosure catalog and become discoverable/describable/callable — without
    /// it, `tool_describe`/`tool_call` report a just-activated tool as "unknown".
    definitions_fingerprint: u64,
    surface_version: Option<CapabilitySurfaceVersion>,
    catalog: CapabilityCatalog,
    active: ActiveSet,
    disclosed_names: BTreeSet<String>,
}

/// Cheap order-independent-of-content fingerprint of the visible tool surface,
/// used to detect mid-turn changes (extension activation / OAuth connect) so the
/// disclosure catalog can refresh. `tool_definitions()` is already name-sorted,
/// so hashing count + names in order is deterministic.
fn definitions_fingerprint(definitions: &[ProviderToolDefinition]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    definitions.len().hash(&mut hasher);
    for definition in definitions {
        definition.name.as_str().hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Debug, Clone)]
struct BridgeInvocation {
    kind: BridgeKind,
    arguments: Value,
}

/// Which synthetic bridge a stored [`BridgeInvocation`] resolves to at invoke
/// time. Replaces discriminating on stashed name strings so the dispatch in
/// [`ToolDisclosureCapabilityDecorator::invoke_bridge`] is exhaustive and
/// legible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeKind {
    /// `tool_search` — keyword-rank the deferred catalog.
    Search,
    /// `tool_describe` — return a named tool's parameter schema.
    Describe,
    /// Internal auto-schema (describe-first): return a deferred tool's schema
    /// after a blind call to it failed pre-dispatch validation.
    DescribeFirst,
    /// `tool_call` — invoke a tool by name. Reaching invoke with this kind means
    /// the target could not be resolved (a resolvable target is dispatched
    /// directly and never stored as a bridge invocation), so it always errors
    /// recoverably.
    Call,
}

impl BridgeKind {
    /// Map a stored bridge name to its kind. Returns `None` for a name that is
    /// not one of the known bridges.
    fn from_provider_name(name: &str) -> Option<Self> {
        match name {
            TOOL_SEARCH_NAME => Some(Self::Search),
            TOOL_DESCRIBE_NAME => Some(Self::Describe),
            DESCRIBE_FIRST_BRIDGE_NAME => Some(Self::DescribeFirst),
            TOOL_CALL_NAME => Some(Self::Call),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedToolTarget {
    definition: ProviderToolDefinition,
    target_call: ProviderToolCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PromotionScopeKey {
    tenant_id: TenantId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    thread_id: ThreadId,
}

impl PromotionScopeKey {
    fn from_run_context(run_context: &LoopRunContext) -> Self {
        let scope = &run_context.scope;
        Self {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
        }
    }
}

#[async_trait]
impl LoopCapabilityPort for ToolDisclosureCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let state = self.turn_state()?;
        let Some(state) = state.as_ref() else {
            return Ok(Vec::new());
        };
        let catalog_total_count = state.catalog.len();
        let catalog_total_schema_tokens = state.catalog.total_schema_tokens();
        // Live token savings = how much of the full (authorized) tool surface we
        // avoided advertising this turn. Lets a benchmark/live run report the
        // real reduction directly from one log line (the fixture benchmark can't,
        // since its names are decoupled from the real core).
        let reduction_pct = if catalog_total_schema_tokens > 0 {
            100.0
                * (1.0
                    - (f64::from(state.active.advertised_tokens)
                        / f64::from(catalog_total_schema_tokens)))
        } else {
            0.0
        };
        debug!(
            target: "ironclaw::reborn::context_shadow",
            catalog_total_count,
            catalog_total_schema_tokens,
            advertised_tool_count = state.active.definitions.len(),
            advertised_tool_schema_tokens = state.active.advertised_tokens,
            deferred = state.active.deferred,
            reduction_pct,
            "reborn live tool disclosure surface"
        );
        Ok(state.active.definitions.clone())
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if !is_bridge_name(tool_call.name.as_str()) {
            if let Some(target) = self.direct_deferred_target(tool_call)? {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    capability_id = target.definition.capability_id.as_str(),
                    "reborn tool disclosure resolving direct deferred provider tool call"
                );
                // Resolve to the catalog's capability id directly. This is the
                // resolvability gate for the gateway pre-check; it must NOT depend
                // on the inner port being able to re-resolve the (unadvertised)
                // provider name, which it cannot for a deferred tool. The real
                // effective_capability_ids / approval expansion are applied later
                // by validate/register, which dispatch the synthesized target call.
                return Ok(ProviderToolCallCapabilityIds::single(
                    target.definition.capability_id,
                ));
            }
            return self.inner.provider_tool_call_capability_ids(tool_call);
        }
        if tool_call.name.as_str() == TOOL_CALL_NAME
            && let Some(target) = self.allowed_tool_call_target(tool_call)?
        {
            return Ok(ProviderToolCallCapabilityIds {
                provider_capability_id: target.definition.capability_id.clone(),
                effective_capability_ids: vec![target.definition.capability_id],
            });
        }
        let Some(definition) = bridge_tool_definitions()
            .into_iter()
            .find(|definition| definition.name == tool_call.name)
        else {
            return Err(invalid_invocation("bridge tool definition is unavailable"));
        };
        Ok(ProviderToolCallCapabilityIds::single(
            definition.capability_id,
        ))
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if !is_bridge_name(tool_call.name.as_str()) {
            if let Some(target) = self.direct_deferred_target(tool_call)? {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    capability_id = target.definition.capability_id.as_str(),
                    "reborn tool disclosure validating direct deferred provider tool call"
                );
                return self.inner.validate_provider_tool_call(&target.target_call);
            }
            return self.inner.validate_provider_tool_call(tool_call);
        }
        if matches!(
            tool_call.name.as_str(),
            TOOL_SEARCH_NAME | TOOL_DESCRIBE_NAME
        ) {
            return Ok(());
        }
        if tool_call.name.as_str() == TOOL_CALL_NAME
            && let Some(target) = self.allowed_tool_call_target(tool_call)?
        {
            // A resolved target that fails inner validation must NOT abort the
            // whole provider response — the gateway discards the entire response
            // on a validation error, which turns a recoverable bad-arguments
            // `tool_call` into a run-borking failure. Probe validation for an
            // early diagnostic, but always return Ok: registration falls back to
            // the bridge path on failure, surfacing a recoverable invalid_input
            // at invoke time that the model can correct and retry.
            //
            // NOTE: this makes `validate_provider_tool_call` no longer mean "this
            // will pass" for the bridge path — the real gate is `register`. That
            // muddied contract is a workaround for the gateway's discard-the-whole-
            // response-on-validate-error behavior; the honest fix is upstream (fail
            // only the offending call, not the whole response). Tracked in the
            // context-management design doc under "validate contract"; remove this
            // probe-and-swallow once the gateway stops discarding the response.
            if let Err(error) = self.inner.validate_provider_tool_call(&target.target_call) {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    target = target.definition.name.as_str(),
                    error_kind = ?error.kind,
                    "tool_call target failed inner validation; deferring to recoverable bridge failure"
                );
            }
            return Ok(());
        }
        Ok(())
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let RegisterProviderToolCallRequest {
            tool_call,
            activity_id,
        } = request;
        // Inspecting a deferred tool via `capability_info` promotes it for direct
        // use next turn (search → capability_info → direct call). Runs before
        // dispatch so the promotion lands even though the call itself delegates
        // to the inner port below.
        self.note_capability_info_target(&tool_call)?;
        if !is_bridge_name(tool_call.name.as_str()) {
            if let Some(target) = self.direct_deferred_target(&tool_call)? {
                // Preserve the model's emitted wire name in the replay when it is
                // already valid (the common `__`-encoded case) so the replayed
                // assistant tool call mirrors what the model generated. Only when
                // the model called the deferred tool by a non-wire-safe form —
                // most often the dotted catalog capability_id like
                // `google-calendar.list_events` — fall back to the resolved
                // definition's canonical name; recording a dotted name fails
                // `validate_provider_tool_name` and borks the run on transcript
                // write.
                let replay_tool_name =
                    replay_provider_tool_name(&tool_call.name, &target.definition.name);
                debug!(
                    tool_name = tool_call.name.as_str(),
                    replay_tool_name = replay_tool_name.as_str(),
                    capability_id = target.definition.capability_id.as_str(),
                    "reborn tool disclosure registering direct deferred provider tool call"
                );
                // Describe-first (see `should_describe_first`): a deferred tool
                // called by name with arguments that fail pre-dispatch validation
                // gets its schema instead of a blind dispatch, one-shot per
                // undisclosed tool.
                if self.should_describe_first(&target)? {
                    debug!(
                        tool_name = tool_call.name.as_str(),
                        capability_id = target.definition.capability_id.as_str(),
                        "deferred direct call failed pre-dispatch validation before its schema was disclosed; returning schema (describe-first)"
                    );
                    return self.register_describe_first(
                        &tool_call,
                        replay_tool_name,
                        target.definition.name.as_str(),
                    );
                }
                let mut candidate = self
                    .inner
                    .register_provider_tool_call(register_request(target.target_call, activity_id))
                    .await?;
                candidate.provider_replay = Some(provider_replay_for(&tool_call, replay_tool_name));
                self.record_promotable_input(
                    candidate.input_ref.as_str(),
                    candidate.capability_id.clone(),
                )?;
                return Ok(candidate);
            }
            return self
                .inner
                .register_provider_tool_call(register_request(tool_call, activity_id))
                .await;
        }
        if tool_call.name.as_str() == TOOL_CALL_NAME
            && let Some(target) = self.allowed_tool_call_target(&tool_call)?
        {
            // The model invoked the `tool_call` bridge itself (a valid wire
            // name); the replay reflects that actual call, not the target.
            let bridge_provider_tool_name = tool_call.name.clone();
            // Describe-first (see `should_describe_first`): same as the
            // direct-deferred path above, but for a tool reached via the bridge.
            if self.should_describe_first(&target)? {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    target = target.definition.name.as_str(),
                    "tool_call to an undisclosed tool failed pre-dispatch validation; returning schema (describe-first)"
                );
                return self.register_describe_first(
                    &tool_call,
                    bridge_provider_tool_name,
                    target.definition.name.as_str(),
                );
            }
            match self
                .inner
                .register_provider_tool_call(register_request(target.target_call, activity_id))
                .await
            {
                Ok(mut candidate) => {
                    candidate.provider_replay =
                        Some(provider_replay_for(&tool_call, bridge_provider_tool_name));
                    self.record_promotable_input(
                        candidate.input_ref.as_str(),
                        candidate.capability_id.clone(),
                    )?;
                    return Ok(candidate);
                }
                Err(error) => {
                    // The resolved target could not be registered (e.g. malformed
                    // arguments for a deferred tool). Fall back to the bridge path
                    // so the model receives a recoverable invalid_input failure at
                    // invoke time instead of the whole run aborting.
                    debug!(
                        tool_name = tool_call.name.as_str(),
                        error_kind = ?error.kind,
                        "tool_call target registration failed; falling back to recoverable bridge failure"
                    );
                    return self.register_bridge_call(tool_call);
                }
            }
        }
        self.register_bridge_call(tool_call)
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        // The inner surface is the full reachable authorized catalog *before* we
        // narrow the advertised `descriptors` below. Capture it as the call-time
        // "callable" view so the model-visible capability filter authorizes
        // bridge / forgiving-direct calls to catalog tools the model legitimately
        // reaches this turn but that aren't advertised. Without this, a resumed
        // run whose discovered tools dropped off the advertised surface has its
        // retry hard-rejected as "outside the model-visible capability view".
        let callable_capability_ids: Vec<CapabilityId> = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        let mut state = self.turn_state()?;
        let Some(state) = state.as_mut() else {
            surface.callable_capability_ids = Some(callable_capability_ids);
            return Ok(surface);
        };
        state.surface_version = Some(surface.version.clone());
        let active_or_disclosed_descriptors = state
            .catalog
            .active_or_disclosed_descriptors(&state.active, &state.disclosed_names);
        let active_or_disclosed_ids: BTreeSet<CapabilityId> = active_or_disclosed_descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        let bridge_descriptors: Vec<_> = active_or_disclosed_descriptors
            .into_iter()
            .filter(|descriptor| is_bridge_capability_id(&descriptor.capability_id))
            .collect();
        surface.descriptors.retain(|descriptor| {
            active_or_disclosed_ids.contains(&descriptor.capability_id)
                && !is_bridge_capability_id(&descriptor.capability_id)
        });
        let mut advertised_ids: BTreeSet<CapabilityId> = surface
            .descriptors
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect();
        for descriptor in bridge_descriptors {
            if advertised_ids.insert(descriptor.capability_id.clone()) {
                surface.descriptors.push(descriptor);
            }
        }
        // Callable = the full reachable catalog (captured above) UNION the tools
        // actually advertised this turn UNION every bridge capability. Only
        // `tool_search` is advertised now, but the `tool_describe` / `tool_call`
        // synthetic capabilities are still used internally — describe-first routes
        // a schema response through `tool_describe`'s capability id. Keeping ALL
        // bridge capabilities callable authorizes that internal routing at the
        // executor visibility gate even though they are no longer advertised;
        // without it a describe-first response is rejected as "not visible".
        let mut callable: BTreeSet<CapabilityId> = callable_capability_ids.into_iter().collect();
        callable.extend(
            surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.clone()),
        );
        callable.extend(
            bridge_tool_definitions()
                .into_iter()
                .map(|definition| definition.capability_id),
        );
        surface.callable_capability_ids = Some(callable.into_iter().collect());
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if !is_bridge_capability_id(&request.capability_id) {
            let target_capability_id = self
                .tool_call_target_inputs
                .lock()
                .map_err(|e| {
                    invalid_invocation(format!("tool_call target store lock is poisoned: {e}"))
                })?
                .get(request.input_ref.as_str())
                .cloned();
            let outcome = self.inner.invoke_capability(request).await?;
            // Promote on a completed dispatch OR a gate suspension (approval/auth/
            // resource). A tool the model dispatched that paused for a user action
            // is just as "earned" as a completed one, and it MUST stay visible
            // across the BlockedApproval/BlockedAuth resume: otherwise the per-turn
            // disclosed set resets, the tool drops off the model-visible surface,
            // and the model's retry is hard-rejected by the visible-surface filter
            // ("outside the model-visible capability view") — discarding the whole
            // response and borking the run. A hard *failure* still does NOT promote
            // (the model may abandon it), so this does not drift toward advertising
            // every discovered tool — only ones the model actually invoked.
            if (matches!(outcome, CapabilityOutcome::Completed(_)) || outcome.is_suspension())
                && let Some(capability_id) = target_capability_id
            {
                self.promote_target(&capability_id)?;
            }
            return Ok(outcome);
        }
        self.invoke_bridge(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::with_capacity(request.invocations.len());
        let mut stopped_on_suspension = false;
        for invocation in request.invocations {
            let outcome = self.invoke_capability(invocation).await?;
            let is_suspension = outcome.is_suspension();
            outcomes.push(outcome);
            if request.stop_on_first_suspension && is_suspension {
                stopped_on_suspension = true;
                break;
            }
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

impl ToolDisclosureCapabilityPort {
    fn turn_state(
        &self,
    ) -> Result<MutexGuard<'_, Option<ToolDisclosureTurnState>>, AgentLoopHostError> {
        let mut guard = self.turn_state.lock().map_err(|e| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("tool disclosure turn state lock is poisoned: {e}"),
            )
        })?;
        let definitions = self.inner.tool_definitions()?;
        let fingerprint = definitions_fingerprint(&definitions);
        let same_turn = guard
            .as_ref()
            .map(|state| state.turn_id == self.run_context.turn_id)
            .unwrap_or(false);
        let rebuild = guard
            .as_ref()
            .map(|state| {
                state.turn_id != self.run_context.turn_id
                    || state.definitions_fingerprint != fingerprint
            })
            .unwrap_or(true);
        if rebuild {
            let catalog = CapabilityCatalog::new(&definitions, &[]);
            let promoted = self.promoted_for_scope()?;
            let active = select_active_set(&catalog, &promoted, self.caps);
            // Preserve disclosure progress across a same-turn refresh (a tool the
            // model already described stays disclosed); a genuine turn change
            // starts fresh.
            let (surface_version, disclosed_names) = guard
                .take()
                .filter(|_| same_turn)
                .map(|state| (state.surface_version, state.disclosed_names))
                .unwrap_or((None, BTreeSet::new()));
            *guard = Some(ToolDisclosureTurnState {
                turn_id: self.run_context.turn_id,
                definitions_fingerprint: fingerprint,
                surface_version,
                catalog,
                active,
                disclosed_names,
            });
        }
        Ok(guard)
    }

    fn promoted_for_scope(&self) -> Result<PromotedSet, AgentLoopHostError> {
        let key = PromotionScopeKey::from_run_context(&self.run_context);
        let guard = self.promoted_by_scope.lock().map_err(|e| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("tool disclosure promoted set lock is poisoned: {e}"),
            )
        })?;
        Ok(guard.get(&key).cloned().unwrap_or_default())
    }

    fn promote_target(&self, capability_id: &CapabilityId) -> Result<(), AgentLoopHostError> {
        let name = {
            let guard = self.turn_state()?;
            let Some(state) = guard.as_ref() else {
                return Ok(());
            };
            state
                .catalog
                .definition_by_capability_id(capability_id)
                .map(|definition| definition.name.to_string())
        };
        let Some(name) = name else {
            return Ok(());
        };
        let key = PromotionScopeKey::from_run_context(&self.run_context);
        let mut guard = self.promoted_by_scope.lock().map_err(|e| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("tool disclosure promoted set lock is poisoned: {e}"),
            )
        })?;
        guard.entry(key).or_default().push(name);
        Ok(())
    }

    /// When the model inspects a deferred tool via `capability_info`, treat it as
    /// intent to use that tool: disclose it this turn and promote it for the scope
    /// so it becomes advertised with its full schema and directly callable next
    /// turn — the `tool_search` → `capability_info` → direct-call flow. This is the
    /// same disclose+promote a `tool_describe` / successful `tool_call` already
    /// does, wired onto the `capability_info` path so that path can stand alone.
    /// No-op for a non-`capability_info` call or a target not in the catalog.
    fn note_capability_info_target(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if tool_call.name.as_str() != CAPABILITY_INFO_NAME {
            return Ok(());
        }
        let Some(target_name) = tool_call
            .arguments
            .get("name")
            .or_else(|| tool_call.arguments.get("capability_id"))
            .and_then(Value::as_str)
        else {
            return Ok(());
        };
        let capability_id = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(());
            };
            // `capability_info`'s `name` may be the provider tool name (dotted or
            // encoded) or the canonical capability id — resolve either.
            let resolved = state
                .catalog
                .search_result(target_name)
                .map(|result| (result.name, result.capability_id))
                .or_else(|| {
                    CapabilityId::new(target_name)
                        .ok()
                        .and_then(|capability_id| {
                            state
                                .catalog
                                .definition_by_capability_id(&capability_id)
                                .map(|definition| {
                                    (
                                        definition.name.to_string(),
                                        definition.capability_id.clone(),
                                    )
                                })
                        })
                });
            let Some((name, capability_id)) = resolved else {
                return Ok(());
            };
            state.disclosed_names.insert(name);
            capability_id
        };
        debug!(
            target = target_name,
            capability_id = capability_id.as_str(),
            "capability_info inspected a deferred tool; disclosing + promoting it for direct use"
        );
        self.promote_target(&capability_id)
    }

    fn record_promotable_input(
        &self,
        input_ref: &str,
        capability_id: CapabilityId,
    ) -> Result<(), AgentLoopHostError> {
        self.tool_call_target_inputs
            .lock()
            .map_err(|e| invalid_invocation(format!("tool target store lock is poisoned: {e}")))?
            .insert(input_ref.to_string(), capability_id);
        Ok(())
    }

    /// Whether the model has loaded this tool's schema this turn (via
    /// `tool_search` / `tool_describe` / a prior describe-first). Gates
    /// describe-first so it fires at most once per undisclosed tool: once the
    /// schema is in context, a still-invalid call dispatches and fails through the
    /// normal path the no-progress detector can count.
    fn is_disclosed(&self, name: &str) -> Result<bool, AgentLoopHostError> {
        let guard = self.turn_state()?;
        Ok(guard
            .as_ref()
            .map(|state| state.disclosed_names.contains(name))
            .unwrap_or(false))
    }

    fn register_bridge_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let Some(definition) = bridge_tool_definitions()
            .into_iter()
            .find(|definition| definition.name == tool_call.name)
        else {
            return Err(invalid_invocation("bridge tool definition is unavailable"));
        };
        let digest_input = provider_call_digest_input(
            &tool_call.id,
            tool_call.name.as_str(),
            &tool_call.arguments,
        );
        let digest = ironclaw_host_api::sha256_digest_token(digest_input.as_bytes());
        let input_ref = CapabilityInputRef::new(format!("{DISCLOSURE_INPUT_PREFIX}{digest}"))
            .map_err(|e| {
                invalid_invocation(format!("bridge input ref could not be represented: {e}"))
            })?;
        self.bridge_inputs
            .lock()
            .map_err(|e| invalid_invocation(format!("bridge input store lock is poisoned: {e}")))?
            .insert(
                input_ref.as_str().to_string(),
                BridgeInvocation {
                    kind: BridgeKind::from_provider_name(tool_call.name.as_str()).ok_or_else(
                        || invalid_invocation("bridge tool definition is unavailable"),
                    )?,
                    arguments: tool_call.arguments.clone(),
                },
            );
        let surface_version = self.current_surface_version()?;
        Ok(CapabilityCallCandidate {
            activity_id: CapabilityActivityId::new(),
            surface_version,
            capability_id: definition.capability_id,
            input_ref,
            effective_capability_ids: Vec::new(),
            provider_replay: Some(provider_replay_for(&tool_call, tool_call.name.clone())),
        })
    }

    /// Whether a resolved deferred call should be answered with its schema
    /// (describe-first) rather than dispatched blind.
    ///
    /// This is the blind-call regression tool disclosure introduces:
    /// pre-disclosure the full schema was always in context so the model filled
    /// required fields; with schemas deferred the model calls by name alone,
    /// omitting required arguments and looping on the opaque validation error.
    /// True when the target's schema has NOT been disclosed this turn AND its
    /// arguments fail pre-dispatch validation. Once disclosed, a still-invalid
    /// retry dispatches and fails normally, so the no-progress detector still
    /// observes the repeated failure. Well-formed blind calls return false and
    /// dispatch directly, adding no round-trip on correct calls.
    fn should_describe_first(
        &self,
        target: &ResolvedToolTarget,
    ) -> Result<bool, AgentLoopHostError> {
        if self.is_disclosed(target.definition.name.as_str())? {
            return Ok(false);
        }
        // Resolution failure: the inner port can't even resolve the call.
        if self
            .inner
            .validate_provider_tool_call(&target.target_call)
            .is_err()
        {
            return Ok(true);
        }
        // Input-schema failure: the call resolves, but its arguments don't satisfy
        // the tool's parameter schema. Pre-disclosure the full schema was always
        // in context so the model formatted the call; deferred, it calls the tool
        // blind and a nested-shape error (e.g. a `schedule` `oneOf`) hands back no
        // schema to recover from, so a weak model guesses the shape and spirals.
        // Probe the arguments against the catalog schema and describe-first on a
        // mismatch so the model's retry carries the real schema + examples.
        Ok(!arguments_satisfy_schema(
            &target.target_call.arguments,
            &target.definition.parameters,
        ))
    }

    /// Register a deferred call whose arguments failed pre-dispatch validation as
    /// an auto-schema (describe-first) bridge response rather than a blind
    /// dispatch. `invoke_describe_first` returns the tool's parameter schema and
    /// marks it disclosed, so the model's retry carries the required fields.
    ///
    /// The candidate borrows the `tool_describe` bridge capability id so
    /// `invoke_capability` routes it to `invoke_bridge`; the stored
    /// `BridgeInvocation` name (`DESCRIBE_FIRST_BRIDGE_NAME`) distinguishes it from
    /// a genuine `tool_describe`. The replay mirrors the model's actual call —
    /// `replay_tool_name` is the wire-safe name the caller already resolved (the
    /// bridge name, or the canonical definition name for a dotted direct call).
    fn register_describe_first(
        &self,
        tool_call: &ProviderToolCall,
        replay_tool_name: ProviderToolName,
        target_name: &str,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let Some(definition) = bridge_tool_definitions()
            .into_iter()
            .find(|definition| definition.name.as_str() == TOOL_DESCRIBE_NAME)
        else {
            return Err(invalid_invocation(
                "tool_describe bridge definition is unavailable",
            ));
        };
        // Distinct digest input so an auto-schema input never collides with a
        // genuine bridge input for the same provider call id.
        let digest_input = provider_call_digest_input(
            &format!("{}:auto-schema", tool_call.id),
            target_name,
            &tool_call.arguments,
        );
        let digest = ironclaw_host_api::sha256_digest_token(digest_input.as_bytes());
        let input_ref = CapabilityInputRef::new(format!("{DISCLOSURE_INPUT_PREFIX}{digest}"))
            .map_err(|e| {
                invalid_invocation(format!(
                    "auto-schema input ref could not be represented: {e}"
                ))
            })?;
        self.bridge_inputs
            .lock()
            .map_err(|e| invalid_invocation(format!("bridge input store lock is poisoned: {e}")))?
            .insert(
                input_ref.as_str().to_string(),
                BridgeInvocation {
                    kind: BridgeKind::DescribeFirst,
                    arguments: json!({ "name": target_name }),
                },
            );
        let surface_version = self.current_surface_version()?;
        Ok(CapabilityCallCandidate {
            activity_id: CapabilityActivityId::new(),
            surface_version,
            capability_id: definition.capability_id,
            input_ref,
            effective_capability_ids: Vec::new(),
            provider_replay: Some(provider_replay_for(tool_call, replay_tool_name)),
        })
    }

    fn current_surface_version(&self) -> Result<CapabilitySurfaceVersion, AgentLoopHostError> {
        let guard = self.turn_state()?;
        guard
            .as_ref()
            .and_then(|state| state.surface_version.clone())
            .ok_or_else(|| invalid_invocation("capability surface is unavailable"))
    }

    async fn invoke_bridge(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let bridge = self
            .bridge_inputs
            .lock()
            .map_err(|e| invalid_invocation(format!("bridge input store lock is poisoned: {e}")))?
            .get(request.input_ref.as_str())
            .cloned()
            .ok_or_else(|| invalid_invocation("bridge input is unavailable"))?;
        match bridge.kind {
            BridgeKind::Search => self.invoke_tool_search(&request, &bridge).await,
            BridgeKind::Describe => self.invoke_tool_describe(&request, &bridge).await,
            BridgeKind::DescribeFirst => self.invoke_describe_first(&request, &bridge).await,
            BridgeKind::Call => Ok(failed_invalid_input(
                "tool_call target is not a known tool; use tool_search to find the correct tool name",
            )),
        }
    }

    async fn invoke_tool_search(
        &self,
        request: &CapabilityInvocation,
        bridge: &BridgeInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let Some(query) = bridge.arguments.get("query").and_then(Value::as_str) else {
            return Ok(failed_invalid_input("tool_search requires query"));
        };
        let query = query.trim();
        if query.is_empty() {
            return Ok(failed_invalid_input("tool_search requires query"));
        }
        let limit = bridge
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(10)
            .clamp(1, 50);
        let output = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_invalid_input("tool catalog is unavailable"));
            };
            let names = tool_search_rank(&state.catalog, query, limit);
            let mut results = Vec::new();
            for name in names {
                state.disclosed_names.insert(name.clone());
                if let Some(result) = state.catalog.search_result(&name) {
                    results.push(json!({
                        "name": result.name,
                        "capability_id": result.capability_id.as_str(),
                        "description": result.description,
                        "required": result.required_params,
                    }));
                }
            }
            json!({
                "query": query,
                "results": results,
            })
        };
        self.completed_bridge_result(request, output, "tool_search returned catalog matches")
            .await
    }

    async fn invoke_tool_describe(
        &self,
        request: &CapabilityInvocation,
        bridge: &BridgeInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let Some(name) = bridge.arguments.get("name").and_then(Value::as_str) else {
            return Ok(failed_invalid_input("tool_describe requires name"));
        };
        if is_bridge_name(name) {
            return Ok(failed_invalid_input(
                "tool_describe target must not be a bridge",
            ));
        }
        let output = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_invalid_input("tool catalog is unavailable"));
            };
            let Some(result) = state.catalog.search_result(name) else {
                return Ok(failed_invalid_input("tool_describe target is unknown"));
            };
            state.disclosed_names.insert(name.to_string());
            json!({
                "name": result.name,
                "capability_id": result.capability_id.as_str(),
                "description": result.description,
                "required": result.required_params,
                "parameters": result.parameters,
            })
        };
        self.completed_bridge_result(request, output, "tool_describe returned schema")
            .await
    }

    /// Invoke an auto-schema (describe-first) bridge: return the target tool's
    /// parameter schema and mark it disclosed, so the model's retry carries the
    /// required fields. Mirrors `invoke_tool_describe` but its note tells the
    /// model the schema was loaded automatically because its call did not match —
    /// the schema is rendered exactly when the model needs it, restoring the
    /// pre-disclosure guarantee for the one call that got it wrong.
    async fn invoke_describe_first(
        &self,
        request: &CapabilityInvocation,
        bridge: &BridgeInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let Some(name) = bridge.arguments.get("name").and_then(Value::as_str) else {
            return Ok(failed_invalid_input("auto-schema requires a target name"));
        };
        let output = {
            let mut guard = self.turn_state()?;
            let Some(state) = guard.as_mut() else {
                return Ok(failed_invalid_input("tool catalog is unavailable"));
            };
            let Some(result) = state.catalog.search_result(name) else {
                return Ok(failed_invalid_input("auto-schema target is unknown"));
            };
            state.disclosed_names.insert(result.name.clone());
            json!({
                "status": "schema_loaded",
                "note": "Your previous arguments did not match this tool's schema (its schema had not been loaded yet). Here is the parameter schema — call the tool again with the required arguments.",
                "name": result.name,
                "capability_id": result.capability_id.as_str(),
                "description": result.description,
                "required": result.required_params,
                "parameters": result.parameters,
            })
        };
        self.completed_bridge_result(request, output, "auto-loaded tool schema before invocation")
            .await
    }

    async fn completed_bridge_result(
        &self,
        request: &CapabilityInvocation,
        output: Value,
        safe_summary: &'static str,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let write = self
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &self.run_context,
                input_ref: &request.input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &request.capability_id,
                output,
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
            })
            .await?;
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: write.result_ref,
            safe_summary: safe_summary.to_string(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: write.byte_len,
            output_digest: write.output_digest,
            model_observation: write.model_observation,
        }))
    }

    fn target_call(
        &self,
        tool_call: &ProviderToolCall,
        target: &ProviderToolDefinition,
        arguments: Value,
    ) -> ProviderToolCall {
        let digest_input =
            provider_call_digest_input(&tool_call.id, target.name.as_str(), &arguments);
        let target_id = ironclaw_host_api::sha256_digest_token(digest_input.as_bytes());
        ProviderToolCall {
            provider_id: tool_call.provider_id.clone(),
            provider_model_id: tool_call.provider_model_id.clone(),
            turn_id: tool_call.turn_id.clone(),
            id: format!("{}:{target_id}", tool_call.id),
            name: target.name.clone(),
            arguments,
            response_reasoning: tool_call.response_reasoning.clone(),
            reasoning: tool_call.reasoning.clone(),
            signature: tool_call.signature.clone(),
        }
    }

    fn allowed_tool_call_target(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<Option<ResolvedToolTarget>, AgentLoopHostError> {
        let Some(name) = tool_call.arguments.get("name").and_then(Value::as_str) else {
            return Ok(None);
        };
        if is_bridge_name(name) {
            return Ok(None);
        }
        let arguments = tool_call
            .arguments
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let guard = self.turn_state()?;
        let Some(state) = guard.as_ref() else {
            return Ok(None);
        };
        // Forgiving resolution: resolve any tool the catalog knows by name,
        // regardless of whether it has been advertised or discovered this turn.
        // A *direct* call to an undisclosed tool already resolves via
        // `direct_deferred_target`, so the `tool_call` bridge must not be
        // stricter than the direct path. Requiring prior disclosure here was a
        // dead end: a model that calls `tool_call` before `tool_search`/
        // `tool_describe` got a generic "invalid_input" with no recovery hint and
        // looped until the run died. Resolving forgivingly lets the call dispatch
        // and surface the tool's *real* schema error (with repairs) — which the
        // model can act on — and earns promotion on success via the register
        // path's `record_promotable_input`. Safety/approval/auth gates still run
        // at dispatch, so this is a token-economy boundary, not a security one.
        let Some(definition) = self.catalog_target(state, name) else {
            return Ok(None);
        };
        let target_call = self.target_call(tool_call, &definition, arguments);
        Ok(Some(ResolvedToolTarget {
            definition,
            target_call,
        }))
    }

    fn direct_deferred_target(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<Option<ResolvedToolTarget>, AgentLoopHostError> {
        if is_bridge_name(tool_call.name.as_str()) {
            return Ok(None);
        }
        let guard = self.turn_state()?;
        let Some(state) = guard.as_ref() else {
            debug!(
                tool_name = tool_call.name.as_str(),
                "reborn tool disclosure direct-deferred miss: no turn state"
            );
            return Ok(None);
        };
        let Some(definition) = self.catalog_target(state, tool_call.name.as_str()) else {
            // DIAGNOSTIC (temporary): the model called a non-bridge tool that the
            // catalog could not resolve by name. Sample catalog names + capability
            // ids that share the called tool's provider prefix, so a name-form
            // mismatch (dotted vs `__`-encoded) is visible vs. genuinely absent.
            let prefix: String = tool_call
                .name
                .as_str()
                .chars()
                .take_while(|c| *c != '_' && *c != '.' && *c != '-')
                .collect();
            let sample: Vec<String> = state
                .catalog
                .definitions()
                .filter(|definition| {
                    definition.name.as_str().starts_with(&prefix)
                        || definition.capability_id.as_str().starts_with(&prefix)
                })
                .map(|definition| {
                    format!("{}|{}", definition.name, definition.capability_id.as_str())
                })
                .take(8)
                .collect();
            debug!(
                tool_name = tool_call.name.as_str(),
                catalog_len = state.catalog.len(),
                prefix = prefix.as_str(),
                prefix_matches = ?sample,
                "reborn tool disclosure direct-deferred miss: not found in catalog by name"
            );
            return Ok(None);
        };
        let active = state
            .active
            .definitions
            .iter()
            .any(|candidate| candidate.name == tool_call.name);
        if active {
            // Normal path: the tool is advertised, so the inner port dispatches
            // it directly. Not a forgiving-path case.
            Ok(None)
        } else {
            let target_call = self.target_call(tool_call, &definition, tool_call.arguments.clone());
            Ok(Some(ResolvedToolTarget {
                definition,
                target_call,
            }))
        }
    }

    fn catalog_target(
        &self,
        state: &ToolDisclosureTurnState,
        provider_name: &str,
    ) -> Option<ProviderToolDefinition> {
        state
            .catalog
            .definition_by_name(provider_name)
            .or_else(|| {
                state
                    .catalog
                    .definitions()
                    .find(|definition| definition_matches_provider_name(definition, provider_name))
            })
            .cloned()
    }
}

/// Whether `arguments` satisfy `schema`, used only as a describe-first *assist*
/// (never as a gate).
///
/// Conservative by design: if the schema can't be compiled — an unresolved
/// `$ref`, a dialect `jsonschema` rejects — we return `true` ("satisfied") so the
/// call dispatches normally and the real capability-input validator remains the
/// single source of truth. This probe only decides whether to hand the model the
/// schema early; a false negative would merely block that assist, never a call.
fn arguments_satisfy_schema(arguments: &Value, schema: &Value) -> bool {
    match jsonschema::validator_for(schema) {
        Ok(validator) => validator.is_valid(arguments),
        Err(_) => true,
    }
}

/// Choose the wire name to record in a forgiving direct-deferred replay.
///
/// Preserve the model's emitted name when it is already a valid provider tool
/// name (the common `__`-encoded case) so the replayed assistant tool call
/// faithfully mirrors what the model generated. Only when the model called the
/// deferred tool by a non-wire-safe form — most often the dotted catalog
/// `capability_id` such as `google-calendar.list_events` — fall back to the
/// resolved definition's canonical name, which is always wire-safe. Recording a
/// dotted name fails `validate_provider_tool_name` and borks the run on the
/// assistant transcript / provider-error result-ref write.
fn replay_provider_tool_name(
    called_name: &ProviderToolName,
    definition_name: &ProviderToolName,
) -> ProviderToolName {
    if ironclaw_safety::validate_provider_tool_name(called_name.as_str()).is_ok() {
        called_name.clone()
    } else {
        definition_name.clone()
    }
}

/// Build the provider-call replay metadata recorded with a capability candidate.
///
/// `provider_tool_name` is the wire name the replay (and any provider-error
/// result reference) serializes into the transcript. It MUST be a canonical
/// provider tool name (`[A-Za-z0-9_-]`) because `validate_provider_tool_name`
/// rejects anything else and a failed transcript write borks the whole run. On
/// the forgiving direct-deferred path the model may have called a deferred tool
/// by its dotted catalog `capability_id` (e.g. `google-calendar.list_events`);
/// callers there must pass the resolved definition's `name` (the `__`-encoded
/// wire name), NOT the raw `tool_call.name`. Bridge/normal paths pass the
/// already-valid `tool_call.name`.
fn provider_replay_for(
    tool_call: &ProviderToolCall,
    provider_tool_name: ProviderToolName,
) -> ProviderToolCallReplay {
    ProviderToolCallReplay {
        provider_id: tool_call.provider_id.clone(),
        provider_model_id: tool_call.provider_model_id.clone(),
        provider_turn_id: tool_call.turn_id.clone().unwrap_or_default(),
        provider_call_id: tool_call.id.clone(),
        provider_tool_name,
        arguments: tool_call.arguments.clone(),
        response_reasoning: tool_call.response_reasoning.clone(),
        reasoning: tool_call.reasoning.clone(),
        signature: tool_call.signature.clone(),
    }
}

/// Build the inner-port registration request for a synthesized target call,
/// preserving any activity identity the gateway bound to the bridge call so the
/// inner port registers the dispatched target under the same id.
fn register_request(
    tool_call: ProviderToolCall,
    activity_id: Option<CapabilityActivityId>,
) -> RegisterProviderToolCallRequest {
    match activity_id {
        Some(activity_id) => RegisterProviderToolCallRequest::for_activity(tool_call, activity_id),
        None => RegisterProviderToolCallRequest::new(tool_call),
    }
}

fn provider_call_digest_input(provider_call_id: &str, name: &str, arguments: &Value) -> String {
    json!({
        "provider_call_id": provider_call_id,
        "name": name,
        "arguments": canonicalize_json(arguments),
    })
    .to_string()
}

fn failed_invalid_input(summary: &'static str) -> CapabilityOutcome {
    CapabilityOutcome::Failed(CapabilityFailure {
        error_kind: CapabilityFailureKind::InvalidInput,
        safe_summary: summary.to_string(),
        detail: None,
    })
}

fn invalid_invocation(summary: impl Into<String>) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_satisfy_schema_gates_describe_first_on_nested_shape() {
        // trigger_create's `schedule` must be an object (oneOf cron/once); a weak
        // model that calls it deferred often sends a bare cron string. That must
        // read as "does not satisfy" so describe-first hands over the schema.
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "schedule": {
                    "oneOf": [
                        {"type": "object", "properties": {"kind": {"const": "cron"}}, "required": ["kind", "expression"]},
                        {"type": "object", "properties": {"kind": {"const": "once"}}, "required": ["kind", "at"]}
                    ]
                }
            },
            "required": ["name", "schedule"]
        });
        assert!(
            !arguments_satisfy_schema(&json!({"name": "r", "schedule": "*/30 * * * *"}), &schema),
            "a bare-string schedule must fail the object oneOf → describe-first"
        );
        assert!(
            arguments_satisfy_schema(
                &json!({"name": "r", "schedule": {"kind": "cron", "expression": "*/30 * * * *"}}),
                &schema
            ),
            "the correct object shape must satisfy the schema → dispatch directly"
        );
        // Unresolved $ref / uncompilable schema is treated as satisfied (assist,
        // never a gate): the real capability validator stays authoritative.
        assert!(arguments_satisfy_schema(
            &json!({"anything": true}),
            &json!({"$ref": "https://example.com/not-resolvable.json"})
        ));
    }

    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_loop_support::CapabilityWriteResult;
    use ironclaw_turns::{
        InMemoryRunProfileResolver, LoopResultRef, RunProfileResolver, TurnRunId, TurnScope,
        run_profile::{
            CapabilityDescriptorView, ConcurrencyHint, ResolvedRunProfile,
            RunProfileResolutionRequest,
        },
    };

    struct SpyPort {
        definitions: Vec<ProviderToolDefinition>,
        surface_version: CapabilitySurfaceVersion,
        registered_calls: Mutex<Vec<ProviderToolCall>>,
        invocations: Mutex<Vec<CapabilityInvocation>>,
    }

    #[async_trait]
    impl LoopCapabilityPort for SpyPort {
        fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
            Ok(self.definitions.clone())
        }

        fn provider_tool_call_capability_ids(
            &self,
            tool_call: &ProviderToolCall,
        ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
            let definition = self
                .definitions
                .iter()
                .find(|definition| definition.name == tool_call.name)
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "provider tool call is outside the visible capability surface",
                    )
                })?;
            Ok(ProviderToolCallCapabilityIds::single(
                definition.capability_id.clone(),
            ))
        }

        fn validate_provider_tool_call(
            &self,
            tool_call: &ProviderToolCall,
        ) -> Result<(), AgentLoopHostError> {
            // Sentinel: lets a test drive the describe-first path by failing
            // pre-dispatch validation for a resolved target (mirrors the
            // `register_explodes` register-failure sentinel above).
            if tool_call
                .arguments
                .get("__force_invalid")
                .and_then(Value::as_bool)
                == Some(true)
            {
                return Err(invalid_invocation(
                    "spy validation rejects forced-invalid input",
                ));
            }
            self.provider_tool_call_capability_ids(tool_call)
                .map(|_| ())
        }

        async fn register_provider_tool_call(
            &self,
            request: RegisterProviderToolCallRequest,
        ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
            let RegisterProviderToolCallRequest {
                tool_call,
                activity_id,
            } = request;
            // Sentinel: lets tests drive the gateway's "register failed" arm.
            if tool_call.name.as_str() == "register_explodes" {
                return Err(invalid_invocation("spy register explodes"));
            }
            self.validate_provider_tool_call(&tool_call)?;
            self.registered_calls
                .lock()
                .expect("registered calls lock")
                .push(tool_call.clone());
            let definition = self
                .definitions
                .iter()
                .find(|definition| definition.name == tool_call.name)
                .expect("test target definition")
                .clone();
            Ok(CapabilityCallCandidate {
                activity_id: activity_id.unwrap_or_else(CapabilityActivityId::new),
                surface_version: self.surface_version.clone(),
                capability_id: definition.capability_id,
                input_ref: input_ref(format!("input:{}", tool_call.name)),
                effective_capability_ids: Vec::new(),
                provider_replay: Some(provider_replay_for(&tool_call, tool_call.name.clone())),
            })
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                callable_capability_ids: None,
                version: self.surface_version.clone(),
                descriptors: self
                    .definitions
                    .iter()
                    .map(|definition| CapabilityDescriptorView {
                        capability_id: definition.capability_id.clone(),
                        provider: None,
                        runtime: ironclaw_host_api::RuntimeKind::FirstParty,
                        safe_name: definition.name.to_string(),
                        safe_description: definition.description.clone(),
                        concurrency_hint: ConcurrencyHint::SafeForParallel,
                        parameters_schema: definition.parameters.clone(),
                    })
                    .collect(),
            })
        }

        async fn invoke_capability(
            &self,
            request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            // Sentinel: lets a test drive a gate (approval) suspension outcome.
            let suspends = request.capability_id.as_str() == "fixture.suspends";
            self.invocations
                .lock()
                .expect("invocations lock")
                .push(request);
            if suspends {
                return Ok(CapabilityOutcome::ApprovalRequired {
                    gate_ref: ironclaw_turns::LoopGateRef::new("gate:test")
                        .expect("valid gate ref"),
                    safe_summary: "approval needed".to_string(),
                    approval_resume: None,
                });
            }
            Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:target").expect("valid result ref"),
                safe_summary: "target completed".to_string(),
                progress: CapabilityProgress::MadeProgress,
                terminate_hint: false,
                byte_len: 2,
                output_digest: None,
                model_observation: None,
            }))
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            let mut outcomes = Vec::new();
            for invocation in request.invocations {
                outcomes.push(self.invoke_capability(invocation).await?);
            }
            Ok(CapabilityBatchOutcome {
                outcomes,
                stopped_on_suspension: false,
            })
        }
    }

    struct TestWriter;

    #[async_trait]
    impl LoopCapabilityResultWriter for TestWriter {
        async fn write_capability_result(
            &self,
            write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            let result_digest =
                ironclaw_host_api::sha256_digest_token(write.input_ref.as_str().as_bytes())
                    .replace(':', ".");
            Ok(CapabilityWriteResult::without_output_digest(
                LoopResultRef::new(format!("result:{result_digest}")).expect("valid result ref"),
                write.output.to_string().len() as u64,
            ))
        }
    }

    #[tokio::test]
    async fn search_discloses_tool_call_dispatches_target_and_promotes_next_turn() {
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition(
                "fixture.hidden",
                "hidden_tool",
                "Hidden workspace operation",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_scope),
        );

        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        assert!(
            !surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.safe_name == "hidden_tool"),
            "deferred tool should not be model-visible before discovery"
        );
        assert_eq!(
            surface
                .descriptors
                .iter()
                .find(|descriptor| descriptor.safe_name == "read_file")
                .expect("read_file descriptor")
                .concurrency_hint,
            ConcurrencyHint::SafeForParallel,
            "visible surface must preserve inner descriptor metadata"
        );
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            advertised
                .iter()
                .any(|definition| definition.name.as_str() == TOOL_SEARCH_NAME),
            "tool_search stays advertised as the discovery entry point"
        );
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == TOOL_CALL_NAME),
            "tool_call is no longer advertised (discovery is capability_info + direct call); \
             it still resolves internally for the forgiving path"
        );
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool")
        );

        // Forgiving `tool_call` resolution of an undisclosed catalog tool is
        // covered by `tool_call_resolves_undisclosed_catalog_target_forgivingly`.
        // This test focuses on the search -> disclose -> dispatch -> promote flow.

        let search = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_SEARCH_NAME,
                json!({"query": "hidden", "limit": 5}),
            )))
            .await
            .expect("search registers");
        let search_outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: search.activity_id,
                surface_version: search.surface_version,
                capability_id: search.capability_id,
                input_ref: search.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("search invokes");
        assert!(matches!(search_outcome, CapabilityOutcome::Completed(_)));

        let disclosed_surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface after search");
        assert!(
            disclosed_surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.safe_name == "hidden_tool"),
            "same-turn search should disclose target to the executor surface"
        );

        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            )))
            .await
            .expect("disclosed tool_call registers as target");
        assert_eq!(target.capability_id.as_str(), "fixture.hidden");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name
                .as_str(),
            TOOL_CALL_NAME
        );
        let batch = port
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: vec![CapabilityInvocation {
                    activity_id: target.activity_id,
                    surface_version: target.surface_version,
                    capability_id: target.capability_id,
                    input_ref: target.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                }],
                stop_on_first_suspension: true,
            })
            .await
            .expect("target batch invokes");
        assert!(matches!(
            batch.outcomes.as_slice(),
            [CapabilityOutcome::Completed(_)]
        ));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "hidden_tool"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "fixture.hidden"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        let next_advertised = next_turn.tool_definitions().expect("next tool definitions");
        assert!(
            next_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "successful deferred tool_call should promote the target on the next turn"
        );
    }

    #[tokio::test]
    async fn direct_deferred_catalog_tool_dispatches_target_and_promotes_next_turn() {
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition(
                "fixture.hidden",
                "hidden_tool",
                "Hidden workspace operation",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "hidden_tool starts deferred"
        );

        let direct_call = provider_call("hidden_tool", json!({"path": "demo"}));
        let capability_ids = port
            .provider_tool_call_capability_ids(&direct_call)
            .expect("direct deferred call resolves through inner");
        assert_eq!(
            capability_ids.provider_capability_id.as_str(),
            "fixture.hidden"
        );
        port.validate_provider_tool_call(&direct_call)
            .expect("direct deferred call validates through inner");
        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(direct_call))
            .await
            .expect("direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "fixture.hidden");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name
                .as_str(),
            "hidden_tool"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: target.activity_id,
                surface_version: target.surface_version,
                capability_id: target.capability_id,
                input_ref: target.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target invokes");
        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "hidden_tool"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "fixture.hidden"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        let next_advertised = next_turn.tool_definitions().expect("next tool definitions");
        assert!(
            next_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "successful direct deferred call should promote the target on the next turn"
        );
    }

    #[tokio::test]
    async fn capability_info_on_deferred_tool_promotes_it_for_direct_use_next_turn() {
        // Firat's discovery flow: tool_search (names) -> capability_info (loads +
        // promotes) -> direct call. Inspecting a deferred tool via capability_info
        // must disclose it this turn and promote it for the next, so it becomes
        // directly callable — without this the model inspects a tool it can never
        // reach and loops.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        assert!(
            !port
                .tool_definitions()
                .expect("tool definitions")
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "hidden_tool starts deferred"
        );

        // The model inspects the deferred tool by its canonical capability id.
        let inspect = provider_call("capability_info", json!({"name": "fixture.hidden"}));
        port.note_capability_info_target(&inspect)
            .expect("capability_info promotes the inspected target");

        // This turn the inspected tool is disclosed onto the callable surface
        // (visible_capabilities descriptors), so a call to it is authorized.
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface after inspect");
        assert!(
            surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id.as_str() == "fixture.hidden"),
            "capability_info discloses the inspected tool onto the surface this turn"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        assert!(
            next_turn
                .tool_definitions()
                .expect("next tool definitions")
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "capability_info promotes the inspected tool for the next turn"
        );
    }

    #[tokio::test]
    async fn undisclosed_invalid_deferred_call_returns_schema_instead_of_dispatching() {
        // The failure tool disclosure introduced: the model calls a deferred tool
        // whose schema it has not loaded, with arguments that fail validation (a
        // required field — e.g. an id — it does not have). Pre-disclosure the
        // schema was always in context; now it is deferred, so the model calls
        // blind and loops on the opaque schema error. Describe-first returns the
        // schema as a recoverable completion WITHOUT dispatching the target blind,
        // so the model's retry can be well-formed.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"__force_invalid": true}}),
            )))
            .await
            .expect("describe-first registers");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "an undisclosed invalid call must route to a schema (bridge) response, not the target"
        );
        assert!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .is_empty(),
            "describe-first must NOT register/dispatch the target on the inner port"
        );

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("describe-first invokes");
        assert!(
            matches!(outcome, CapabilityOutcome::Completed(_)),
            "describe-first returns the schema as a recoverable completion"
        );
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .is_empty(),
            "describe-first must NOT invoke the target on the inner port"
        );
    }

    #[tokio::test]
    async fn well_formed_blind_deferred_call_dispatches_without_describe_first() {
        // Describe-first must not tax correct calls: a blind call whose arguments
        // pass validation dispatches straight to the target (no wasted round-trip),
        // matching the zero-round-trip pre-disclosure behavior.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            )))
            .await
            .expect("valid blind call registers");
        assert_eq!(
            candidate.capability_id.as_str(),
            "fixture.hidden",
            "a well-formed blind call dispatches the target directly, not describe-first"
        );
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target registered")
                .name
                .as_str(),
            "hidden_tool"
        );
    }

    #[tokio::test]
    async fn describe_first_is_one_shot_so_repeated_failures_still_reach_dispatch() {
        // Backstop-safety: describe-first fires at most once per undisclosed tool.
        // After the schema is disclosed, a still-invalid call must dispatch (and
        // fail) through the normal path rather than returning a schema again —
        // otherwise a wedged model would receive an endless stream of
        // "made progress" schema responses and the no-progress detector would
        // never observe the repeated failure.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        // First invalid blind call -> describe-first (schema bridge), discloses.
        let first = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"__force_invalid": true}}),
            )))
            .await
            .expect("first registers");
        assert!(
            is_bridge_capability_id(&first.capability_id),
            "first undisclosed invalid call is describe-first"
        );
        port.invoke_capability(CapabilityInvocation {
            activity_id: first.activity_id,
            surface_version: first.surface_version,
            capability_id: first.capability_id,
            input_ref: first.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await
        .expect("first invokes (discloses schema)");

        // Second still-invalid call -> now disclosed, so it no longer intercepts:
        // it dispatches, the inner port rejects it, and a recoverable Failed
        // outcome (countable by the no-progress detector) surfaces — NOT a schema.
        let second = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"__force_invalid": true}}),
            )))
            .await
            .expect("second registers via recoverable fallback");
        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: second.activity_id,
                surface_version: second.surface_version,
                capability_id: second.capability_id,
                input_ref: second.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("second invokes");
        assert!(
            matches!(outcome, CapabilityOutcome::Failed(_)),
            "after disclosure a still-invalid call surfaces a Failed outcome the no-progress detector can count, not another schema"
        );
    }

    #[tokio::test]
    async fn direct_deferred_encoded_wire_name_records_canonical_wire_name_in_replay() {
        // Regression: a weak model calls a deferred provider tool by its canonical
        // `__`-encoded wire name (e.g. `google-calendar__list_events`, which
        // `tool_search`/`tool_describe` surface) before it is advertised. The
        // forgiving direct-deferred path resolves that, and the recorded provider
        // replay (consumed by the assistant transcript and any provider-error
        // result ref) MUST carry the canonical wire name so it serializes without
        // tripping `validate_provider_tool_name`. (The dotted capability_id form a
        // model might otherwise copy can no longer reach this port: `ProviderToolName`
        // excludes dots, so the gateway rejects such a call before it lands here.)
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition(
                "google-calendar.list_events",
                "google-calendar__list_events",
                "List Google Calendar events",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "google-calendar__list_events"),
            "deferred Google Calendar tool starts hidden"
        );

        // The model calls the deferred tool by its `__`-encoded wire name before
        // it is advertised.
        let deferred_call = provider_call("google-calendar__list_events", json!({"path": "demo"}));
        port.provider_tool_call_capability_ids(&deferred_call)
            .expect("deferred wire name resolves through forgiving path");
        port.validate_provider_tool_call(&deferred_call)
            .expect("deferred wire name validates through forgiving path");
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(deferred_call))
            .await
            .expect("deferred wire name registers as target");

        let replay = candidate.provider_replay.as_ref().expect("provider replay");
        assert_eq!(
            replay.provider_tool_name.as_str(),
            "google-calendar__list_events",
            "replay records the canonical wire name"
        );
        // The recorded name must serialize into the transcript without error.
        ironclaw_safety::validate_provider_tool_name(replay.provider_tool_name.as_str())
            .expect("recorded provider tool name is wire-safe");
    }

    #[tokio::test]
    async fn gate_suspended_target_is_promoted_so_it_survives_the_resume() {
        // Regression: a tool the model dispatched that paused on an approval/auth
        // gate must stay model-visible across the resume, exactly like a completed
        // dispatch. Otherwise the per-turn disclosed set resets on resume, the tool
        // drops off the surface, and the model's retry is hard-rejected by the
        // visible-surface filter ("outside the model-visible capability view") —
        // discarding the response and borking the run. Only *invoked* tools promote
        // (completed or gate-suspended), so a mere search/describe still does not,
        // and the advertised surface does not balloon toward "all tools".
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.suspends", "suspends_tool", "Needs approval"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        assert!(
            !port
                .tool_definitions()
                .expect("tool definitions")
                .iter()
                .any(|definition| definition.name.as_str() == "suspends_tool"),
            "suspends_tool starts deferred"
        );

        // Direct-deferred call -> resolves -> dispatch -> APPROVAL suspension.
        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                "suspends_tool",
                json!({"path": "demo"}),
            )))
            .await
            .expect("direct deferred call registers as target");
        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: target.activity_id,
                surface_version: target.surface_version,
                capability_id: target.capability_id,
                input_ref: target.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target invokes");
        assert!(
            outcome.is_suspension(),
            "the gate must suspend the call, not complete it"
        );

        // The resume is a fresh decorator instance (new turn state) sharing the
        // promoted store, exactly like the live BlockedApproval resume. The
        // gate-blocked tool must still be advertised.
        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        assert!(
            next_turn
                .tool_definitions()
                .expect("next tool definitions")
                .iter()
                .any(|definition| definition.name.as_str() == "suspends_tool"),
            "a gate-suspended tool must be promoted so it survives the resume"
        );
    }

    #[tokio::test]
    async fn callable_set_includes_advertised_bridges_so_the_visible_filter_keeps_them() {
        // Regression: callable_capability_ids was derived only from the inner
        // catalog, which excludes the synthesized bridges. The outer model-visible
        // filter is seeded from callable and strips any advertised tool not in it —
        // so the bridges (tool_search / tool_describe / tool_call) vanished from the
        // model's tool list and it could no longer discover anything ("tool_search
        // is not available"). Callable must be a superset of everything advertised
        // this turn AND still include the deferred long tail.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.hidden", "hidden_tool", "Hidden operation"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            advertised
                .iter()
                .any(|d| d.name.as_str() == TOOL_SEARCH_NAME),
            "fixture must be in deferred mode so the bridges are advertised"
        );
        let callable: std::collections::HashSet<_> = surface
            .callable_capability_ids
            .as_ref()
            .expect("disclosure narrows the surface, so callable set is populated")
            .iter()
            .cloned()
            .collect();
        // Every advertised tool — bridges included — must be authorizable, or the
        // visible-surface filter strips it from the model's tool list.
        for descriptor in &surface.descriptors {
            assert!(
                callable.contains(&descriptor.capability_id),
                "advertised tool {} missing from callable; the visible filter would strip it",
                descriptor.capability_id.as_str()
            );
        }
        // The deferred long tail stays callable (the original purpose of callable).
        assert!(
            callable.iter().any(|id| id.as_str() == "fixture.hidden"),
            "deferred catalog tool must remain callable"
        );
    }

    #[tokio::test]
    async fn direct_provider_encoded_builtin_dispatches_and_promotes() {
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("builtin.echo", "echo", "Echo the input"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "echo"),
            "echo starts deferred"
        );

        let direct_call = provider_call("builtin__echo", json!({"path": "demo"}));
        let capability_ids = port
            .provider_tool_call_capability_ids(&direct_call)
            .expect("provider-encoded direct deferred call resolves");
        assert_eq!(
            capability_ids.provider_capability_id.as_str(),
            "builtin.echo"
        );
        assert_eq!(
            capability_ids
                .effective_capability_ids
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["builtin.echo"]
        );
        port.validate_provider_tool_call(&direct_call)
            .expect("provider-encoded direct deferred call validates against resolved target");
        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(direct_call))
            .await
            .expect("provider-encoded direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "builtin.echo");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name
                .as_str(),
            "builtin__echo"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: target.activity_id,
                surface_version: target.surface_version,
                capability_id: target.capability_id,
                input_ref: target.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target invokes");
        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "echo",
            "inner registration must receive the catalog target name"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "builtin.echo"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        let next_advertised = next_turn.tool_definitions().expect("next tool definitions");
        assert!(
            next_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "echo"),
            "successful provider-encoded direct deferred call should promote the target next turn"
        );
    }

    #[tokio::test]
    async fn direct_provider_encoded_non_builtin_extension_tool_dispatches_and_promotes() {
        // Generality guard: the forgiving direct-deferred path must resolve ANY
        // deferred tool by its provider-encoded wire name, not just `builtin__*`.
        // Production sets `ProviderToolDefinition.name` to the encoded wire name
        // (`capability.provider_tool_name`, see capability_port surface_snapshot)
        // for every provider, and the catalog matches it by exact name. This
        // fixture mirrors that for a NON-builtin extension tool
        // (`gmail.send_message` -> wire `gmail__send_message`), so the resolution
        // cannot lean on the builtin-specific `strip_prefix("builtin__")` leniency
        // — if it did, this tool would fail "unresolved unadvertised" exactly like
        // the long tail of extension/MCP tools would in production.
        let definitions = vec![
            provider_definition("builtin.read_file", "builtin__read_file", "Read a file"),
            provider_definition(
                "gmail.send_message",
                "gmail__send_message",
                "Send an email via Gmail",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_scope),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "gmail__send_message"),
            "gmail__send_message starts deferred"
        );

        let direct_call = provider_call("gmail__send_message", json!({"path": "demo"}));
        let capability_ids = port
            .provider_tool_call_capability_ids(&direct_call)
            .expect("provider-encoded non-builtin direct deferred call resolves");
        assert_eq!(
            capability_ids.provider_capability_id.as_str(),
            "gmail.send_message"
        );
        assert_eq!(
            capability_ids
                .effective_capability_ids
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["gmail.send_message"]
        );
        port.validate_provider_tool_call(&direct_call)
            .expect("provider-encoded non-builtin direct deferred call validates against target");
        let target = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(direct_call))
            .await
            .expect("provider-encoded non-builtin direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "gmail.send_message");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name
                .as_str(),
            "gmail__send_message"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: target.activity_id,
                surface_version: target.surface_version,
                capability_id: target.capability_id,
                input_ref: target.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target invokes");
        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "gmail__send_message",
            "inner registration must receive the catalog target name"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "gmail.send_message"
        );

        let next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            promoted_by_scope,
        );
        next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("next visible surface");
        let next_advertised = next_turn.tool_definitions().expect("next tool definitions");
        assert!(
            next_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "gmail__send_message"),
            "successful non-builtin direct deferred call should promote the target next turn"
        );
    }

    #[tokio::test]
    async fn tool_call_resolves_undisclosed_catalog_target_forgivingly() {
        // Regression: a model (often a strong one) may invoke a catalog tool via
        // the `tool_call` bridge WITHOUT first discovering it through
        // tool_search/tool_describe. The bridge used to reject that with a generic
        // `invalid_input` ("unknown or not disclosed") carrying no recovery hint,
        // so the model looped on the same dead-end call until the run died. The
        // bridge must be no stricter than a direct call: an undisclosed catalog
        // tool resolves and dispatches to the target.
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition(
                "fixture.hidden",
                "hidden_tool",
                "Hidden workspace operation",
            ),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        let advertised = port.tool_definitions().expect("tool definitions");
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "hidden_tool starts deferred (never discovered this turn)"
        );

        // tool_call the deferred tool WITHOUT any prior tool_search/tool_describe.
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            )))
            .await
            .expect("undisclosed tool_call resolves forgivingly");
        assert_eq!(
            candidate.capability_id.as_str(),
            "fixture.hidden",
            "undisclosed tool_call must resolve to the catalog target, not the bridge"
        );

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("target dispatches");
        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .last()
                .expect("target call")
                .name
                .as_str(),
            "hidden_tool",
            "the inner port must receive the unwrapped target call"
        );
        assert_eq!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .last()
                .expect("target invocation")
                .capability_id
                .as_str(),
            "fixture.hidden"
        );
    }

    #[tokio::test]
    async fn tool_call_target_registration_failure_falls_back_to_recoverable_bridge_failure() {
        // Regression: the forgiving tool_call path resolves a deferred target, but
        // if the inner port then rejects it (e.g. malformed arguments), that must
        // surface as a RECOVERABLE invalid_input the model can retry — NOT a hard
        // error, which the gateway turns into a run-borking discard of the whole
        // provider response. (Observed live with gpt-5.5: repeated tool_call
        // validation rejections, run ending Failed / driver_protocol_violation.)
        let definitions = vec![
            provider_definition("fixture.read_file", "read_file", "Read a file"),
            provider_definition("fixture.explodes", "register_explodes", "Register fails"),
            provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
            provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
            provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
            provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");

        let bridge_call = provider_call(
            TOOL_CALL_NAME,
            json!({"name": "register_explodes", "arguments": {"path": "demo"}}),
        );
        // Validation must NOT hard-fail — that would abort the whole response.
        port.validate_provider_tool_call(&bridge_call)
            .expect("bridge validate downgrades a target failure to recoverable");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(bridge_call))
            .await
            .expect("bridge register falls back instead of erroring");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "a target that cannot register must fall back to the bridge path"
        );

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("bridge handles the fallback");
        assert!(
            matches!(
                outcome,
                CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    ..
                })
            ),
            "fallback must be a recoverable InvalidInput failure, not run death"
        );
    }

    #[tokio::test]
    async fn tool_call_targeting_a_bridge_is_rejected_without_dispatch() {
        // Recursion guard: tool_call(name = a bridge) must NOT re-enter the
        // bridge or dispatch anything — it is a model-recoverable failure.
        let inner = Arc::new(SpyPort {
            definitions: vec![provider_definition(
                "fixture.read_file",
                "read_file",
                "Read a file",
            )],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        // Build the surface first, as the real loop always does before a call.
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": TOOL_SEARCH_NAME, "arguments": {}}),
            )))
            .await
            .expect("recursive tool_call registers on the bridge path");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "recursive tool_call must stay on the bridge path, never resolve to a target"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("bridge handles recursion");
        assert!(
            matches!(
                outcome,
                CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    ..
                })
            ),
            "recursive tool_call must be a recoverable InvalidInput failure, not run death"
        );
        assert!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .is_empty(),
            "recursion must not register any target call on the inner port"
        );
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .is_empty(),
            "recursion must not dispatch to the inner port"
        );
    }

    #[tokio::test]
    async fn tool_call_targeting_unknown_tool_is_rejected_without_dispatch() {
        // Unknown-target guard: tool_call(name = not in catalog) must be a
        // model-recoverable failure and must not dispatch to the inner port.
        let inner = Arc::new(SpyPort {
            definitions: vec![provider_definition(
                "fixture.read_file",
                "read_file",
                "Read a file",
            )],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );

        // Build the surface first, as the real loop always does before a call.
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");

        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "does_not_exist", "arguments": {}}),
            )))
            .await
            .expect("unknown-target tool_call registers on the bridge path");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "unknown-target tool_call must stay on the bridge path"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("bridge handles unknown target");
        assert!(
            matches!(
                outcome,
                CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    ..
                })
            ),
            "unknown-target tool_call must be a recoverable InvalidInput failure"
        );
        assert!(
            inner
                .registered_calls
                .lock()
                .expect("registered calls lock")
                .is_empty(),
            "unknown target must not register any call on the inner port"
        );
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocations lock")
                .is_empty(),
            "unknown target must not dispatch to the inner port"
        );
    }

    #[tokio::test]
    async fn promotions_are_scoped_by_full_turn_scope_not_thread_only() {
        let inner = Arc::new(SpyPort {
            definitions: vec![
                provider_definition("fixture.read_file", "read_file", "Read a file"),
                provider_definition(
                    "fixture.hidden",
                    "hidden_tool",
                    "Hidden workspace operation",
                ),
                provider_definition("fixture.extra_1", "extra_tool_1", "Extra operation"),
                provider_definition("fixture.extra_2", "extra_tool_2", "Extra operation"),
                provider_definition("fixture.extra_3", "extra_tool_3", "Extra operation"),
                provider_definition("fixture.extra_4", "extra_tool_4", "Extra operation"),
            ],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_scope = Arc::new(Mutex::new(HashMap::new()));
        let tenant_a_first_turn = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            run_context_for(
                "tenant-a",
                "agent-tool-disclosure",
                "project-tool-disclosure",
                "shared-thread",
                TurnId::new(),
            )
            .await,
            Arc::clone(&promoted_by_scope),
        );
        tenant_a_first_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");
        let search = tenant_a_first_turn
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_SEARCH_NAME,
                json!({"query": "hidden", "limit": 5}),
            )))
            .await
            .expect("search registers");
        assert!(matches!(
            tenant_a_first_turn
                .invoke_capability(CapabilityInvocation {
                    activity_id: search.activity_id,
                    surface_version: search.surface_version,
                    capability_id: search.capability_id,
                    input_ref: search.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("search invokes"),
            CapabilityOutcome::Completed(_)
        ));
        let target = tenant_a_first_turn
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            )))
            .await
            .expect("target registers");
        assert!(matches!(
            tenant_a_first_turn
                .invoke_capability(CapabilityInvocation {
                    activity_id: target.activity_id,
                    surface_version: target.surface_version,
                    capability_id: target.capability_id,
                    input_ref: target.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("target invokes"),
            CapabilityOutcome::Completed(_)
        ));

        let tenant_b_next_turn = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context_for(
                "tenant-b",
                "agent-tool-disclosure",
                "project-tool-disclosure",
                "shared-thread",
                TurnId::new(),
            )
            .await,
            promoted_by_scope,
        );
        tenant_b_next_turn
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("tenant B surface builds");
        let tenant_b_advertised = tenant_b_next_turn
            .tool_definitions()
            .expect("tenant B tool definitions");
        assert!(
            !tenant_b_advertised
                .iter()
                .any(|definition| definition.name.as_str() == "hidden_tool"),
            "promotion from tenant A must not leak to tenant B with the same thread id"
        );
    }

    #[tokio::test]
    async fn tool_search_rejects_missing_non_string_or_blank_query() {
        let inner = Arc::new(SpyPort {
            definitions: vec![provider_definition(
                "fixture.read_file",
                "read_file",
                "Read a file",
            )],
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let port = disclosure_port(
            inner as Arc<dyn LoopCapabilityPort>,
            run_context(TurnId::new()).await,
            Arc::new(Mutex::new(HashMap::new())),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface builds turn state");

        for arguments in [
            json!({}),
            json!({"query": 42}),
            json!({"query": ""}),
            json!({"query": "   "}),
        ] {
            let candidate =
                port.register_provider_tool_call(RegisterProviderToolCallRequest::new(
                    provider_call(TOOL_SEARCH_NAME, arguments),
                ))
                .await
                .expect("tool_search registers");
            let outcome = port
                .invoke_capability(CapabilityInvocation {
                    activity_id: candidate.activity_id,
                    surface_version: candidate.surface_version,
                    capability_id: candidate.capability_id,
                    input_ref: candidate.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                })
                .await
                .expect("tool_search invokes");
            assert!(matches!(
                outcome,
                CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    ..
                })
            ));
        }
    }

    fn disclosure_port(
        inner: Arc<dyn LoopCapabilityPort>,
        run_context: LoopRunContext,
        promoted_by_scope: Arc<Mutex<HashMap<PromotionScopeKey, PromotedSet>>>,
    ) -> ToolDisclosureCapabilityPort {
        ToolDisclosureCapabilityPort {
            inner,
            run_context,
            result_writer: Arc::new(TestWriter),
            promoted_by_scope,
            caps: DisclosureCaps {
                max_tokens: u32::MAX,
                max_tools: 5,
                ctx_limit: None,
            },
            turn_state: Mutex::new(None),
            bridge_inputs: Mutex::new(BTreeMap::new()),
            tool_call_target_inputs: Mutex::new(BTreeMap::new()),
        }
    }

    async fn run_context(turn_id: TurnId) -> LoopRunContext {
        run_context_for(
            "tenant-tool-disclosure",
            "agent-tool-disclosure",
            "project-tool-disclosure",
            "thread-tool-disclosure",
            turn_id,
        )
        .await
    }

    async fn run_context_for(
        tenant: &str,
        agent: &str,
        project: &str,
        thread: &str,
        turn_id: TurnId,
    ) -> LoopRunContext {
        let tenant_id = TenantId::new(tenant).expect("valid tenant");
        let agent_id = AgentId::new(agent).expect("valid agent");
        let project_id = ProjectId::new(project).expect("valid project");
        let thread_id = ThreadId::new(thread).expect("valid thread");
        let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved: ResolvedRunProfile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("run profile resolves");
        LoopRunContext::new(turn_scope, turn_id, TurnRunId::new(), resolved)
    }

    fn provider_definition(
        capability_id: &str,
        name: &str,
        description: &str,
    ) -> ProviderToolDefinition {
        ProviderToolDefinition {
            capability_id: CapabilityId::new(capability_id).expect("valid capability id"),
            name: ProviderToolName::new(name).expect("valid provider tool name"),
            description: description.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn provider_call(name: &str, arguments: Value) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("provider-turn".to_string()),
            id: format!("call-{name}"),
            name: ProviderToolName::new(name).expect("valid provider tool name"),
            arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    fn input_ref(value: impl Into<String>) -> CapabilityInputRef {
        CapabilityInputRef::new(value.into()).expect("valid input ref")
    }
}
