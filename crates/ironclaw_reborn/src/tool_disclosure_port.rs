use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, InvocationId};
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
    TOOL_SEARCH_NAME, bridge_tool_definitions, is_bridge_capability_id, is_bridge_name,
    select_active_set, tool_search_rank,
};

const DISCLOSURE_INPUT_PREFIX: &str = "input:tool-disclosure:";

pub(crate) struct ToolDisclosureCapabilityDecorator {
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    promoted_by_thread: Arc<Mutex<HashMap<String, PromotedSet>>>,
    caps: DisclosureCaps,
}

impl ToolDisclosureCapabilityDecorator {
    pub(crate) fn new(result_writer: Arc<dyn LoopCapabilityResultWriter>) -> Self {
        Self {
            result_writer,
            promoted_by_thread: Arc::new(Mutex::new(HashMap::new())),
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
            promoted_by_thread: Arc::clone(&self.promoted_by_thread),
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
    promoted_by_thread: Arc<Mutex<HashMap<String, PromotedSet>>>,
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

#[async_trait]
impl LoopCapabilityPort for ToolDisclosureCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let state = self.turn_state()?;
        let Some(state) = state.as_ref() else {
            return Ok(Vec::new());
        };
        debug!(
            target: "ironclaw::reborn::context_shadow",
            advertised_tool_count = state.active.definitions.len(),
            deferred = state.active.deferred,
            advertised_tool_schema_tokens = state.active.advertised_tokens,
            "reborn live tool disclosure surface"
        );
        Ok(state.active.definitions.clone())
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if !is_bridge_name(&tool_call.name) {
            return self.inner.provider_tool_call_capability_ids(tool_call);
        }
        if tool_call.name == TOOL_CALL_NAME
            && let Ok(Some(target)) = self.allowed_tool_call_target(tool_call)
        {
            return Ok(ProviderToolCallCapabilityIds {
                provider_capability_id: target.capability_id.clone(),
                effective_capability_ids: vec![target.capability_id],
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
            return self.inner.validate_provider_tool_call(tool_call);
        }
        if matches!(
            tool_call.name.as_str(),
            TOOL_SEARCH_NAME | TOOL_DESCRIBE_NAME
        ) {
            return Ok(());
        }
        if tool_call.name == TOOL_CALL_NAME
            && let Ok(Some(target_call)) = self.synthetic_target_call(tool_call)
        {
            return self.inner.validate_provider_tool_call(&target_call);
        }
        Ok(())
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        if !is_bridge_name(&tool_call.name) {
            return self.inner.register_provider_tool_call(tool_call).await;
        }
        if tool_call.name == TOOL_CALL_NAME
            && let Ok(Some(target_call)) = self.synthetic_target_call(&tool_call)
        {
            let mut candidate = self.inner.register_provider_tool_call(target_call).await?;
            candidate.provider_replay = Some(provider_replay_for(&tool_call));
            self.tool_call_target_inputs
                .lock()
                .map_err(|_| invalid_invocation("tool_call target store lock is poisoned"))?
                .insert(
                    candidate.input_ref.as_str().to_string(),
                    candidate.capability_id.clone(),
                );
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
        let mut descriptors = state
            .catalog
            .active_or_disclosed_descriptors(&state.active, &state.disclosed_names);
        descriptors.retain(|descriptor| {
            surface
                .descriptors
                .iter()
                .any(|inner| inner.capability_id == descriptor.capability_id)
                || is_bridge_capability_id(&descriptor.capability_id)
        });
        surface.descriptors = descriptors;
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
                .map_err(|_| invalid_invocation("tool_call target store lock is poisoned"))?
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
        let mut guard = self.turn_state.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "tool disclosure turn state lock is poisoned",
            )
        })?;
        let rebuild = guard
            .as_ref()
            .map(|state| state.turn_id != self.run_context.turn_id)
            .unwrap_or(true);
        if rebuild {
            let definitions = self.inner.tool_definitions()?;
            let catalog = CapabilityCatalog::new(&definitions, &[]);
            let promoted = self.promoted_for_thread()?;
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

    fn promoted_for_thread(&self) -> Result<PromotedSet, AgentLoopHostError> {
        let key = self.run_context.thread_id.to_string();
        let guard = self.promoted_by_thread.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "tool disclosure promoted set lock is poisoned",
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
        let key = self.run_context.thread_id.to_string();
        let mut guard = self.promoted_by_thread.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "tool disclosure promoted set lock is poisoned",
            )
        })?;
        guard.entry(key).or_default().push(name);
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
        let digest_input = tool_call.arguments.to_string();
        let digest = ironclaw_host_api::sha256_digest_token(digest_input.as_bytes());
        let input_ref = CapabilityInputRef::new(format!("{DISCLOSURE_INPUT_PREFIX}{digest}"))
            .map_err(|_| invalid_invocation("bridge input ref could not be represented"))?;
        self.bridge_inputs
            .lock()
            .map_err(|_| invalid_invocation("bridge input store lock is poisoned"))?
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
            .map_err(|_| invalid_invocation("bridge input store lock is poisoned"))?
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
        let query = bridge
            .arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
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

    fn synthetic_target_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<Option<ProviderToolCall>, AgentLoopHostError> {
        let Some(target) = self.allowed_tool_call_target(tool_call)? else {
            return Ok(None);
        };
        let arguments = tool_call
            .arguments
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let digest_input = format!("{}\0{}", tool_call.id, target.name);
        let target_id = ironclaw_host_api::sha256_digest_token(digest_input.as_bytes());
        Ok(Some(ProviderToolCall {
            provider_id: tool_call.provider_id.clone(),
            provider_model_id: tool_call.provider_model_id.clone(),
            turn_id: tool_call.turn_id.clone(),
            id: format!("{}:{target_id}", tool_call.id),
            name: target.name,
            arguments,
            response_reasoning: tool_call.response_reasoning.clone(),
            reasoning: tool_call.reasoning.clone(),
            signature: tool_call.signature.clone(),
        }))
    }

    fn allowed_tool_call_target(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<Option<ProviderToolDefinition>, AgentLoopHostError> {
        let Some(name) = tool_call.arguments.get("name").and_then(Value::as_str) else {
            return Ok(None);
        };
        if is_bridge_name(name) {
            return Ok(None);
        }
        let guard = self.turn_state()?;
        let Some(state) = guard.as_ref() else {
            return Ok(None);
        };
        let Some(definition) = state.catalog.definition_by_name(name).cloned() else {
            return Ok(None);
        };
        let active = state
            .active
            .definitions
            .iter()
            .any(|candidate| candidate.name == name);
        if active || state.disclosed_names.contains(name) {
            Ok(Some(definition))
        } else {
            Ok(None)
        }
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

fn failed_invalid_input(summary: &'static str) -> CapabilityOutcome {
    CapabilityOutcome::Failed(CapabilityFailure {
        error_kind: CapabilityFailureKind::InvalidInput,
        safe_summary: summary.to_string(),
        detail: None,
    })
}

fn invalid_invocation(summary: &'static str) -> AgentLoopHostError {
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

        async fn register_provider_tool_call(
            &self,
            tool_call: ProviderToolCall,
        ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
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
            provider_definition("fixture.file_read", "file_read", "Read a file"),
            provider_definition(
                "fixture.hidden",
                "hidden_tool",
                "Hidden workspace operation",
            ),
        ];
        let inner = Arc::new(SpyPort {
            definitions,
            surface_version: CapabilitySurfaceVersion::new("surface:test")
                .expect("valid surface version"),
            registered_calls: Mutex::new(Vec::new()),
            invocations: Mutex::new(Vec::new()),
        });
        let promoted_by_thread = Arc::new(Mutex::new(HashMap::new()));
        let first_run_context = run_context(TurnId::new()).await;
        let port = disclosure_port(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            first_run_context,
            Arc::clone(&promoted_by_thread),
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
            promoted_by_thread,
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
    async fn tool_call_targeting_a_bridge_is_rejected_without_dispatch() {
        // Recursion guard: tool_call(name = a bridge) must NOT re-enter the
        // bridge or dispatch anything — it is a model-recoverable failure.
        let inner = Arc::new(SpyPort {
            definitions: vec![provider_definition(
                "fixture.file_read",
                "file_read",
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
                "fixture.file_read",
                "file_read",
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

    fn disclosure_port(
        inner: Arc<dyn LoopCapabilityPort>,
        run_context: LoopRunContext,
        promoted_by_thread: Arc<Mutex<HashMap<String, PromotedSet>>>,
    ) -> ToolDisclosureCapabilityPort {
        ToolDisclosureCapabilityPort {
            inner,
            run_context,
            result_writer: Arc::new(TestWriter),
            promoted_by_thread,
            caps: DisclosureCaps {
                max_tokens: 1,
                max_tools: 1,
                ctx_limit: None,
            },
            turn_state: Mutex::new(None),
            bridge_inputs: Mutex::new(BTreeMap::new()),
            tool_call_target_inputs: Mutex::new(BTreeMap::new()),
        }
    }

    async fn run_context(turn_id: TurnId) -> LoopRunContext {
        let tenant_id = TenantId::new("tenant-tool-disclosure").expect("valid tenant");
        let agent_id = AgentId::new("agent-tool-disclosure").expect("valid agent");
        let project_id = ProjectId::new("project-tool-disclosure").expect("valid project");
        let thread_id = ThreadId::new("thread-tool-disclosure").expect("valid thread");
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
