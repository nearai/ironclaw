use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    capability_surface::CapabilitySurfacePolicy,
    ids::CapabilityId,
    resolution::{Resolution, ResolutionBatch},
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityCallCandidate,
    CapabilityDeniedReasonKind, LoopCapabilityPort, LoopRequest, LoopRequestBatch,
    ProviderToolCall, ProviderToolCallCapabilityIds, ProviderToolDefinition,
    RegisterProviderToolCallRequest, VisibleCapabilityRequest, VisibleCapabilitySurface,
    resolution,
};
use ironclaw_turns::CapabilityActivityId;

use crate::capability_info;

#[derive(Clone)]
pub struct CapabilitySurfacePolicyFilter {
    inner: Arc<dyn LoopCapabilityPort>,
    policy: Arc<CapabilitySurfacePolicy>,
    staged_invocations: Arc<Mutex<HashMap<StagedInvocationKey, Vec<CapabilityId>>>>,
}

#[derive(Clone)]
pub struct CapabilitySurfaceVisibleFilter {
    inner: Arc<dyn LoopCapabilityPort>,
    visible_capability_ids: Arc<HashSet<CapabilityId>>,
    staged_invocations: Arc<Mutex<HashMap<StagedInvocationKey, Vec<CapabilityId>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StagedInvocationKey {
    surface_version: String,
    capability_id: String,
    activity_id: CapabilityActivityId,
    input_ref: String,
}

impl CapabilitySurfacePolicyFilter {
    pub fn new(inner: Arc<dyn LoopCapabilityPort>, policy: Arc<CapabilitySurfacePolicy>) -> Self {
        Self {
            inner,
            policy,
            staged_invocations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn permits(&self, capability_id: &CapabilityId) -> bool {
        self.policy.permits_capability_id(capability_id)
    }
}

impl CapabilitySurfaceVisibleFilter {
    pub fn new(
        inner: Arc<dyn LoopCapabilityPort>,
        visible_capability_ids: impl IntoIterator<Item = CapabilityId>,
    ) -> Self {
        Self {
            inner,
            visible_capability_ids: Arc::new(visible_capability_ids.into_iter().collect()),
            staged_invocations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn permits(&self, capability_id: &CapabilityId) -> bool {
        self.visible_capability_ids.contains(capability_id)
    }
}

#[async_trait]
impl LoopCapabilityPort for CapabilitySurfaceVisibleFilter {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = self.inner.tool_definitions()?;
        definitions.retain(|definition| {
            provider_capability_permitted(&definition.capability_id, |capability_id| {
                self.permits(capability_id)
            })
        });
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        delegate_and_scope_tool_call_capability_ids(
            &self.inner,
            tool_call,
            |capability_id| self.permits(capability_id),
            "provider tool call is outside the model-visible capability view",
        )
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        validate_provider_tool_call_capability_scope(
            self.inner.provider_tool_call_capability_ids(tool_call)?,
            |capability_id| self.permits(capability_id),
            "provider tool call is outside the model-visible capability view",
        )?;
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        validate_provider_tool_call_capability_scope(
            self.inner
                .provider_tool_call_capability_ids(&request.tool_call)?,
            |capability_id| self.permits(capability_id),
            "provider tool call is outside the model-visible capability view",
        )?;
        let candidate = self.inner.register_provider_tool_call(request).await?;
        validate_provider_tool_call_capability_scope(
            candidate_capability_ids(&candidate),
            |capability_id| self.permits(capability_id),
            "provider tool call is outside the model-visible capability view",
        )?;
        record_staged_invocation(&self.staged_invocations, &candidate)?;
        Ok(candidate)
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        apply_policy_filter_to_surface(&mut surface, |capability_id| self.permits(capability_id));
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        if !invocation_capability_permitted(&self.staged_invocations, &request, |capability_id| {
            self.permits(capability_id)
        })? {
            return Ok(model_view_denied_outcome());
        }
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        invoke_filtered_batch(
            &*self.inner,
            request,
            |invocation| {
                invocation_capability_permitted(
                    &self.staged_invocations,
                    invocation,
                    |capability_id| self.permits(capability_id),
                )
            },
            model_view_denied_outcome,
        )
        .await
    }
}

#[async_trait]
impl LoopCapabilityPort for CapabilitySurfacePolicyFilter {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = self.inner.tool_definitions()?;
        definitions.retain(|definition| {
            provider_capability_permitted(&definition.capability_id, |capability_id| {
                self.permits(capability_id)
            })
        });
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        delegate_and_scope_tool_call_capability_ids(
            &self.inner,
            tool_call,
            |capability_id| self.permits(capability_id),
            "provider tool call is outside the resolved capability surface",
        )
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        validate_provider_tool_call_capability_scope(
            self.inner.provider_tool_call_capability_ids(tool_call)?,
            |capability_id| self.permits(capability_id),
            "provider tool call is outside the resolved capability surface",
        )?;
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        validate_provider_tool_call_capability_scope(
            self.inner
                .provider_tool_call_capability_ids(&request.tool_call)?,
            |capability_id| self.permits(capability_id),
            "provider tool call is outside the resolved capability surface",
        )?;
        let candidate = self.inner.register_provider_tool_call(request).await?;
        validate_provider_tool_call_capability_scope(
            candidate_capability_ids(&candidate),
            |capability_id| self.permits(capability_id),
            "provider tool call is outside the resolved capability surface",
        )?;
        record_staged_invocation(&self.staged_invocations, &candidate)?;
        Ok(candidate)
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        apply_policy_filter_to_surface(&mut surface, |capability_id| self.permits(capability_id));
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        if !invocation_capability_permitted(&self.staged_invocations, &request, |capability_id| {
            self.permits(capability_id)
        })? {
            return Ok(surface_profile_denied_outcome());
        }
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        invoke_filtered_batch(
            &*self.inner,
            request,
            |invocation| {
                invocation_capability_permitted(
                    &self.staged_invocations,
                    invocation,
                    |capability_id| self.permits(capability_id),
                )
            },
            surface_profile_denied_outcome,
        )
        .await
    }
}

async fn invoke_filtered_batch(
    inner: &(dyn LoopCapabilityPort + Send + Sync),
    request: LoopRequestBatch,
    permits: impl Fn(&LoopRequest) -> Result<bool, AgentLoopHostError>,
    denied_outcome: fn() -> Resolution,
) -> Result<ResolutionBatch, AgentLoopHostError> {
    let mut slots: Vec<Option<Resolution>> = Vec::with_capacity(request.invocations.len());
    let mut allowed = Vec::new();
    let mut allowed_idx = Vec::new();
    let stop_on_first_suspension = request.stop_on_first_suspension;

    for (index, invocation) in request.invocations.iter().enumerate() {
        if permits(invocation)? {
            allowed.push(invocation.clone());
            allowed_idx.push(index);
            slots.push(None);
        } else {
            slots.push(Some(denied_outcome()));
        }
    }

    let (inner_outcomes, stopped_on_suspension) = if allowed.is_empty() {
        (Vec::new(), false)
    } else {
        let inner_batch = inner
            .invoke_capability_batch(LoopRequestBatch {
                invocations: allowed,
                stop_on_first_suspension: request.stop_on_first_suspension,
            })
            .await?;
        (inner_batch.resolutions, inner_batch.stopped_on_suspension)
    };

    if inner_outcomes.len() > allowed_idx.len() {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability surface filter received too many inner outcomes",
        ));
    }
    if stopped_on_suspension
        && (!stop_on_first_suspension || !inner_outcomes.last().is_some_and(Resolution::parks))
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability surface filter received invalid inner suspension state",
        ));
    }
    if !stopped_on_suspension && inner_outcomes.len() < allowed_idx.len() {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability surface filter received too few inner outcomes without suspension",
        ));
    }

    let n_inner = inner_outcomes.len();
    for (outcome, original_index) in inner_outcomes
        .into_iter()
        .zip(allowed_idx.iter().copied().take(n_inner))
    {
        slots[original_index] = Some(outcome);
    }

    let truncate_to = if stopped_on_suspension && n_inner > 0 {
        allowed_idx[n_inner - 1] + 1
    } else if n_inner == allowed_idx.len() {
        slots.len()
    } else if n_inner == 0 {
        allowed_idx.first().copied().unwrap_or(slots.len())
    } else {
        allowed_idx[n_inner - 1] + 1
    };
    slots.truncate(truncate_to);

    let mut resolutions = Vec::with_capacity(slots.len());
    for slot in slots {
        let outcome = slot.ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "capability surface filter retained an unpopulated outcome slot",
            )
        })?;
        resolutions.push(outcome);
    }

    Ok(ResolutionBatch {
        resolutions,
        stopped_on_suspension,
    })
}

fn apply_policy_filter_to_surface(
    surface: &mut VisibleCapabilitySurface,
    permits: impl Fn(&CapabilityId) -> bool,
) {
    surface
        .descriptors
        .retain(|descriptor| provider_capability_permitted(&descriptor.capability_id, &permits));
    if let Some(callable_capability_ids) = surface.callable_capability_ids.as_mut() {
        callable_capability_ids
            .retain(|capability_id| provider_capability_permitted(capability_id, &permits));
    }
}

/// Resolve `provider_tool_call_capability_ids` through the inner port, then
/// scope-check the result against `permits`.
///
/// Every filtering decorator must resolve the call via the inner port rather
/// than the `LoopCapabilityPort` default, because the default searches
/// `self.tool_definitions()` — the decorator's *already-narrowed* advertised
/// surface — and so rejects deferred/disclosed tools before the inner
/// tool-disclosure forgiving path can resolve them. This helper is the one copy
/// of that "delegate down, then apply my scope" skeleton shared by the
/// per-model visible snapshot and resolved-policy filters.
fn delegate_and_scope_tool_call_capability_ids(
    inner: &Arc<dyn LoopCapabilityPort>,
    tool_call: &ProviderToolCall,
    permits: impl Fn(&CapabilityId) -> bool,
    denial_message: &'static str,
) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
    let capability_ids = inner.provider_tool_call_capability_ids(tool_call)?;
    validate_provider_tool_call_capability_scope(capability_ids.clone(), permits, denial_message)?;
    Ok(capability_ids)
}

fn validate_provider_tool_call_capability_scope(
    capability_ids: ProviderToolCallCapabilityIds,
    permits: impl Fn(&CapabilityId) -> bool,
    denial_message: &'static str,
) -> Result<(), AgentLoopHostError> {
    if !provider_capability_permitted(&capability_ids.provider_capability_id, &permits)
        || capability_ids
            .effective_capability_ids
            .iter()
            .any(|capability_id| !provider_capability_permitted(capability_id, &permits))
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            denial_message,
        ));
    }
    Ok(())
}

fn candidate_capability_ids(candidate: &CapabilityCallCandidate) -> ProviderToolCallCapabilityIds {
    let effective_capability_ids = if candidate.effective_capability_ids.is_empty() {
        vec![candidate.capability_id.clone()]
    } else {
        candidate.effective_capability_ids.clone()
    };
    ProviderToolCallCapabilityIds {
        provider_capability_id: candidate.capability_id.clone(),
        effective_capability_ids,
    }
}

fn record_staged_invocation(
    staged_invocations: &Mutex<HashMap<StagedInvocationKey, Vec<CapabilityId>>>,
    candidate: &CapabilityCallCandidate,
) -> Result<(), AgentLoopHostError> {
    let effective_capability_ids = if candidate.effective_capability_ids.is_empty() {
        vec![candidate.capability_id.clone()]
    } else {
        candidate.effective_capability_ids.clone()
    };
    lock_staged_invocations(staged_invocations)?.insert(
        StagedInvocationKey::from_candidate(candidate),
        effective_capability_ids,
    );
    Ok(())
}

fn invocation_capability_permitted(
    staged_invocations: &Mutex<HashMap<StagedInvocationKey, Vec<CapabilityId>>>,
    invocation: &LoopRequest,
    permits: impl Fn(&CapabilityId) -> bool,
) -> Result<bool, AgentLoopHostError> {
    if !capability_info::is_capability_id(&invocation.capability_id) {
        return Ok(permits(&invocation.capability_id));
    }
    let Some(effective_capability_ids) = lock_staged_invocations(staged_invocations)?
        .get(&StagedInvocationKey::from_invocation(invocation))
        .cloned()
    else {
        return Ok(false);
    };
    Ok(effective_capability_ids
        .iter()
        .all(|capability_id| provider_capability_permitted(capability_id, &permits)))
}

fn lock_staged_invocations(
    staged_invocations: &Mutex<HashMap<StagedInvocationKey, Vec<CapabilityId>>>,
) -> Result<MutexGuard<'_, HashMap<StagedInvocationKey, Vec<CapabilityId>>>, AgentLoopHostError> {
    staged_invocations.lock().map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability staged invocation store lock is poisoned",
        )
    })
}

impl StagedInvocationKey {
    fn from_candidate(candidate: &CapabilityCallCandidate) -> Self {
        Self {
            surface_version: candidate.surface_version.as_str().to_string(),
            capability_id: candidate.capability_id.as_str().to_string(),
            activity_id: candidate.activity_id,
            input_ref: candidate.input_ref.as_str().to_string(),
        }
    }

    fn from_invocation(invocation: &LoopRequest) -> Self {
        Self {
            surface_version: invocation.surface_version.as_str().to_string(),
            capability_id: invocation.capability_id.as_str().to_string(),
            activity_id: invocation.activity_id,
            input_ref: invocation.input_ref.as_str().to_string(),
        }
    }
}

fn provider_capability_permitted(
    capability_id: &CapabilityId,
    permits: impl Fn(&CapabilityId) -> bool,
) -> bool {
    permits(capability_id) || capability_info::is_capability_id(capability_id)
}

fn model_view_denied_outcome() -> Resolution {
    resolution::denied(
        model_view_denied_kind(),
        "capability outside the model-visible view".to_string(),
    )
    .resolution
}

fn model_view_denied_kind() -> CapabilityDeniedReasonKind {
    match CapabilityDeniedReasonKind::unknown("model_view_denied") {
        Ok(kind) => kind,
        Err(_) => {
            debug_assert!(
                false,
                "model_view_denied_kind() fallback reached - this is a contract bug: \
                 'model_view_denied' must be a valid reason kind value"
            );
            CapabilityDeniedReasonKind::EmptySurface
        }
    }
}

fn surface_profile_denied_outcome() -> Resolution {
    resolution::denied(
        surface_profile_denied_kind(),
        "capability not in run-profile surface".to_string(),
    )
    .resolution
}

fn surface_profile_denied_kind() -> CapabilityDeniedReasonKind {
    match CapabilityDeniedReasonKind::unknown("surface_profile_denied") {
        Ok(kind) => kind,
        Err(_) => {
            debug_assert!(
                false,
                "surface_profile_denied_kind() fallback reached - this is a contract bug: \
                 'surface_profile_denied' must be a valid reason kind value"
            );
            CapabilityDeniedReasonKind::EmptySurface
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use ironclaw_host_api::{
        ids::{CapabilityId, ProviderToolName},
        resolution::Blocked,
        runtime::RuntimeKind,
    };
    use ironclaw_loop_contracts::{
        CapabilityDescriptorView, CapabilityInputRef, CapabilitySurfaceVersion, ConcurrencyHint,
        resolution,
    };
    use ironclaw_turns::{LoopGateRef, LoopResultRef};

    use super::*;

    #[derive(Default)]
    struct SpyPort {
        surface: Mutex<Option<VisibleCapabilitySurface>>,
        batch_outcome: Mutex<Option<ironclaw_host_api::resolution::ResolutionBatch>>,
        tool_definitions: Mutex<Vec<ProviderToolDefinition>>,
        provider_call_capability_ids:
            Mutex<HashMap<ProviderToolName, ProviderToolCallCapabilityIds>>,
        registered_candidate_capability_ids: Mutex<Option<ProviderToolCallCapabilityIds>>,
        validated_provider_calls: Mutex<Vec<ProviderToolCall>>,
        provider_calls: Mutex<Vec<ProviderToolCall>>,
        visible_calls: Mutex<usize>,
        invocations: Mutex<Vec<LoopRequest>>,
        batches: Mutex<Vec<LoopRequestBatch>>,
    }

    #[async_trait]
    impl LoopCapabilityPort for SpyPort {
        fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
            Ok(self
                .tool_definitions
                .lock()
                .expect("tool definitions lock")
                .clone())
        }

        fn provider_tool_call_capability_ids(
            &self,
            tool_call: &ProviderToolCall,
        ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
            if let Some(capability_ids) = self
                .provider_call_capability_ids
                .lock()
                .expect("provider call capability ids lock")
                .get(&tool_call.name)
                .cloned()
            {
                return Ok(capability_ids);
            }
            let Some(definition) = self
                .tool_definitions()?
                .into_iter()
                .find(|definition| definition.name == tool_call.name)
            else {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "provider tool call is outside the visible capability surface",
                ));
            };
            Ok(ProviderToolCallCapabilityIds::single(
                definition.capability_id,
            ))
        }

        fn validate_provider_tool_call(
            &self,
            request: &ProviderToolCall,
        ) -> Result<(), AgentLoopHostError> {
            self.validated_provider_calls
                .lock()
                .expect("validated provider call lock")
                .push(request.clone());
            Ok(())
        }

        async fn register_provider_tool_call(
            &self,
            request: RegisterProviderToolCallRequest,
        ) -> Result<ironclaw_loop_contracts::CapabilityCallCandidate, AgentLoopHostError> {
            let RegisterProviderToolCallRequest {
                tool_call,
                activity_id,
            } = request;
            self.provider_calls
                .lock()
                .expect("provider call lock")
                .push(tool_call);
            let capability_ids = self
                .registered_candidate_capability_ids
                .lock()
                .expect("registered candidate capability ids lock")
                .clone()
                .unwrap_or_else(|| provider_call_capability_ids(&["demo.allowed"]));
            Ok(ironclaw_loop_contracts::CapabilityCallCandidate {
                activity_id: activity_id.unwrap_or_default(),
                surface_version: surface_version(),
                capability_id: capability_ids.provider_capability_id,
                input_ref: input_ref("input:provider"),
                effective_capability_ids: capability_ids.effective_capability_ids,
                provider_replay: None,
            })
        }

        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            *self.visible_calls.lock().expect("visible calls lock") += 1;
            Ok(self
                .surface
                .lock()
                .expect("surface lock")
                .clone()
                .expect("test surface is configured"))
        }

        async fn invoke_capability(
            &self,
            request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            self.invocations
                .lock()
                .expect("invocation lock")
                .push(request);
            Ok(completed("result:single"))
        }

        async fn invoke_capability_batch(
            &self,
            request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
            self.batches.lock().expect("batch lock").push(request);
            Ok(self
                .batch_outcome
                .lock()
                .expect("batch outcome lock")
                .clone()
                .unwrap_or_else(|| ironclaw_host_api::resolution::ResolutionBatch {
                    resolutions: vec![completed("result:first"), completed("result:second")],
                    stopped_on_suspension: false,
                }))
        }
    }

    fn capability_id(value: &str) -> CapabilityId {
        CapabilityId::new(value).expect("test capability id is valid")
    }

    fn surface_version() -> CapabilitySurfaceVersion {
        CapabilitySurfaceVersion::new("surface-v1").expect("test version is valid")
    }

    fn input_ref(value: &str) -> CapabilityInputRef {
        CapabilityInputRef::new(value).expect("test input ref is valid")
    }

    fn provider_name(value: &str) -> ProviderToolName {
        ProviderToolName::new(value).expect("provider tool name")
    }

    fn invocation(capability: &str, input: &str) -> LoopRequest {
        LoopRequest {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: surface_version(),
            capability_id: capability_id(capability),
            input_ref: input_ref(input),
            approval_resume: None,
            auth_resume: None,
        }
    }

    fn descriptor(capability: &str) -> CapabilityDescriptorView {
        CapabilityDescriptorView {
            capability_id: capability_id(capability),
            provider: None,
            runtime: RuntimeKind::Wasm,
            safe_name: capability.to_string(),
            safe_description: format!("{capability} description"),
            description_trust: Default::default(),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
            parameters_schema: serde_json::json!({"type":"object","properties":{"input":{"type":"string"}}}),
        }
    }

    fn provider_definition(capability: &str, name: &str) -> ProviderToolDefinition {
        ProviderToolDefinition {
            capability_id: capability_id(capability),
            name: ProviderToolName::new(name).expect("provider tool name"),
            description: format!("{capability} description"),
            description_trust: Default::default(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }

    fn provider_call_capability_ids(capability_ids: &[&str]) -> ProviderToolCallCapabilityIds {
        let provider_capability_id = capability_id(capability_ids[0]);
        ProviderToolCallCapabilityIds {
            provider_capability_id,
            effective_capability_ids: capability_ids
                .iter()
                .map(|capability| capability_id(capability))
                .collect(),
        }
    }

    fn provider_call(name: &str) -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            turn_id: Some("turn_1".to_string()),
            id: "call_1".to_string(),
            name: ProviderToolName::new(name).expect("provider tool name"),
            arguments: serde_json::json!({}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    fn capability_info_call(target: &str) -> ProviderToolCall {
        let mut call = provider_call(capability_info::TOOL_NAME);
        call.arguments = serde_json::json!({ "name": target });
        call
    }

    fn completed(result_ref: &str) -> Resolution {
        resolution::completed(
            LoopResultRef::new(result_ref).expect("test result ref is valid"),
            "done".to_string(),
            ironclaw_loop_contracts::CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        )
    }

    fn approval_required(gate_ref: &str) -> Resolution {
        resolution::approval_required(
            LoopGateRef::new(gate_ref).expect("test gate ref is valid"),
            "approval needed".to_string(),
            None,
        )
        .resolution
    }

    // The §5.3 collapse maps the open-set loop `reason_kind`
    // (`model_view_denied` / `surface_profile_denied`) onto the closed host_api
    // `DenyReason` (both become `PolicyDenied`), so the specific reason tag no
    // longer rides the structured denial channel. The two filter denials remain
    // distinguishable by their fixed, host-authored summaries, which survive on
    // `Denial.summary` — recover the original reason tag from there so these
    // assertions keep verifying *which* filter denied the call.
    fn denied_reason(resolution: &Resolution) -> Option<&'static str> {
        let summary = resolution.denial()?.summary.as_ref()?.as_str();
        match summary {
            "capability not in run-profile surface" => Some("surface_profile_denied"),
            "capability outside the model-visible view" => Some("model_view_denied"),
            _ => None,
        }
    }

    #[tokio::test]
    async fn visible_capabilities_filters_descriptors_and_callable_ids() {
        let inner = Arc::new(SpyPort::default());
        *inner.surface.lock().expect("surface lock") = Some(VisibleCapabilitySurface {
            callable_capability_ids: Some(vec![
                capability_id("demo.a"),
                capability_id("demo.b"),
                capability_id("demo.d"),
            ]),
            version: surface_version(),
            descriptors: vec![
                descriptor("demo.a"),
                descriptor("demo.b"),
                descriptor("demo.c"),
                descriptor("demo.d"),
                descriptor("demo.e"),
            ],
        });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([
                capability_id("demo.b"),
                capability_id("demo.d"),
            ])),
        );

        let surface = filter
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface");

        assert_eq!(surface.version, surface_version());
        assert_eq!(
            surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.as_str())
                .collect::<Vec<_>>(),
            vec!["demo.b", "demo.d"]
        );
        assert_eq!(
            surface.callable_capability_ids,
            Some(vec![capability_id("demo.b"), capability_id("demo.d")])
        );
    }

    #[test]
    fn tool_definitions_filters_provider_tools_to_allowlist() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition("demo.allowed", "demo__allowed"),
            provider_definition("demo.denied", "demo__denied"),
            provider_definition("demo.other_allowed", "demo__other_allowed"),
        ];
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([
                capability_id("demo.allowed"),
                capability_id("demo.other_allowed"),
            ])),
        );

        let definitions = filter.tool_definitions().expect("tool definitions");

        assert_eq!(
            definitions
                .iter()
                .map(|definition| (definition.capability_id.as_str(), definition.name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("demo.allowed", "demo__allowed"),
                ("demo.other_allowed", "demo__other_allowed"),
            ]
        );
    }

    #[test]
    fn tool_definitions_preserve_capability_info_for_filtered_surface() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
            provider_definition("demo.denied", "demo__denied"),
        ];
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let definitions = filter.tool_definitions().expect("tool definitions");

        assert_eq!(
            definitions
                .iter()
                .map(|definition| (definition.capability_id.as_str(), definition.name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
                ("demo.allowed", "demo__allowed"),
            ]
        );
    }

    #[tokio::test]
    async fn visible_capabilities_preserve_capability_info_for_filtered_surface() {
        let inner = Arc::new(SpyPort::default());
        *inner.surface.lock().expect("surface lock") = Some(VisibleCapabilitySurface {
            callable_capability_ids: None,
            version: surface_version(),
            descriptors: vec![
                descriptor(capability_info::CAPABILITY_ID),
                descriptor("demo.allowed"),
                descriptor("demo.denied"),
            ],
        });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let surface = filter
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface");

        assert_eq!(
            surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.as_str())
                .collect::<Vec<_>>(),
            vec![capability_info::CAPABILITY_ID, "demo.allowed"]
        );
    }

    #[tokio::test]
    async fn invoke_denied_when_not_in_allowlist() {
        let inner = Arc::new(SpyPort::default());
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let outcome = filter
            .invoke_capability(invocation("demo.denied", "input:denied"))
            .await
            .expect("outcome");

        assert_eq!(denied_reason(&outcome), Some("surface_profile_denied"));
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocation lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn provider_tool_call_denied_before_inner_registration_when_not_allowed() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition("demo.allowed", "demo__allowed"),
            provider_definition("demo.denied", "demo__denied"),
        ];
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let error = filter
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(provider_call(
                "demo__denied",
            )))
            .await
            .expect_err("denied provider call should fail before staging");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            inner
                .provider_calls
                .lock()
                .expect("provider calls lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn capability_info_target_denied_before_profile_filter_inner_registration() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
            provider_definition("demo.denied", "demo__denied"),
        ];
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name(capability_info::TOOL_NAME),
                provider_call_capability_ids(&[capability_info::CAPABILITY_ID, "demo.denied"]),
            );
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let error = filter
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                capability_info_call("demo__denied"),
            ))
            .await
            .expect_err("denied capability_info target should fail before staging");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            inner
                .provider_calls
                .lock()
                .expect("provider calls lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn capability_info_target_rechecked_after_profile_filter_inner_registration() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
            provider_definition("demo.denied", "demo__denied"),
        ];
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name(capability_info::TOOL_NAME),
                provider_call_capability_ids(&[capability_info::CAPABILITY_ID, "demo.allowed"]),
            );
        *inner
            .registered_candidate_capability_ids
            .lock()
            .expect("registered candidate capability ids lock") =
            Some(provider_call_capability_ids(&[
                capability_info::CAPABILITY_ID,
                "demo.denied",
            ]));
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let error = filter
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                capability_info_call("demo__allowed"),
            ))
            .await
            .expect_err("changed capability_info target should fail after staging");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert_eq!(
            inner
                .provider_calls
                .lock()
                .expect("provider calls lock")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn capability_info_invocation_requires_staged_effective_target() {
        let inner = Arc::new(SpyPort::default());
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let outcome = filter
            .invoke_capability(invocation(capability_info::CAPABILITY_ID, "input:provider"))
            .await
            .expect("outcome");

        assert_eq!(denied_reason(&outcome), Some("surface_profile_denied"));
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocation lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn capability_info_invocation_uses_staged_effective_target() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
        ];
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name(capability_info::TOOL_NAME),
                provider_call_capability_ids(&[capability_info::CAPABILITY_ID, "demo.allowed"]),
            );
        *inner
            .registered_candidate_capability_ids
            .lock()
            .expect("registered candidate capability ids lock") =
            Some(provider_call_capability_ids(&[
                capability_info::CAPABILITY_ID,
                "demo.allowed",
            ]));
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );
        let candidate = filter
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                capability_info_call("demo__allowed"),
            ))
            .await
            .expect("allowed capability_info target should stage");

        filter
            .invoke_capability(LoopRequest {
                activity_id: candidate.activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("staged capability_info invocation should pass");

        assert_eq!(inner.invocations.lock().expect("invocation lock").len(), 1);
    }

    #[tokio::test]
    async fn capability_info_invocation_rejects_mismatched_activity_id() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
        ];
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name(capability_info::TOOL_NAME),
                provider_call_capability_ids(&[capability_info::CAPABILITY_ID, "demo.allowed"]),
            );
        *inner
            .registered_candidate_capability_ids
            .lock()
            .expect("registered candidate capability ids lock") =
            Some(provider_call_capability_ids(&[
                capability_info::CAPABILITY_ID,
                "demo.allowed",
            ]));
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );
        let candidate = filter
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                capability_info_call("demo__allowed"),
            ))
            .await
            .expect("allowed capability_info target should stage");

        let outcome = filter
            .invoke_capability(LoopRequest {
                activity_id: CapabilityActivityId::new(),
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id,
                input_ref: candidate.input_ref,
                approval_resume: None,
                auth_resume: None,
            })
            .await
            .expect("mismatched staged capability_info invocation should be denied");

        assert_eq!(denied_reason(&outcome), Some("surface_profile_denied"));
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocation lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn visible_filter_batches_staged_capability_info_invocation() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") =
            Some(ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![completed("result:capability-info")],
                stopped_on_suspension: false,
            });
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
        ];
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name(capability_info::TOOL_NAME),
                provider_call_capability_ids(&[capability_info::CAPABILITY_ID, "demo.allowed"]),
            );
        *inner
            .registered_candidate_capability_ids
            .lock()
            .expect("registered candidate capability ids lock") =
            Some(provider_call_capability_ids(&[
                capability_info::CAPABILITY_ID,
                "demo.allowed",
            ]));
        let filter =
            CapabilitySurfaceVisibleFilter::new(inner.clone(), [capability_id("demo.allowed")]);
        let candidate = filter
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(
                capability_info_call("demo__allowed"),
            ))
            .await
            .expect("allowed capability_info target should stage");

        filter
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![LoopRequest {
                    activity_id: candidate.activity_id,
                    surface_version: candidate.surface_version,
                    capability_id: candidate.capability_id,
                    input_ref: candidate.input_ref,
                    approval_resume: None,
                    auth_resume: None,
                }],
                stop_on_first_suspension: true,
            })
            .await
            .expect("staged capability_info batch should pass");

        assert_eq!(inner.batches.lock().expect("batch lock").len(), 1);
    }

    #[test]
    fn capability_info_target_denied_before_profile_filter_inner_validation() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
            provider_definition("demo.denied", "demo__denied"),
        ];
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name(capability_info::TOOL_NAME),
                provider_call_capability_ids(&[capability_info::CAPABILITY_ID, "demo.denied"]),
            );
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let error = filter
            .validate_provider_tool_call(&capability_info_call("demo__denied"))
            .expect_err("denied capability_info target should fail before inner validation");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            inner
                .validated_provider_calls
                .lock()
                .expect("validated provider calls lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn capability_info_target_denied_before_visible_filter_inner_registration() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
            provider_definition("demo.denied", "demo__denied"),
        ];
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name(capability_info::TOOL_NAME),
                provider_call_capability_ids(&[capability_info::CAPABILITY_ID, "demo.denied"]),
            );
        let filter =
            CapabilitySurfaceVisibleFilter::new(inner.clone(), [capability_id("demo.allowed")]);

        let mut call = capability_info_call("demo.denied");
        call.arguments["detail"] = serde_json::json!("schema");
        let error = filter
            .register_provider_tool_call(RegisterProviderToolCallRequest::new(call))
            .await
            .expect_err("denied capability_info target should fail before staging");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            inner
                .provider_calls
                .lock()
                .expect("provider calls lock")
                .is_empty()
        );
    }

    #[test]
    fn capability_info_target_denied_before_visible_filter_inner_validation() {
        let inner = Arc::new(SpyPort::default());
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") = vec![
            provider_definition(capability_info::CAPABILITY_ID, capability_info::TOOL_NAME),
            provider_definition("demo.allowed", "demo__allowed"),
            provider_definition("demo.denied", "demo__denied"),
        ];
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name(capability_info::TOOL_NAME),
                provider_call_capability_ids(&[capability_info::CAPABILITY_ID, "demo.denied"]),
            );
        let filter =
            CapabilitySurfaceVisibleFilter::new(inner.clone(), [capability_id("demo.allowed")]);

        let error = filter
            .validate_provider_tool_call(&capability_info_call("demo.denied"))
            .expect_err("denied capability_info target should fail before inner validation");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(
            inner
                .validated_provider_calls
                .lock()
                .expect("validated provider calls lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn batch_partitions_correctly() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") =
            Some(ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![completed("result:first"), completed("result:second")],
                stopped_on_suspension: false,
            });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([
                capability_id("demo.first"),
                capability_id("demo.second"),
            ])),
        );

        let outcome = filter
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.denied", "input:denied"),
                    invocation("demo.second", "input:second"),
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect("batch outcome");

        assert_eq!(outcome.resolutions.len(), 3);
        assert!(matches!(
            &outcome.resolutions[0],
            Resolution::Done(outcome)
                if outcome.refs.origin.as_ref().is_some_and(|origin| origin.as_str() == "result:first")
        ));
        assert_eq!(
            denied_reason(&outcome.resolutions[1]),
            Some("surface_profile_denied")
        );
        assert!(matches!(
            &outcome.resolutions[2],
            Resolution::Done(outcome)
                if outcome.refs.origin.as_ref().is_some_and(|origin| origin.as_str() == "result:second")
        ));
        let batches = inner.batches.lock().expect("batch lock");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].invocations.len(), 2);
        assert_eq!(
            batches[0].invocations[0].capability_id.as_str(),
            "demo.first"
        );
        assert_eq!(
            batches[0].invocations[1].capability_id.as_str(),
            "demo.second"
        );
    }

    #[tokio::test]
    async fn short_inner_outcomes_without_suspension_are_rejected() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") =
            Some(ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![completed("result:first")],
                stopped_on_suspension: false,
            });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([
                capability_id("demo.first"),
                capability_id("demo.second"),
            ])),
        );

        let error = filter
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.second", "input:second"),
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect_err("short non-suspended outcomes violate the inner port contract");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
        assert_eq!(
            error.safe_summary,
            "capability surface filter received too few inner outcomes without suspension"
        );
    }

    #[tokio::test]
    async fn partial_inner_outcomes_truncate_correctly() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") =
            Some(ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![completed("result:first"), approval_required("gate:second")],
                stopped_on_suspension: true,
            });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([
                capability_id("demo.first"),
                capability_id("demo.second"),
                capability_id("demo.third"),
            ])),
        );

        let outcome = filter
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.denied", "input:denied"),
                    invocation("demo.second", "input:second"),
                    invocation("demo.third", "input:third"),
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect("batch outcome");

        assert_eq!(outcome.resolutions.len(), 3);
        assert_eq!(
            denied_reason(&outcome.resolutions[1]),
            Some("surface_profile_denied")
        );
        assert!(outcome.stopped_on_suspension);
    }

    #[tokio::test]
    async fn inner_suspension_without_requested_early_stop_is_rejected() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") =
            Some(ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![approval_required("gate:first")],
                stopped_on_suspension: true,
            });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([
                capability_id("demo.first"),
                capability_id("demo.second"),
            ])),
        );

        let error = filter
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.second", "input:second"),
                ],
                stop_on_first_suspension: false,
            })
            .await
            .expect_err("inner suspension requires a caller-requested early stop");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
        assert_eq!(
            error.safe_summary,
            "capability surface filter received invalid inner suspension state"
        );
    }

    #[tokio::test]
    async fn inner_suspension_requires_a_final_parking_resolution() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") =
            Some(ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![completed("result:first")],
                stopped_on_suspension: true,
            });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([
                capability_id("demo.first"),
                capability_id("demo.second"),
            ])),
        );

        let error = filter
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.second", "input:second"),
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect_err("inner suspension must end with a parking resolution");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
        assert_eq!(
            error.safe_summary,
            "capability surface filter received invalid inner suspension state"
        );
    }

    #[tokio::test]
    async fn stopped_inner_batch_truncates_denials_after_last_allowed_outcome() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") =
            Some(ironclaw_host_api::resolution::ResolutionBatch {
                resolutions: vec![approval_required("gate:first")],
                stopped_on_suspension: true,
            });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner.clone(),
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.first",
            )])),
        );

        let outcome = filter
            .invoke_capability_batch(LoopRequestBatch {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.denied", "input:denied"),
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect("batch outcome");

        assert!(outcome.stopped_on_suspension);
        assert_eq!(outcome.resolutions.len(), 1);
        assert!(matches!(
            outcome.resolutions.as_slice(),
            [Resolution::Blocked(Blocked::Approval(_))]
        ));
        let batches = inner.batches.lock().expect("batch lock");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].invocations.len(), 1);
        assert!(batches[0].stop_on_first_suspension);
    }

    #[tokio::test]
    async fn surface_version_preserved() {
        let inner = Arc::new(SpyPort::default());
        *inner.surface.lock().expect("surface lock") = Some(VisibleCapabilitySurface {
            callable_capability_ids: None,
            version: surface_version(),
            descriptors: vec![descriptor("demo.allowed"), descriptor("demo.denied")],
        });
        let filter = CapabilitySurfacePolicyFilter::new(
            inner,
            Arc::new(CapabilitySurfacePolicy::allow_only([capability_id(
                "demo.allowed",
            )])),
        );

        let surface = filter
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface");

        assert_eq!(surface.version.as_str(), "surface-v1");
    }

    #[test]
    fn visible_filter_delegates_provider_tool_call_resolution_to_inner() {
        // Regression: the model gateway pre-check calls
        // `provider_tool_call_capability_ids` on whatever capability port it holds
        // — here a CapabilitySurfaceVisibleFilter. If the filter fell back to the
        // LoopCapabilityPort default (which only searches its OWN already-filtered
        // `tool_definitions`), every deferred/disclosed tool would be rejected
        // before the inner forgiving port could resolve it — which is exactly the
        // "outside the visible capability surface" failure that killed every
        // deferred extension-tool call. The filter must delegate to inner, then
        // apply the visible-scope check.
        let inner = Arc::new(SpyPort::default());
        // Only the advertised tool is in tool_definitions (the filtered surface)...
        *inner
            .tool_definitions
            .lock()
            .expect("tool definitions lock") =
            vec![provider_definition("demo.advertised", "demo__advertised")];
        // ...but the inner port can still RESOLVE a deferred tool (mimicking the
        // tool-disclosure forgiving path) that is NOT in tool_definitions.
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name("demo__deferred"),
                provider_call_capability_ids(&["demo.deferred"]),
            );
        // The deferred tool's capability id IS in the model-visible view (it was
        // disclosed via tool_search), so the call must resolve.
        let filter = CapabilitySurfaceVisibleFilter::new(
            Arc::clone(&inner) as Arc<dyn LoopCapabilityPort>,
            [
                capability_id("demo.advertised"),
                capability_id("demo.deferred"),
            ],
        );

        let resolved = filter
            .provider_tool_call_capability_ids(&provider_call("demo__deferred"))
            .expect("deferred-but-visible tool resolves through the inner forgiving path");
        assert_eq!(
            resolved.provider_capability_id,
            capability_id("demo.deferred")
        );

        // A resolvable tool whose capability id is NOT in the visible view is still
        // rejected by the scope check.
        inner
            .provider_call_capability_ids
            .lock()
            .expect("provider call capability ids lock")
            .insert(
                provider_name("demo__hidden"),
                provider_call_capability_ids(&["demo.hidden"]),
            );
        let error = filter
            .provider_tool_call_capability_ids(&provider_call("demo__hidden"))
            .expect_err("non-visible tool is rejected");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }
}
// arch-exempt: large_file, capability surface migration remains centralized, plan #6175
