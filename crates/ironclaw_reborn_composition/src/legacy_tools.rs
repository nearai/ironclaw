//! Bench-only seam: expose externally-supplied tools (e.g. the nearai-bench
//! legacy `ironclaw::tools::Tool` service mocks — gmail/slack/calendar/sheets/
//! notion) to the reborn agent without authoring first-party extensions.
//!
//! The caller supplies (1) tool specs (name/description/JSON-params) and (2) a
//! dead-simple async `ExtraCapabilityDispatch::invoke(capability_id, args) ->
//! json`. We wrap the local-dev capability port in a decorator that advertises
//! those tools on the visible surface + in `tool_definitions`, and on invocation
//! resolves the staged input (via the shared `capability_io`), calls the
//! dispatch, and writes the result back through the same `capability_io` (so the
//! trajectory observer + transcript see it). Everything else delegates to the
//! inner local-dev port.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, InvocationId, RuntimeKind};
use ironclaw_loop_support::{
    CapabilityResultWrite, LoopCapabilityInputResolver, LoopCapabilityPortDecorator,
    LoopCapabilityResultWriter,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation, CapabilityBatchOutcome,
    CapabilityCallCandidate, CapabilityDescriptorView, CapabilityFailure, CapabilityFailureKind,
    CapabilityInvocation, CapabilityOutcome, CapabilityProgress, CapabilityResultMessage,
    ConcurrencyHint, LoopCapabilityPort, LoopRunContext, ProviderToolCall,
    ProviderToolCallCapabilityIds, ProviderToolCallReplay, ProviderToolDefinition,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};

/// One externally-supplied tool advertised to the reborn agent.
#[derive(Debug, Clone)]
pub struct ExtraToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Caller-provided execution of an extra tool: run `capability_id` with `input`
/// (the model's JSON arguments) and return the JSON result the model sees.
#[async_trait]
pub trait ExtraCapabilityDispatch: Send + Sync + std::fmt::Debug {
    async fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

/// Decorator factory: installed on `RebornRuntimeInput`, applied over the
/// local-dev port inside `capability_wiring` (which supplies the shared
/// resolver/writer).
#[derive(Clone)]
pub(crate) struct ExtraCapabilitiesDecorator {
    specs: Arc<Vec<ExtraToolSpec>>,
    dispatch: Arc<dyn ExtraCapabilityDispatch>,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
}

impl ExtraCapabilitiesDecorator {
    pub(crate) fn new(
        specs: Vec<ExtraToolSpec>,
        dispatch: Arc<dyn ExtraCapabilityDispatch>,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
    ) -> Self {
        Self {
            specs: Arc::new(specs),
            dispatch,
            input_resolver,
            result_writer,
        }
    }
}

impl LoopCapabilityPortDecorator for ExtraCapabilitiesDecorator {
    fn decorate(
        &self,
        run_context: &LoopRunContext,
        inner: Arc<dyn LoopCapabilityPort>,
    ) -> Arc<dyn LoopCapabilityPort> {
        Arc::new(ExtraCapabilitiesPort {
            inner,
            specs: Arc::clone(&self.specs),
            dispatch: Arc::clone(&self.dispatch),
            input_resolver: Arc::clone(&self.input_resolver),
            result_writer: Arc::clone(&self.result_writer),
            run_context: run_context.clone(),
        })
    }
}

struct ExtraCapabilitiesPort {
    inner: Arc<dyn LoopCapabilityPort>,
    specs: Arc<Vec<ExtraToolSpec>>,
    dispatch: Arc<dyn ExtraCapabilityDispatch>,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    run_context: LoopRunContext,
}

/// Bench tool names (gmail, notion_*, …) aren't valid dotted capability ids, so
/// we route them under a `bench.<name>` id (the model still sees the raw name).
fn cap_id(name: &str) -> CapabilityId {
    CapabilityId::new(format!("bench.{name}")).expect("bench.<name> is a valid capability id")
}

/// A capability result/failure `safe_summary` must be ≤512 bytes, contain no
/// control chars, and none of `{}[]`<>/\` (payload/path delimiters) — otherwise
/// the executor rejects it as an "unsafe strategy summary" and kills the whole
/// turn. Tool error strings routinely contain those, so sanitize.
fn safe_summary(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(480).collect();
    if truncated.is_empty() {
        "tool call".to_string()
    } else {
        truncated
    }
}

impl ExtraCapabilitiesPort {
    /// True if `name` (the model-facing tool name) is one of ours.
    fn is_mine_name(&self, name: &str) -> bool {
        self.specs.iter().any(|s| s.name == name)
    }

    /// Map a `bench.<name>` capability id back to its tool name, if it's ours.
    fn bench_name<'a>(&self, capability_id: &'a str) -> Option<&'a str> {
        capability_id
            .strip_prefix("bench.")
            .filter(|n| self.is_mine_name(n))
    }

    fn my_definitions(&self) -> Vec<ProviderToolDefinition> {
        self.specs
            .iter()
            .map(|s| ProviderToolDefinition {
                capability_id: cap_id(&s.name),
                name: s.name.clone(),
                description: s.description.clone(),
                parameters: s.parameters.clone(),
            })
            .collect()
    }
}

#[async_trait]
impl LoopCapabilityPort for ExtraCapabilitiesPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut defs = self.inner.tool_definitions()?;
        defs.extend(self.my_definitions());
        Ok(defs)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if self.is_mine_name(&tool_call.name) {
            return Ok(ProviderToolCallCapabilityIds::single(cap_id(&tool_call.name)));
        }
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if self.is_mine_name(&tool_call.name) {
            return Ok(());
        }
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        if !self.is_mine_name(&tool_call.name) {
            return self.inner.register_provider_tool_call(tool_call).await;
        }
        let capability_id = cap_id(&tool_call.name);
        let surface_version = self
            .visible_capabilities(VisibleCapabilityRequest)
            .await?
            .version;
        let input_ref = self
            .input_resolver
            .register_provider_tool_call_input(&self.run_context, &tool_call)
            .await?;
        Ok(CapabilityCallCandidate {
            surface_version,
            capability_id: capability_id.clone(),
            input_ref,
            effective_capability_ids: vec![capability_id],
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: tool_call.provider_id,
                provider_model_id: tool_call.provider_model_id,
                provider_turn_id: tool_call.turn_id.unwrap_or_default(),
                provider_call_id: tool_call.id,
                provider_tool_name: tool_call.name,
                arguments: tool_call.arguments,
                response_reasoning: tool_call.response_reasoning,
                reasoning: tool_call.reasoning,
                signature: tool_call.signature,
            }),
        })
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        for spec in self.specs.iter() {
            surface.descriptors.push(CapabilityDescriptorView {
                capability_id: cap_id(&spec.name),
                provider: None,
                runtime: RuntimeKind::System,
                safe_name: spec.name.clone(),
                safe_description: spec.description.clone(),
                // Service mocks mutate shared state — serialize them.
                concurrency_hint: ConcurrencyHint::Exclusive,
                parameters_schema: spec.parameters.clone(),
            });
        }
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let Some(tool_name) = self.bench_name(request.capability_id.as_str()).map(str::to_string)
        else {
            return self.inner.invoke_capability(request).await;
        };
        let input = self
            .input_resolver
            .resolve_capability_input(&self.run_context, &request.input_ref)
            .await?;
        let output = match self.dispatch.invoke(&tool_name, input).await {
            Ok(output) => output,
            Err(message) => {
                return Ok(CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::OperationFailed,
                    safe_summary: safe_summary(&format!("{tool_name} failed: {message}")),
                    detail: None,
                }));
            }
        };
        let (result_ref, byte_len) = self
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
            result_ref,
            safe_summary: safe_summary(&format!("{tool_name} returned")),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len,
        }))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::new();
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
