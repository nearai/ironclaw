use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ProviderToolName, RuntimeKind};
use ironclaw_loop_support::{LoopCapabilityInputResolver, LoopCapabilityResultWriter};
use ironclaw_turns::{
    CapabilityActivityId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityCallCandidate, CapabilityDescriptorView,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilitySurfaceVersion,
        ConcurrencyHint, LoopCapabilityPort, LoopRunContext, ProviderToolCall,
        ProviderToolCallCapabilityIds, ProviderToolCallReplay, ProviderToolDefinition,
        RegisterProviderToolCallRequest, VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};

pub(super) fn wrap_local_dev_synthetic_capabilities(
    inner: Arc<dyn LoopCapabilityPort>,
    capabilities: Vec<LocalDevSyntheticCapability>,
    run_context: LoopRunContext,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
    if capabilities.is_empty() {
        return Ok(inner);
    }
    Ok(Arc::new(LocalDevSyntheticCapabilityPort::new(
        inner,
        capabilities,
        run_context,
        input_resolver,
        result_writer,
        trajectory_observer,
    )?))
}

pub(super) struct LocalDevSyntheticCapability {
    descriptor: LocalDevSyntheticCapabilityDescriptor,
    handler: Arc<dyn LocalDevSyntheticCapabilityHandler>,
}

impl LocalDevSyntheticCapability {
    pub(super) fn new(
        descriptor: LocalDevSyntheticCapabilityDescriptor,
        handler: Arc<dyn LocalDevSyntheticCapabilityHandler>,
    ) -> Self {
        Self {
            descriptor,
            handler,
        }
    }
}

pub(super) struct LocalDevSyntheticCapabilityDescriptor {
    capability_id: CapabilityId,
    provider_tool_name: ProviderToolName,
    description: String,
    concurrency_hint: ConcurrencyHint,
    parameters_schema: serde_json::Value,
}

impl LocalDevSyntheticCapabilityDescriptor {
    pub(super) fn new(
        capability_id: &str,
        provider_tool_name: &str,
        description: &str,
        concurrency_hint: ConcurrencyHint,
        parameters_schema: serde_json::Value,
    ) -> Result<Self, AgentLoopHostError> {
        Ok(Self {
            capability_id: CapabilityId::new(capability_id).map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "synthetic capability id is invalid",
                )
            })?,
            provider_tool_name: ProviderToolName::new(provider_tool_name).map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "synthetic provider tool name is invalid",
                )
            })?,
            description: description.to_string(),
            concurrency_hint,
            parameters_schema,
        })
    }

    fn descriptor_view(&self) -> CapabilityDescriptorView {
        CapabilityDescriptorView {
            capability_id: self.capability_id.clone(),
            provider: None,
            runtime: RuntimeKind::System,
            safe_name: self.provider_tool_name.as_str().to_string(),
            safe_description: self.description.clone(),
            concurrency_hint: self.concurrency_hint,
            parameters_schema: self.parameters_schema.clone(),
        }
    }

    fn tool_definition(&self) -> ProviderToolDefinition {
        ProviderToolDefinition {
            capability_id: self.capability_id.clone(),
            name: self.provider_tool_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters_schema.clone(),
        }
    }
}

pub(super) struct LocalDevSyntheticCapabilityInvocation {
    pub(super) run_context: LoopRunContext,
    pub(super) request: CapabilityInvocation,
    pub(super) input: serde_json::Value,
    pub(super) result_writer: Arc<dyn LoopCapabilityResultWriter>,
}

#[async_trait]
pub(super) trait LocalDevSyntheticCapabilityHandler: Send + Sync {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError>;

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError>;
}

struct LocalDevSyntheticCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    result_writer: Arc<dyn LoopCapabilityResultWriter>,
    capabilities_by_id: HashMap<CapabilityId, LocalDevSyntheticCapability>,
    capability_ids_by_provider_tool_name: HashMap<ProviderToolName, CapabilityId>,
    current_surface_version: StdMutex<Option<CapabilitySurfaceVersion>>,
    provider_tool_call_registrations:
        StdMutex<HashMap<String, SyntheticProviderToolCallRegistration>>,
    /// Synthetic calls resolve input + write the result here, bypassing the
    /// inner `HostRuntimeLoopCapabilityPort` input hook. Hold the observer so we
    /// can emit `on_capability_input` ourselves — otherwise consumers see the
    /// result event (from `LocalDevCapabilityIo`) with no matching input.
    trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
}

struct SyntheticProviderToolCallRegistration {
    activity_id: CapabilityActivityId,
    capability_id: CapabilityId,
}

impl LocalDevSyntheticCapabilityPort {
    fn new(
        inner: Arc<dyn LoopCapabilityPort>,
        capabilities: Vec<LocalDevSyntheticCapability>,
        run_context: LoopRunContext,
        input_resolver: Arc<dyn LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
        trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    ) -> Result<Self, AgentLoopHostError> {
        let mut capabilities_by_id = HashMap::new();
        let mut capability_ids_by_provider_tool_name = HashMap::new();
        for capability in capabilities {
            let capability_id = capability.descriptor.capability_id.clone();
            let provider_tool_name = capability.descriptor.provider_tool_name.clone();
            if capabilities_by_id
                .insert(capability_id.clone(), capability)
                .is_some()
                || capability_ids_by_provider_tool_name
                    .insert(provider_tool_name, capability_id)
                    .is_some()
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "duplicate synthetic capability registration",
                ));
            }
        }
        Ok(Self {
            inner,
            run_context,
            input_resolver,
            result_writer,
            capabilities_by_id,
            capability_ids_by_provider_tool_name,
            current_surface_version: StdMutex::new(None),
            provider_tool_call_registrations: StdMutex::new(HashMap::new()),
            trajectory_observer,
        })
    }

    fn current_surface_version(&self) -> Result<CapabilitySurfaceVersion, AgentLoopHostError> {
        self.current_surface_version
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "synthetic capability surface lock failed",
                )
            })?
            .clone()
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::StaleSurface,
                    "capability surface is unavailable",
                )
            })
    }

    fn synthetic_provider_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Option<(&CapabilityId, &LocalDevSyntheticCapability)> {
        self.capability_ids_by_provider_tool_name
            .get(&tool_call.name)
            .and_then(|capability_id| self.capabilities_by_id.get_key_value(capability_id))
    }

    async fn register_synthetic_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
        activity_id: Option<CapabilityActivityId>,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let Some((capability_id, _)) = self.synthetic_provider_call(&tool_call) else {
            return self
                .inner
                .register_provider_tool_call(RegisterProviderToolCallRequest {
                    tool_call,
                    activity_id,
                })
                .await;
        };
        let capability_id = capability_id.clone();
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
        let activity_id =
            self.record_provider_tool_call_registration(&input_ref, &capability_id, activity_id)?;
        Ok(CapabilityCallCandidate {
            activity_id,
            surface_version: self.current_surface_version()?,
            capability_id: capability_id.clone(),
            input_ref,
            effective_capability_ids: vec![capability_id.clone()],
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: tool_call.provider_id,
                provider_model_id: tool_call.provider_model_id,
                provider_turn_id,
                provider_call_id: tool_call.id,
                provider_tool_name: tool_call.name,
                arguments: tool_call.arguments,
                response_reasoning: tool_call.response_reasoning,
                reasoning: tool_call.reasoning,
                signature: tool_call.signature,
            }),
        })
    }

    fn record_provider_tool_call_registration(
        &self,
        input_ref: &CapabilityInputRef,
        capability_id: &CapabilityId,
        activity_id: Option<CapabilityActivityId>,
    ) -> Result<CapabilityActivityId, AgentLoopHostError> {
        let mut registrations = self.provider_tool_call_registrations.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "synthetic provider tool-call registration store lock failed",
            )
        })?;
        let record = registrations
            .entry(input_ref.as_str().to_string())
            .or_insert_with(|| SyntheticProviderToolCallRegistration {
                activity_id: activity_id.unwrap_or_default(),
                capability_id: capability_id.clone(),
            });
        if record.capability_id != *capability_id {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool-call capability identity changed",
            ));
        }
        if let Some(activity_id) = activity_id
            && record.activity_id != activity_id
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool-call activity identity changed",
            ));
        }
        Ok(record.activity_id)
    }

    fn validate_provider_tool_call_registration_activity(
        &self,
        input_ref: &CapabilityInputRef,
        activity_id: CapabilityActivityId,
    ) -> Result<(), AgentLoopHostError> {
        let registrations = self.provider_tool_call_registrations.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "synthetic provider tool-call registration store lock failed",
            )
        })?;
        if let Some(registration) = registrations.get(input_ref.as_str())
            && registration.activity_id != activity_id
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "registered provider tool-call activity identity does not match the requested activity",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl LoopCapabilityPort for LocalDevSyntheticCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = self.inner.tool_definitions()?;
        if self
            .current_surface_version
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "synthetic capability surface lock failed",
                )
            })?
            .is_none()
        {
            return Ok(definitions);
        }
        for capability in self.capabilities_by_id.values() {
            if !definitions
                .iter()
                .any(|definition| definition.capability_id == capability.descriptor.capability_id)
            {
                definitions.push(capability.descriptor.tool_definition());
            }
        }
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if let Some((capability_id, _)) = self.synthetic_provider_call(tool_call) {
            return Ok(ProviderToolCallCapabilityIds::single(capability_id.clone()));
        }
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if let Some((_, capability)) = self.synthetic_provider_call(tool_call) {
            capability
                .handler
                .validate_provider_arguments(&tool_call.arguments)?;
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
        if self.synthetic_provider_call(&tool_call).is_some() {
            return self
                .register_synthetic_provider_tool_call(tool_call, activity_id)
                .await;
        }
        self.inner
            .register_provider_tool_call(RegisterProviderToolCallRequest {
                tool_call,
                activity_id,
            })
            .await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        for capability_id in self.capabilities_by_id.keys() {
            if surface
                .descriptors
                .iter()
                .any(|descriptor| &descriptor.capability_id == capability_id)
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "synthetic capability conflicts with runtime capability surface",
                ));
            }
        }
        *self.current_surface_version.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "synthetic capability surface lock failed",
            )
        })? = Some(surface.version.clone());
        let mut synthetic_descriptors = self
            .capabilities_by_id
            .values()
            .map(|capability| capability.descriptor.descriptor_view())
            .collect::<Vec<_>>();
        synthetic_descriptors.sort_by(|left, right| left.safe_name.cmp(&right.safe_name));
        surface.descriptors.extend(synthetic_descriptors);
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let Some(capability) = self.capabilities_by_id.get(&request.capability_id) else {
            return self.inner.invoke_capability(request).await;
        };
        let handler = Arc::clone(&capability.handler);
        if request.surface_version != self.current_surface_version()? {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "synthetic capability call cites a stale capability surface",
            ));
        }
        if request.approval_resume.is_some() && request.auth_resume.is_some() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability invocation has both approval_resume and auth_resume set; \
                 these resume modes are mutually exclusive",
            ));
        }
        let effective_input_ref = request
            .approval_resume
            .as_ref()
            .map(|resume| &resume.input_ref)
            .unwrap_or(&request.input_ref);
        self.validate_provider_tool_call_registration_activity(
            effective_input_ref,
            request.activity_id,
        )?;
        let input = match request.approval_resume.as_ref() {
            Some(resume) => resume.input.clone(),
            None => {
                self.input_resolver
                    .resolve_capability_input(&self.run_context, &request.input_ref)
                    .await?
            }
        };
        // The inner port's input hook is bypassed for synthetic capabilities, so
        // emit the input event here — otherwise `LocalDevCapabilityIo` would stage
        // a result with no matching input and consumers would see an unpaired
        // event missing the tool arguments. Best-effort + panic-isolated, matching
        // the other observer call sites.
        if let Some(observer) = &self.trajectory_observer {
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                observer.on_capability_input(
                    request.input_ref.as_str(),
                    request.capability_id.as_str(),
                    &input,
                );
            }));
            if caught.is_err() {
                tracing::warn!(
                    capability_id = request.capability_id.as_str(),
                    "trajectory observer on_capability_input panicked; dropping event"
                );
            }
        }
        handler
            .invoke(LocalDevSyntheticCapabilityInvocation {
                run_context: self.run_context.clone(),
                request,
                input,
                result_writer: Arc::clone(&self.result_writer),
            })
            .await
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

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_loop_support::{
        CapabilityResultWrite, CapabilityWriteResult, EmptyLoopCapabilityPort,
    };
    use ironclaw_turns::{
        LoopResultRef, RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId,
        TurnScope,
        run_profile::{InMemoryRunProfileResolver, VisibleCapabilityRequest},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TEST_CAPABILITY_ID: &str = "test.synthetic";
    const TEST_PROVIDER_TOOL_NAME: &str = "test__synthetic";

    struct FixedInputResolver {
        input_ref: CapabilityInputRef,
        input: serde_json::Value,
    }

    #[async_trait]
    impl LoopCapabilityInputResolver for FixedInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(self.input.clone())
        }

        async fn register_provider_tool_call_input(
            &self,
            _run_context: &LoopRunContext,
            _tool_call: &ProviderToolCall,
        ) -> Result<CapabilityInputRef, AgentLoopHostError> {
            Ok(self.input_ref.clone())
        }
    }

    struct NoopResultWriter;

    #[async_trait]
    impl LoopCapabilityResultWriter for NoopResultWriter {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            Ok(CapabilityWriteResult::without_output_digest(
                LoopResultRef::new("result:synthetic-test").expect("valid result ref"),
                0,
            ))
        }
    }

    struct CountingResultWriter {
        writes: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for CountingResultWriter {
        async fn write_capability_result(
            &self,
            _write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(CapabilityWriteResult::without_output_digest(
                LoopResultRef::new("result:synthetic-counting").expect("valid result ref"),
                0,
            ))
        }
    }

    struct TestSyntheticHandler;

    #[async_trait]
    impl LocalDevSyntheticCapabilityHandler for TestSyntheticHandler {
        fn validate_provider_arguments(
            &self,
            _arguments: &serde_json::Value,
        ) -> Result<(), AgentLoopHostError> {
            Ok(())
        }

        async fn invoke(
            &self,
            _invocation: LocalDevSyntheticCapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "test handler should not be invoked",
            ))
        }
    }

    struct CountingSyntheticHandler {
        invocations: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LocalDevSyntheticCapabilityHandler for CountingSyntheticHandler {
        fn validate_provider_arguments(
            &self,
            _arguments: &serde_json::Value,
        ) -> Result<(), AgentLoopHostError> {
            Ok(())
        }

        async fn invoke(
            &self,
            _invocation: LocalDevSyntheticCapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            self.invocations.fetch_add(1, Ordering::SeqCst);
            Ok(CapabilityOutcome::Completed(
                ironclaw_turns::run_profile::CapabilityResultMessage {
                    result_ref: LoopResultRef::new("result:synthetic-handler")
                        .expect("valid result ref"),
                    safe_summary: "synthetic handler completed".to_string(),
                    progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
                    terminate_hint: false,
                    byte_len: 0,
                    output_digest: None,
                    model_observation: None,
                },
            ))
        }
    }

    async fn run_context() -> LoopRunContext {
        let profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new("tenant-synthetic-registration").expect("tenant id"),
                Some(AgentId::new("agent-synthetic-registration").expect("agent id")),
                Some(ProjectId::new("project-synthetic-registration").expect("project id")),
                ThreadId::new("thread-synthetic-registration").expect("thread id"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            profile,
        )
    }

    async fn synthetic_port() -> LocalDevSyntheticCapabilityPort {
        synthetic_port_with_io(Arc::new(TestSyntheticHandler), Arc::new(NoopResultWriter)).await
    }

    async fn synthetic_port_with_io(
        handler: Arc<dyn LocalDevSyntheticCapabilityHandler>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
    ) -> LocalDevSyntheticCapabilityPort {
        let capability = LocalDevSyntheticCapability::new(
            LocalDevSyntheticCapabilityDescriptor::new(
                TEST_CAPABILITY_ID,
                TEST_PROVIDER_TOOL_NAME,
                "Synthetic test capability",
                ConcurrencyHint::SafeForParallel,
                serde_json::json!({"type": "object"}),
            )
            .expect("descriptor"),
            handler,
        );
        let port = LocalDevSyntheticCapabilityPort::new(
            Arc::new(EmptyLoopCapabilityPort),
            vec![capability],
            run_context().await,
            Arc::new(FixedInputResolver {
                input_ref: CapabilityInputRef::new("input:synthetic-provider-call")
                    .expect("input ref"),
                input: serde_json::json!({"message": "hello"}),
            }),
            result_writer,
            None,
        )
        .expect("synthetic port");
        port.visible_capabilities(VisibleCapabilityRequest {})
            .await
            .expect("visible surface");
        port
    }

    fn provider_tool_call() -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            turn_id: Some("provider-turn-1".to_string()),
            id: "provider-call-1".to_string(),
            name: ProviderToolName::new(TEST_PROVIDER_TOOL_NAME).expect("provider tool name"),
            arguments: serde_json::json!({"message": "hello"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    fn different_activity_id(activity_id: CapabilityActivityId) -> CapabilityActivityId {
        loop {
            let candidate = CapabilityActivityId::new();
            if candidate != activity_id {
                return candidate;
            }
        }
    }

    #[tokio::test]
    async fn duplicate_synthetic_provider_call_reuses_activity_id_for_same_input_ref() {
        let port = synthetic_port().await;
        let first = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_tool_call()))
            .await
            .expect("first registration");
        let second = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_tool_call()))
            .await
            .expect("duplicate registration");

        assert_eq!(second.input_ref, first.input_ref);
        assert_eq!(second.activity_id, first.activity_id);
    }

    #[tokio::test]
    async fn synthetic_provider_call_rejects_explicit_activity_id_change_for_same_input_ref() {
        let port = synthetic_port().await;
        let activity_id = CapabilityActivityId::new();
        let first = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::for_activity(
                provider_tool_call(),
                activity_id,
            ))
            .await
            .expect("first registration");
        let error = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::for_activity(
                provider_tool_call(),
                different_activity_id(activity_id),
            ))
            .await
            .expect_err("activity id changes must be rejected");

        assert_eq!(first.activity_id, activity_id);
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            error.safe_summary.contains("activity identity"),
            "error should name the activity identity mismatch: {:?}",
            error.safe_summary
        );
    }

    #[tokio::test]
    async fn synthetic_provider_call_rejects_invocation_activity_mismatch_before_dispatch() {
        let handler_invocations = Arc::new(AtomicUsize::new(0));
        let result_writes = Arc::new(AtomicUsize::new(0));
        let port = synthetic_port_with_io(
            Arc::new(CountingSyntheticHandler {
                invocations: Arc::clone(&handler_invocations),
            }),
            Arc::new(CountingResultWriter {
                writes: Arc::clone(&result_writes),
            }),
        )
        .await;
        let activity_id = CapabilityActivityId::new();
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::for_activity(
                provider_tool_call(),
                activity_id,
            ))
            .await
            .expect("provider call registers");

        let error = port
            .invoke_capability(CapabilityInvocation {
                activity_id: different_activity_id(activity_id),
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect_err("activity mismatch must fail before synthetic dispatch");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert_eq!(handler_invocations.load(Ordering::SeqCst), 0);
        assert_eq!(result_writes.load(Ordering::SeqCst), 0);
    }
}
