use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, CapabilityId, InvocationId, ProjectId, TenantId, ThreadId};
use ironclaw_loop_support::{
    CapabilityResultWrite, LoopCapabilityPortDecorator, LoopCapabilityResultWriter,
};
use ironclaw_turns::{
    TurnId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityCallCandidate, CapabilityFailure, CapabilityFailureKind,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilityProgress,
        CapabilityResultMessage, CapabilitySurfaceVersion, LoopCapabilityPort, LoopRunContext,
        ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolCallReplay,
        ProviderToolDefinition, VisibleCapabilityRequest, VisibleCapabilitySurface,
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
    surface_version: Option<CapabilitySurfaceVersion>,
    catalog: CapabilityCatalog,
    active: ActiveSet,
    disclosed_names: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct BridgeInvocation {
    name: String,
    arguments: Value,
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
        if !is_bridge_name(&tool_call.name) {
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
        if tool_call.name == TOOL_CALL_NAME
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
        if !is_bridge_name(&tool_call.name) {
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
        if tool_call.name == TOOL_CALL_NAME
            && let Some(target) = self.allowed_tool_call_target(tool_call)?
        {
            return self.inner.validate_provider_tool_call(&target.target_call);
        }
        Ok(())
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        if !is_bridge_name(&tool_call.name) {
            if let Some(target) = self.direct_deferred_target(&tool_call)? {
                debug!(
                    tool_name = tool_call.name.as_str(),
                    capability_id = target.definition.capability_id.as_str(),
                    "reborn tool disclosure registering direct deferred provider tool call"
                );
                let mut candidate = self
                    .inner
                    .register_provider_tool_call(target.target_call)
                    .await?;
                candidate.provider_replay = Some(provider_replay_for(&tool_call));
                self.record_promotable_input(
                    candidate.input_ref.as_str(),
                    candidate.capability_id.clone(),
                )?;
                return Ok(candidate);
            }
            return self.inner.register_provider_tool_call(tool_call).await;
        }
        if tool_call.name == TOOL_CALL_NAME
            && let Some(target) = self.allowed_tool_call_target(&tool_call)?
        {
            let mut candidate = self
                .inner
                .register_provider_tool_call(target.target_call)
                .await?;
            candidate.provider_replay = Some(provider_replay_for(&tool_call));
            self.record_promotable_input(
                candidate.input_ref.as_str(),
                candidate.capability_id.clone(),
            )?;
            return Ok(candidate);
        }
        self.register_bridge_call(tool_call)
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        let mut state = self.turn_state()?;
        let Some(state) = state.as_mut() else {
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
            if matches!(outcome, CapabilityOutcome::Completed(_))
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
        let rebuild = guard
            .as_ref()
            .map(|state| state.turn_id != self.run_context.turn_id)
            .unwrap_or(true);
        if rebuild {
            let definitions = self.inner.tool_definitions()?;
            let catalog = CapabilityCatalog::new(&definitions, &[]);
            let promoted = self.promoted_for_scope()?;
            let active = select_active_set(&catalog, &promoted, self.caps);
            *guard = Some(ToolDisclosureTurnState {
                turn_id: self.run_context.turn_id,
                surface_version: None,
                catalog,
                active,
                disclosed_names: BTreeSet::new(),
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
                .definition_by_name_for_capability(capability_id)
                .map(|definition| definition.name.clone())
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
        let digest_input =
            provider_call_digest_input(&tool_call.id, &tool_call.name, &tool_call.arguments);
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
                    name: tool_call.name.clone(),
                    arguments: tool_call.arguments.clone(),
                },
            );
        let surface_version = self.current_surface_version()?;
        Ok(CapabilityCallCandidate {
            surface_version,
            capability_id: definition.capability_id,
            input_ref,
            effective_capability_ids: Vec::new(),
            provider_replay: Some(provider_replay_for(&tool_call)),
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
        match bridge.name.as_str() {
            TOOL_SEARCH_NAME => self.invoke_tool_search(&request, &bridge).await,
            TOOL_DESCRIBE_NAME => self.invoke_tool_describe(&request, &bridge).await,
            TOOL_CALL_NAME => Ok(failed_invalid_input(
                "tool_call target is unknown or not disclosed",
            )),
            _ => Ok(failed_invalid_input("unknown bridge tool")),
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
            })
            .await?;
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: write.result_ref,
            safe_summary: safe_summary.to_string(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: write.byte_len,
            output_digest: write.output_digest,
        }))
    }

    fn target_call(
        &self,
        tool_call: &ProviderToolCall,
        target: &ProviderToolDefinition,
        arguments: Value,
    ) -> ProviderToolCall {
        let digest_input = provider_call_digest_input(&tool_call.id, &target.name, &arguments);
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
        let Some(definition) = self.catalog_target(state, name) else {
            return Ok(None);
        };
        let active = state
            .active
            .definitions
            .iter()
            .any(|candidate| candidate.capability_id == definition.capability_id);
        let disclosed = state.disclosed_names.contains(name)
            || state.disclosed_names.contains(&definition.name);
        if active || disclosed {
            let target_call = self.target_call(tool_call, &definition, arguments);
            Ok(Some(ResolvedToolTarget {
                definition,
                target_call,
            }))
        } else {
            Ok(None)
        }
    }

    fn direct_deferred_target(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<Option<ResolvedToolTarget>, AgentLoopHostError> {
        if is_bridge_name(&tool_call.name) {
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
        let Some(definition) = self.catalog_target(state, &tool_call.name) else {
            // DIAGNOSTIC (temporary): the model called a non-bridge tool that the
            // catalog could not resolve by name. Sample catalog names + capability
            // ids that share the called tool's provider prefix, so a name-form
            // mismatch (dotted vs `__`-encoded) is visible vs. genuinely absent.
            let prefix: String = tool_call
                .name
                .chars()
                .take_while(|c| *c != '_' && *c != '.' && *c != '-')
                .collect();
            let sample: Vec<String> = state
                .catalog
                .definitions()
                .filter(|definition| {
                    definition.name.starts_with(&prefix)
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

trait CatalogLookupByCapability {
    fn definition_by_name_for_capability(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&ProviderToolDefinition>;
}

impl CatalogLookupByCapability for CapabilityCatalog {
    fn definition_by_name_for_capability(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&ProviderToolDefinition> {
        self.definitions()
            .find(|definition| &definition.capability_id == capability_id)
    }
}

fn provider_replay_for(tool_call: &ProviderToolCall) -> ProviderToolCallReplay {
    ProviderToolCallReplay {
        provider_id: tool_call.provider_id.clone(),
        provider_model_id: tool_call.provider_model_id.clone(),
        provider_turn_id: tool_call.turn_id.clone().unwrap_or_default(),
        provider_call_id: tool_call.id.clone(),
        provider_tool_name: tool_call.name.clone(),
        arguments: tool_call.arguments.clone(),
        response_reasoning: tool_call.response_reasoning.clone(),
        reasoning: tool_call.reasoning.clone(),
        signature: tool_call.signature.clone(),
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
            self.provider_tool_call_capability_ids(tool_call)
                .map(|_| ())
        }

        async fn register_provider_tool_call(
            &self,
            tool_call: ProviderToolCall,
        ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
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
                surface_version: self.surface_version.clone(),
                capability_id: definition.capability_id,
                input_ref: input_ref(format!("input:{}", tool_call.name)),
                effective_capability_ids: Vec::new(),
                provider_replay: Some(provider_replay_for(&tool_call)),
            })
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                version: self.surface_version.clone(),
                descriptors: self
                    .definitions
                    .iter()
                    .map(|definition| CapabilityDescriptorView {
                        capability_id: definition.capability_id.clone(),
                        provider: None,
                        runtime: ironclaw_host_api::RuntimeKind::FirstParty,
                        safe_name: definition.name.clone(),
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
            self.invocations
                .lock()
                .expect("invocations lock")
                .push(request);
            Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:target").expect("valid result ref"),
                safe_summary: "target completed".to_string(),
                progress: CapabilityProgress::MadeProgress,
                terminate_hint: false,
                byte_len: 2,
                output_digest: None,
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
                .any(|definition| definition.name == TOOL_CALL_NAME)
        );
        assert!(
            !advertised
                .iter()
                .any(|definition| definition.name == "hidden_tool")
        );

        let undisclosed = port
            .register_provider_tool_call(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {}}),
            ))
            .await
            .expect("undisclosed tool_call registers as bridge failure");
        assert!(
            is_bridge_capability_id(&undisclosed.capability_id),
            "undisclosed tool_call should stay on bridge path"
        );
        let failed = port
            .invoke_capability(CapabilityInvocation {
                surface_version: undisclosed.surface_version,
                capability_id: undisclosed.capability_id,
                input_ref: undisclosed.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("bridge invalid failure");
        assert!(matches!(
            failed,
            CapabilityOutcome::Failed(CapabilityFailure {
                error_kind: CapabilityFailureKind::InvalidInput,
                ..
            })
        ));

        let search = port
            .register_provider_tool_call(provider_call(
                TOOL_SEARCH_NAME,
                json!({"query": "hidden", "limit": 5}),
            ))
            .await
            .expect("search registers");
        let search_outcome = port
            .invoke_capability(CapabilityInvocation {
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
            .register_provider_tool_call(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            ))
            .await
            .expect("disclosed tool_call registers as target");
        assert_eq!(target.capability_id.as_str(), "fixture.hidden");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name,
            TOOL_CALL_NAME
        );
        let batch = port
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: vec![CapabilityInvocation {
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
                .name,
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
                .any(|definition| definition.name == "hidden_tool"),
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
                .any(|definition| definition.name == "hidden_tool"),
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
            .register_provider_tool_call(direct_call)
            .await
            .expect("direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "fixture.hidden");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name,
            "hidden_tool"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
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
                .name,
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
                .any(|definition| definition.name == "hidden_tool"),
            "successful direct deferred call should promote the target on the next turn"
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
                .any(|definition| definition.name == "echo"),
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
            .register_provider_tool_call(direct_call)
            .await
            .expect("provider-encoded direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "builtin.echo");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name,
            "builtin__echo"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
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
                .name,
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
                .any(|definition| definition.name == "echo"),
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
                .any(|definition| definition.name == "gmail__send_message"),
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
            .register_provider_tool_call(direct_call)
            .await
            .expect("provider-encoded non-builtin direct deferred call registers as target");
        assert_eq!(target.capability_id.as_str(), "gmail.send_message");
        assert_eq!(
            target
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_tool_name,
            "gmail__send_message"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
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
                .name,
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
                .any(|definition| definition.name == "gmail__send_message"),
            "successful non-builtin direct deferred call should promote the target next turn"
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
            .register_provider_tool_call(provider_call(
                TOOL_CALL_NAME,
                json!({"name": TOOL_SEARCH_NAME, "arguments": {}}),
            ))
            .await
            .expect("recursive tool_call registers on the bridge path");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "recursive tool_call must stay on the bridge path, never resolve to a target"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
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
            .register_provider_tool_call(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "does_not_exist", "arguments": {}}),
            ))
            .await
            .expect("unknown-target tool_call registers on the bridge path");
        assert!(
            is_bridge_capability_id(&candidate.capability_id),
            "unknown-target tool_call must stay on the bridge path"
        );
        let outcome = port
            .invoke_capability(CapabilityInvocation {
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
            .register_provider_tool_call(provider_call(
                TOOL_SEARCH_NAME,
                json!({"query": "hidden", "limit": 5}),
            ))
            .await
            .expect("search registers");
        assert!(matches!(
            tenant_a_first_turn
                .invoke_capability(CapabilityInvocation {
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
            .register_provider_tool_call(provider_call(
                TOOL_CALL_NAME,
                json!({"name": "hidden_tool", "arguments": {"path": "demo"}}),
            ))
            .await
            .expect("target registers");
        assert!(matches!(
            tenant_a_first_turn
                .invoke_capability(CapabilityInvocation {
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
                .any(|definition| definition.name == "hidden_tool"),
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
            let candidate = port
                .register_provider_tool_call(provider_call(TOOL_SEARCH_NAME, arguments))
                .await
                .expect("tool_search registers");
            let outcome = port
                .invoke_capability(CapabilityInvocation {
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
            name: name.to_string(),
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
            name: name.to_string(),
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
