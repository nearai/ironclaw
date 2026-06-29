//! Loop capability decorator for client-supplied ("external") tools.
//!
//! Mirrors [`super::synthetic_capability::LocalDevSyntheticCapabilityPort`] but,
//! instead of executing a synthetic capability, it *parks* the run and returns
//! control to the API client. The caller tool definitions come from the
//! per-run [`ExternalToolCatalog`] (registered by the OpenAI-compatible
//! Responses surface), so the model is offered the client's tools alongside the
//! agent's own capabilities. When the model calls one:
//!
//! - the first invocation finds no client-submitted output in the catalog and
//!   returns [`CapabilityOutcome::ExternalToolPending`] — the loop parks as
//!   `BlockedExternalTool` and the client is handed the function call;
//! - after the client submits the output (stored in the catalog by call id) and
//!   the run resumes, the re-dispatched invocation finds the output, writes it
//!   as the capability result, and returns [`CapabilityOutcome::Completed`] —
//!   the loop continues without ever executing the tool host-side.
//!
//! A client tool whose name shadows a host capability on the resolved surface is
//! rejected (coexistence with shadow-rejection), so a caller cannot silently
//! override a real capability.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, InvocationId, ProviderToolName, RuntimeKind};
use ironclaw_loop_support::{
    CapabilityResultWrite, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation, CapabilityBatchOutcome,
    CapabilityCallCandidate, CapabilityInvocation, CapabilityOutcome, CapabilityProgress,
    CapabilityResultMessage, CapabilitySurfaceVersion, ConcurrencyHint, LoopCapabilityPort,
    LoopRunContext, ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolCallReplay,
    ProviderToolDefinition, RegisterProviderToolCallRequest, VisibleCapabilityRequest,
    VisibleCapabilitySurface,
};
use ironclaw_turns::{ExternalToolCatalog, PendingExternalCall};
use ironclaw_turns::{LoopGateRef, TurnRunId};

/// Wrap `inner` so the per-run external tools in `catalog` are offered to the
/// model and parked (not executed) when called. Returns `inner` unchanged when
/// no external-tool capability could ever apply — the decorator itself is cheap
/// and fetches specs lazily at surface-resolution time, so it is always safe to
/// install.
pub(super) fn wrap_local_dev_external_tools(
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    catalog: Arc<dyn ExternalToolCatalog>,
) -> Arc<dyn LoopCapabilityPort> {
    Arc::new(ExternalToolCapabilityPort {
        inner,
        run_id: run_context.run_id,
        run_context,
        input_resolver,
        result_writer,
        catalog,
        surface: StdMutex::new(None),
    })
}

struct ResolvedSurface {
    version: CapabilitySurfaceVersion,
    specs_by_capability_id: HashMap<CapabilityId, ToolSpec>,
    capability_ids_by_tool_name: HashMap<ProviderToolName, CapabilityId>,
}

struct ToolSpec {
    tool_name: ProviderToolName,
    description: String,
    parameters_schema: serde_json::Value,
}

impl ToolSpec {
    fn descriptor_view(
        &self,
        capability_id: &CapabilityId,
    ) -> ironclaw_turns::run_profile::CapabilityDescriptorView {
        ironclaw_turns::run_profile::CapabilityDescriptorView {
            capability_id: capability_id.clone(),
            provider: None,
            runtime: RuntimeKind::System,
            safe_name: self.tool_name.as_str().to_string(),
            safe_description: self.description.clone(),
            // External tools are client-side; the host never runs them in
            // parallel, and they always park, so mark them exclusive.
            concurrency_hint: ConcurrencyHint::Exclusive,
            parameters_schema: self.parameters_schema.clone(),
        }
    }

    fn tool_definition(&self, capability_id: &CapabilityId) -> ProviderToolDefinition {
        ProviderToolDefinition::from_typed_parts(
            capability_id.clone(),
            self.tool_name.clone(),
            self.description.clone(),
            self.parameters_schema.clone(),
        )
    }
}

struct ExternalToolCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    run_id: TurnRunId,
    run_context: LoopRunContext,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    catalog: Arc<dyn ExternalToolCatalog>,
    surface: StdMutex<Option<ResolvedSurface>>,
}

/// Synthetic capability id for an external tool under the `external_tool.`
/// namespace.
fn external_tool_capability_id(
    provider_tool_name: &ProviderToolName,
) -> Result<CapabilityId, AgentLoopHostError> {
    CapabilityId::new(format!("external_tool.{}", provider_tool_name.as_str())).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "external tool name cannot be represented as a capability id",
        )
    })
}

fn provider_tool_name_for_external_tool(
    tool_name: &str,
) -> Result<ProviderToolName, AgentLoopHostError> {
    let provider_tool_name = tool_name.to_ascii_lowercase();
    ProviderToolDefinition::validate_name(&provider_tool_name).map_err(|error| {
        AgentLoopHostError::new(
            error.kind,
            format!(
                "external tool name cannot be represented as a provider tool name: {}",
                error.safe_summary
            ),
        )
    })
}

impl ExternalToolCapabilityPort {
    fn surface_version(&self) -> Result<CapabilitySurfaceVersion, AgentLoopHostError> {
        self.surface
            .lock()
            .map_err(|_| surface_lock_error())?
            .as_ref()
            .map(|surface| surface.version.clone())
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::StaleSurface,
                    "external tool capability surface is unavailable",
                )
            })
    }

    /// Whether `capability_id` is one this decorator owns (per the last resolved
    /// surface). Returns false (delegating to inner) when no surface is cached.
    fn owns_capability(&self, capability_id: &CapabilityId) -> bool {
        self.surface
            .lock()
            .ok()
            .and_then(|surface| {
                surface
                    .as_ref()
                    .map(|surface| surface.specs_by_capability_id.contains_key(capability_id))
            })
            .unwrap_or(false)
    }

    fn capability_id_for_tool_name(&self, tool_name: &ProviderToolName) -> Option<CapabilityId> {
        self.surface.lock().ok().and_then(|surface| {
            surface
                .as_ref()
                .and_then(|surface| surface.capability_ids_by_tool_name.get(tool_name).cloned())
        })
    }

    async fn complete_or_park(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if request.surface_version != self.surface_version()? {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "external tool call cites a stale capability surface",
            ));
        }
        let input_ref = request.input_ref.as_str().to_string();
        let call_id = self
            .catalog
            .call_id_for_input_ref(self.run_id, &input_ref)
            .await
            .map_err(catalog_error)?
            .unwrap_or_else(|| input_ref.clone());
        // Client already submitted the output → complete the parked call by
        // writing the output as the capability result (no host-side execution).
        if let Some(output) = self
            .catalog
            .output_for_input_ref(self.run_id, &input_ref)
            .await
            .map_err(catalog_error)?
        {
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
            // The parked call is resolved: drop its pending-call record so a run
            // that parks again on a later call does not re-surface this one.
            self.catalog
                .complete_call_for_input_ref(self.run_id, &input_ref)
                .await
                .map_err(catalog_error)?;
            return Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: write.result_ref,
                safe_summary: "external tool output".to_string(),
                progress: CapabilityProgress::MadeProgress,
                terminate_hint: false,
                byte_len: write.byte_len,
                output_digest: write.output_digest,
            }));
        }
        // No output yet → park and return control to the API client.
        Ok(CapabilityOutcome::ExternalToolPending {
            gate_ref: external_tool_gate_ref(&call_id)?,
            safe_summary: "awaiting client tool output".to_string(),
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for ExternalToolCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = self.inner_tool_definitions()?;
        let surface = self.surface.lock().map_err(|_| surface_lock_error())?;
        if let Some(surface) = surface.as_ref() {
            for (capability_id, spec) in &surface.specs_by_capability_id {
                if !definitions
                    .iter()
                    .any(|definition| &definition.capability_id == capability_id)
                {
                    definitions.push(spec.tool_definition(capability_id));
                }
            }
            definitions.sort_by(|left, right| left.name.cmp(&right.name));
        }
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if let Some(capability_id) = self.capability_id_for_tool_name(&tool_call.name) {
            return Ok(ProviderToolCallCapabilityIds::single(capability_id));
        }
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if self.capability_id_for_tool_name(&tool_call.name).is_some() {
            if tool_call.turn_id.is_none() {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "provider tool call is missing a provider turn id",
                ));
            }
            return Ok(());
        }
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let RegisterProviderToolCallRequest {
            tool_call,
            activity_id,
        } = request;
        let Some(capability_id) = self.capability_id_for_tool_name(&tool_call.name) else {
            return self
                .inner
                .register_provider_tool_call(RegisterProviderToolCallRequest {
                    tool_call,
                    activity_id,
                })
                .await;
        };
        self.validate_provider_tool_call(&tool_call)?;
        let provider_turn_id = tool_call.turn_id.clone().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is missing a provider turn id",
            )
        })?;
        let input_ref = self
            .input_resolver
            .register_provider_tool_call_input(&self.run_context, &tool_call)
            .await?;
        // Bind the loop input_ref to the client-facing provider call id so a
        // submitted output (keyed by call id) can be matched to this parked
        // invocation (keyed by input_ref) on the resume re-dispatch.
        self.catalog
            .bind_call(
                self.run_id,
                input_ref.as_str().to_string(),
                tool_call.id.clone(),
            )
            .await
            .map_err(catalog_error)?;
        // Record the call (name/arguments/call_id) so a parked
        // `BlockedExternalTool` run can render it as a `function_call` output
        // item — the loop checkpoint that also holds this data has no external
        // read path. Cleared in `complete_or_park` once the client output is
        // consumed on resume.
        self.catalog
            .record_pending_call(
                self.run_id,
                PendingExternalCall::new(
                    tool_call.id.clone(),
                    tool_call.name.as_str().to_string(),
                    tool_call.arguments.clone(),
                ),
            )
            .await
            .map_err(catalog_error)?;
        Ok(CapabilityCallCandidate {
            activity_id: activity_id.unwrap_or_default(),
            surface_version: self.surface_version()?,
            capability_id: capability_id.clone(),
            input_ref,
            effective_capability_ids: vec![capability_id],
            provider_replay: Some(ProviderToolCallReplay::from_tool_call(
                tool_call,
                provider_turn_id,
            )),
        })
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        let specs = self
            .catalog
            .specs(self.run_id)
            .await
            .map_err(catalog_error)?;

        let mut specs_by_capability_id = HashMap::new();
        let mut capability_ids_by_tool_name = HashMap::new();
        let mut descriptors = Vec::new();
        for spec in specs {
            let provider_tool_name = provider_tool_name_for_external_tool(spec.name())?;
            // Reject a client tool that shadows a host capability on the surface.
            if surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.safe_name == provider_tool_name.as_str())
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "external tool name shadows a host capability",
                ));
            }
            let capability_id = external_tool_capability_id(&provider_tool_name)?;
            if surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id == capability_id)
                || specs_by_capability_id.contains_key(&capability_id)
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "external tool conflicts with another capability id",
                ));
            }
            capability_ids_by_tool_name.insert(provider_tool_name.clone(), capability_id.clone());
            let tool_spec = ToolSpec {
                tool_name: provider_tool_name,
                description: spec.description().to_string(),
                parameters_schema: spec.parameters_schema().clone(),
            };
            descriptors.push(tool_spec.descriptor_view(&capability_id));
            specs_by_capability_id.insert(capability_id, tool_spec);
        }

        descriptors.sort_by(|left, right| left.safe_name.cmp(&right.safe_name));
        surface.descriptors.extend(descriptors);
        *self.surface.lock().map_err(|_| surface_lock_error())? = Some(ResolvedSurface {
            version: surface.version.clone(),
            specs_by_capability_id,
            capability_ids_by_tool_name,
        });
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if !self.owns_capability(&request.capability_id) {
            return self.inner.invoke_capability(request).await;
        }
        self.complete_or_park(request).await
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
            let is_external_tool_pending =
                matches!(&outcome, CapabilityOutcome::ExternalToolPending { .. });
            outcomes.push(outcome);
            if is_suspension && (request.stop_on_first_suspension || is_external_tool_pending) {
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

impl ExternalToolCapabilityPort {
    fn inner_tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.inner.tool_definitions()
    }
}

fn external_tool_gate_ref(call_id: &str) -> Result<LoopGateRef, AgentLoopHostError> {
    LoopGateRef::new(format!("gate:external_tool-{call_id}")).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "external tool gate ref could not be represented",
        )
    })
}

fn surface_lock_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Internal,
        "external tool capability surface lock failed",
    )
}

fn catalog_error(error: ironclaw_turns::ExternalToolCatalogError) -> AgentLoopHostError {
    match error {
        ironclaw_turns::ExternalToolCatalogError::Unavailable => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "external tool catalog is unavailable",
        ),
        ironclaw_turns::ExternalToolCatalogError::InvalidRegistration { reason } => {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                format!("external tool registration is invalid: {reason}"),
            )
        }
        ironclaw_turns::ExternalToolCatalogError::CallNotPending => AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "external tool call is not pending",
        ),
        ironclaw_turns::ExternalToolCatalogError::OutputAlreadySubmitted => {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "external tool output was already submitted",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_loop_support::CapabilityWriteResult;
    use ironclaw_turns::{
        ExternalToolCatalogError, ExternalToolSpec, InMemoryExternalToolCatalog,
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnScope,
        run_profile::{CapabilityInputRef, InMemoryRunProfileResolver},
    };

    struct EmptyInnerPort;

    #[async_trait]
    impl LoopCapabilityPort for EmptyInnerPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("test.surface.v1").expect("surface version"),
                descriptors: Vec::new(),
            })
        }

        async fn invoke_capability(
            &self,
            _request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "test inner port does not execute capabilities",
            ))
        }

        async fn invoke_capability_batch(
            &self,
            _request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "test inner port does not execute capability batches",
            ))
        }
    }

    struct TestInputResolver;

    #[async_trait]
    impl LoopCapabilityInputResolver for TestInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "test input resolver does not resolve inputs",
            ))
        }

        async fn register_provider_tool_call_input(
            &self,
            _run_context: &LoopRunContext,
            tool_call: &ProviderToolCall,
        ) -> Result<CapabilityInputRef, AgentLoopHostError> {
            CapabilityInputRef::new(format!("input:{}", tool_call.id)).map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "test provider tool-call input ref is invalid",
                )
            })
        }
    }

    struct TestResultWriter;

    #[async_trait]
    impl LoopCapabilityResultWriter for TestResultWriter {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "test result writer does not write results",
            ))
        }
    }

    struct FailingExternalToolCatalog {
        error: ExternalToolCatalogError,
    }

    #[async_trait]
    impl ExternalToolCatalog for FailingExternalToolCatalog {
        async fn register(
            &self,
            _run_id: TurnRunId,
            _specs: Vec<ExternalToolSpec>,
        ) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn specs(
            &self,
            _run_id: TurnRunId,
        ) -> Result<Vec<ExternalToolSpec>, ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn bind_call(
            &self,
            _run_id: TurnRunId,
            _input_ref: String,
            _call_id: String,
        ) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn call_id_for_input_ref(
            &self,
            _run_id: TurnRunId,
            _input_ref: &str,
        ) -> Result<Option<String>, ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn submit_output(
            &self,
            _run_id: TurnRunId,
            _call_id: String,
            _output: serde_json::Value,
        ) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn submit_output_for_pending_call(
            &self,
            _run_id: TurnRunId,
            _call_id: String,
            _output: serde_json::Value,
        ) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn output(
            &self,
            _run_id: TurnRunId,
            _call_id: &str,
        ) -> Result<Option<serde_json::Value>, ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn take_output(
            &self,
            _run_id: TurnRunId,
            _call_id: &str,
        ) -> Result<Option<serde_json::Value>, ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn complete_call_for_input_ref(
            &self,
            _run_id: TurnRunId,
            _input_ref: &str,
        ) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn record_pending_call(
            &self,
            _run_id: TurnRunId,
            _call: PendingExternalCall,
        ) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn pending_calls(
            &self,
            _run_id: TurnRunId,
        ) -> Result<Vec<PendingExternalCall>, ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn clear_pending_call(
            &self,
            _run_id: TurnRunId,
            _call_id: &str,
        ) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn clear(&self, _run_id: TurnRunId) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }
    }

    struct BindFailingExternalToolCatalog {
        error: ExternalToolCatalogError,
    }

    #[async_trait]
    impl ExternalToolCatalog for BindFailingExternalToolCatalog {
        async fn register(
            &self,
            _run_id: TurnRunId,
            _specs: Vec<ExternalToolSpec>,
        ) -> Result<(), ExternalToolCatalogError> {
            Ok(())
        }

        async fn specs(
            &self,
            _run_id: TurnRunId,
        ) -> Result<Vec<ExternalToolSpec>, ExternalToolCatalogError> {
            Ok(vec![external_tool_spec("ClientTool")])
        }

        async fn bind_call(
            &self,
            _run_id: TurnRunId,
            _input_ref: String,
            _call_id: String,
        ) -> Result<(), ExternalToolCatalogError> {
            Err(self.error.clone())
        }

        async fn call_id_for_input_ref(
            &self,
            _run_id: TurnRunId,
            _input_ref: &str,
        ) -> Result<Option<String>, ExternalToolCatalogError> {
            Ok(None)
        }

        async fn submit_output(
            &self,
            _run_id: TurnRunId,
            _call_id: String,
            _output: serde_json::Value,
        ) -> Result<(), ExternalToolCatalogError> {
            Ok(())
        }

        async fn submit_output_for_pending_call(
            &self,
            _run_id: TurnRunId,
            _call_id: String,
            _output: serde_json::Value,
        ) -> Result<(), ExternalToolCatalogError> {
            Ok(())
        }

        async fn output(
            &self,
            _run_id: TurnRunId,
            _call_id: &str,
        ) -> Result<Option<serde_json::Value>, ExternalToolCatalogError> {
            Ok(None)
        }

        async fn take_output(
            &self,
            _run_id: TurnRunId,
            _call_id: &str,
        ) -> Result<Option<serde_json::Value>, ExternalToolCatalogError> {
            Ok(None)
        }

        async fn complete_call_for_input_ref(
            &self,
            _run_id: TurnRunId,
            _input_ref: &str,
        ) -> Result<(), ExternalToolCatalogError> {
            Ok(())
        }

        async fn record_pending_call(
            &self,
            _run_id: TurnRunId,
            _call: PendingExternalCall,
        ) -> Result<(), ExternalToolCatalogError> {
            Ok(())
        }

        async fn pending_calls(
            &self,
            _run_id: TurnRunId,
        ) -> Result<Vec<PendingExternalCall>, ExternalToolCatalogError> {
            Ok(Vec::new())
        }

        async fn clear_pending_call(
            &self,
            _run_id: TurnRunId,
            _call_id: &str,
        ) -> Result<(), ExternalToolCatalogError> {
            Ok(())
        }

        async fn clear(&self, _run_id: TurnRunId) -> Result<(), ExternalToolCatalogError> {
            Ok(())
        }
    }

    async fn run_context() -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new("tenant-external-tools").expect("tenant id"),
                None,
                None,
                ThreadId::new("thread-external-tools").expect("thread id"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }

    fn external_tool_spec(name: &str) -> ExternalToolSpec {
        ExternalToolSpec::new(
            name,
            "client-side external tool",
            serde_json::json!({"type": "object"}),
        )
        .expect("external tool spec")
    }

    async fn wrapped_port_with_specs(
        specs: Vec<ExternalToolSpec>,
    ) -> (Arc<dyn LoopCapabilityPort>, LoopRunContext) {
        let run_context = run_context().await;
        let catalog = Arc::new(InMemoryExternalToolCatalog::new());
        catalog
            .register(run_context.run_id, specs)
            .await
            .expect("register external tools");
        let catalog: Arc<dyn ExternalToolCatalog> = catalog;
        (
            wrap_local_dev_external_tools(
                Arc::new(EmptyInnerPort),
                run_context.clone(),
                Arc::new(TestInputResolver),
                Arc::new(TestResultWriter),
                catalog,
            ),
            run_context,
        )
    }

    async fn wrapped_port_with_catalog(
        catalog: Arc<dyn ExternalToolCatalog>,
    ) -> (Arc<dyn LoopCapabilityPort>, LoopRunContext) {
        let run_context = run_context().await;
        (
            wrap_local_dev_external_tools(
                Arc::new(EmptyInnerPort),
                run_context.clone(),
                Arc::new(TestInputResolver),
                Arc::new(TestResultWriter),
                catalog,
            ),
            run_context,
        )
    }

    #[tokio::test]
    async fn external_tool_surface_maps_provider_name_to_capability_id() {
        let (port, _run_context) =
            wrapped_port_with_specs(vec![external_tool_spec("client_tool")]).await;

        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible capabilities");
        assert_eq!(surface.descriptors.len(), 1);
        assert_eq!(
            surface.descriptors[0].capability_id.as_str(),
            "external_tool.client_tool"
        );
        assert_eq!(surface.descriptors[0].safe_name, "client_tool");

        let definitions = port.tool_definitions().expect("tool definitions");
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name.as_str(), "client_tool");

        let ids = port
            .provider_tool_call_capability_ids(&ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("turn-1".to_string()),
                id: "call-1".to_string(),
                name: ProviderToolName::new("client_tool").expect("provider tool name"),
                arguments: serde_json::json!({}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            })
            .expect("capability ids");
        assert_eq!(
            ids.provider_capability_id.as_str(),
            "external_tool.client_tool"
        );
    }

    #[tokio::test]
    async fn invalid_catalog_registration_surfaces_as_invalid_invocation() {
        let (port, _run_context) =
            wrapped_port_with_catalog(Arc::new(FailingExternalToolCatalog {
                error: ExternalToolCatalogError::InvalidRegistration {
                    reason: "duplicate external tool name",
                },
            }))
            .await;

        let error = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect_err("invalid catalog registration should fail visible capabilities");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("duplicate external tool name"));
    }

    #[tokio::test]
    async fn invalid_catalog_bind_surfaces_as_invalid_invocation() {
        let (port, _run_context) =
            wrapped_port_with_catalog(Arc::new(BindFailingExternalToolCatalog {
                error: ExternalToolCatalogError::InvalidRegistration {
                    reason: "bind rejected external tool call",
                },
            }))
            .await;

        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible capabilities");

        let error = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("turn-1".to_string()),
                id: "call-1".to_string(),
                name: ProviderToolName::new("clienttool").expect("provider tool name"),
                arguments: serde_json::json!({}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .expect_err("invalid bind registration should fail provider tool-call registration");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            error
                .safe_summary
                .contains("bind rejected external tool call")
        );
    }

    #[tokio::test]
    async fn unavailable_catalog_surfaces_as_unavailable() {
        let (port, _run_context) =
            wrapped_port_with_catalog(Arc::new(FailingExternalToolCatalog {
                error: ExternalToolCatalogError::Unavailable,
            }))
            .await;

        let error = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect_err("unavailable catalog should fail visible capabilities");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(error.safe_summary, "external tool catalog is unavailable");
    }

    #[tokio::test]
    async fn external_tool_output_survives_result_write_failure() {
        let run_context = run_context().await;
        let catalog = Arc::new(InMemoryExternalToolCatalog::new());
        catalog
            .register(run_context.run_id, vec![external_tool_spec("get_weather")])
            .await
            .expect("register external tool");
        let port = wrap_local_dev_external_tools(
            Arc::new(EmptyInnerPort),
            run_context.clone(),
            Arc::new(TestInputResolver),
            Arc::new(TestResultWriter),
            catalog.clone(),
        );
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible capabilities");
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(ProviderToolCall {
                provider_id: "test-provider".to_string(),
                provider_model_id: "test-model".to_string(),
                turn_id: Some("turn-1".to_string()),
                id: "call-1".to_string(),
                name: ProviderToolName::new("get_weather").expect("provider tool name"),
                arguments: serde_json::json!({"city": "Boston"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }))
            .await
            .expect("register provider tool call");
        catalog
            .submit_output_for_pending_call(
                run_context.run_id,
                "call-1".to_string(),
                serde_json::json!("72F"),
            )
            .await
            .expect("submit output");

        let error = port
            .invoke_capability(CapabilityInvocation {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect_err("writer failure should propagate");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert_eq!(
            catalog
                .output(run_context.run_id, "call-1")
                .await
                .expect("output"),
            Some(serde_json::json!("72F"))
        );
        assert_eq!(
            catalog
                .pending_calls(run_context.run_id)
                .await
                .expect("pending")
                .len(),
            1
        );
    }
}
