use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_events::AuditSink;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AuditEnvelope, AuditEventId, AuditStage, CorrelationId,
    DecisionSummary, EffectKind, ExtensionId, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_memory::{
    MemoryEventSinkError, MemoryInvocation, MemoryService, MemoryServiceError,
    MemoryServiceErrorKind, MemoryServiceReadRequest, MemoryServiceReadResponse,
    MemoryServiceSearchRequest, MemoryServiceSearchResponse, MemoryServiceTreeRequest,
    MemoryServiceTreeResponse, MemoryServiceWriteRequest, MemoryServiceWriteResponse,
    MemoryWriteStatus, PromptSafetyReasonCode, PromptWriteOperation, PromptWriteSafetyEvent,
    PromptWriteSafetyEventKind, PromptWriteSafetyEventSink,
};
use serde_json::{Value, json};

use crate::memory_provider::MemoryServiceResolver;
use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest, FirstPartyCapabilityResult};

use super::{input_error, operation_error};

// The memory extension rides the always-on first-party lane (like `builtin`),
// as the `ironclaw.memory` extension (backed by the native provider by default,
// swappable via the document-store binding). The model-facing tool names derive
// from these ids (`.` -> `__`): `ironclaw__memory__{read,write,search,tree}`.
pub const MEMORY_SEARCH_CAPABILITY_ID: &str = "ironclaw.memory.search";
pub const MEMORY_WRITE_CAPABILITY_ID: &str = "ironclaw.memory.write";
pub const MEMORY_READ_CAPABILITY_ID: &str = "ironclaw.memory.read";
pub const MEMORY_TREE_CAPABILITY_ID: &str = "ironclaw.memory.tree";
const MEMORY_PROMPT_SAFETY_EXTENSION_ID: &str = "memory.prompt_safety";
const MEMORY_SEARCH_SCOPE: &str = "reborn_internal_persistent_memory";

struct MemoryServices {
    invocation: MemoryInvocation,
    memory_service: Arc<dyn MemoryService>,
}

#[derive(Default)]
pub(super) struct MemoryCapabilityState {
    /// Single construction point for the memory provider (issue #3537). The
    /// tools build their `MemoryService` only through this resolver; `Default`
    /// is native, preserving pre-binding behavior until composition hands down a
    /// config-resolved resolver.
    resolver: MemoryServiceResolver,
    cached_memory_service: Mutex<Option<CachedMemoryService>>,
    #[cfg(test)]
    memory_service_for_test: Option<Arc<dyn MemoryService>>,
}

impl std::fmt::Debug for MemoryCapabilityState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryCapabilityState")
            .field("cached_memory_service", &"<cached-memory-service>")
            .finish()
    }
}

struct CachedMemoryService {
    filesystem: Arc<dyn RootFilesystem>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    service: Arc<dyn MemoryService>,
}

pub(super) async fn dispatch(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    let start = std::time::Instant::now();
    let services = memory_services(state, request)?;
    let output = match request.capability_id.as_str() {
        MEMORY_SEARCH_CAPABILITY_ID => dispatch_search(&services, &request.input).await?,
        MEMORY_WRITE_CAPABILITY_ID => dispatch_write(&services, &request.input).await?,
        MEMORY_READ_CAPABILITY_ID => dispatch_read(&services, &request.input).await?,
        MEMORY_TREE_CAPABILITY_ID => dispatch_tree(&services, &request.input).await?,
        _ => return Err(operation_error()),
    };
    let wall_clock_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    Ok(FirstPartyCapabilityResult::new(
        output,
        ResourceUsage::default().set_wall_clock_ms(wall_clock_ms),
    ))
}

fn memory_services(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
) -> Result<MemoryServices, FirstPartyCapabilityError> {
    ensure_memory_mount(
        request,
        request.capability_id.as_str() == MEMORY_WRITE_CAPABILITY_ID,
    )?;
    Ok(MemoryServices {
        invocation: invocation_for_request(request),
        memory_service: state.service_for(request)?,
    })
}

impl MemoryCapabilityState {
    /// Construct with a resolved memory provider resolver (issue #3537).
    pub(crate) fn with_resolver(resolver: MemoryServiceResolver) -> Self {
        Self {
            resolver,
            ..Default::default()
        }
    }

    pub(super) fn service_for(
        &self,
        request: &FirstPartyCapabilityRequest,
    ) -> Result<Arc<dyn MemoryService>, FirstPartyCapabilityError> {
        #[cfg(test)]
        if let Some(service) = &self.memory_service_for_test {
            return Ok(Arc::clone(service));
        }

        let mut cached = self
            .cached_memory_service
            .lock()
            .map_err(|_| operation_error())?;
        if let Some(cached) = cached.as_ref()
            && Arc::ptr_eq(&cached.filesystem, &request.services.filesystem)
            && audit_sinks_match(
                cached.audit_sink.as_ref(),
                request.services.audit_sink.as_ref(),
            )
        {
            return Ok(Arc::clone(&cached.service));
        }

        let filesystem = Arc::clone(&request.services.filesystem);
        let audit_sink = request.services.audit_sink.clone();
        let prompt_write_safety_event_sink = audit_sink.clone().map(|audit_sink| {
            Arc::new(AuditPromptWriteSafetyEventSink { audit_sink })
                as Arc<dyn PromptWriteSafetyEventSink>
        });
        // Single construction point: the resolver builds the bound provider over
        // this request's filesystem, or returns `None` (document store disabled
        // or bound to an unimplemented third party) → fail closed with a
        // model-visible error instead of silently using native.
        let Some(service) = self
            .resolver
            .resolve_document_store(Arc::clone(&filesystem), prompt_write_safety_event_sink)
        else {
            return Err(binding_unavailable_error());
        };
        *cached = Some(CachedMemoryService {
            filesystem,
            audit_sink,
            service: Arc::clone(&service),
        });
        Ok(service)
    }

    #[cfg(test)]
    pub(super) fn with_memory_service_for_test(memory_service: Arc<dyn MemoryService>) -> Self {
        Self {
            resolver: MemoryServiceResolver::native(),
            cached_memory_service: Mutex::new(None),
            memory_service_for_test: Some(memory_service),
        }
    }
}

/// Fail-closed error when the document-store binding is disabled or bound to a
/// provider that is not constructable here (e.g. an unimplemented third party).
///
/// Host-authored fixed text — no binding/extension id is interpolated, so the
/// safe-summary validator cannot reject it (see `agent-loop-capabilities.md`).
fn binding_unavailable_error() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::with_safe_summary(
        RuntimeDispatchErrorKind::OperationFailed,
        "memory is unavailable for the configured provider binding",
    )
}

fn audit_sinks_match(
    left: Option<&Arc<dyn AuditSink>>,
    right: Option<&Arc<dyn AuditSink>>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

struct AuditPromptWriteSafetyEventSink {
    audit_sink: Arc<dyn AuditSink>,
}

#[async_trait]
impl PromptWriteSafetyEventSink for AuditPromptWriteSafetyEventSink {
    async fn record_prompt_write_safety_event(
        &self,
        event: PromptWriteSafetyEvent,
    ) -> Result<(), MemoryEventSinkError> {
        let Some(audit_context) = event.audit_context.as_ref() else {
            return Err(MemoryEventSinkError::new(
                "prompt-write safety event missing audit context",
            ));
        };
        let resource_scope = audit_context.resource_scope.clone();
        let record = AuditEnvelope {
            event_id: AuditEventId::new(),
            correlation_id: audit_context.correlation_id,
            stage: match event.kind {
                PromptWriteSafetyEventKind::Rejected => AuditStage::Denied,
                _ => AuditStage::After,
            },
            timestamp: Utc::now(),
            tenant_id: resource_scope.tenant_id,
            user_id: resource_scope.user_id,
            agent_id: resource_scope.agent_id,
            project_id: resource_scope.project_id,
            mission_id: resource_scope.mission_id,
            thread_id: resource_scope.thread_id,
            invocation_id: resource_scope.invocation_id,
            process_id: None,
            approval_request_id: None,
            extension_id: Some(ExtensionId::new(MEMORY_PROMPT_SAFETY_EXTENSION_ID).map_err(
                |error| {
                    MemoryEventSinkError::new(format!(
                        "invalid memory prompt-safety extension id: {error}"
                    ))
                },
            )?),
            action: ActionSummary {
                kind: prompt_write_action_kind(event.operation).to_string(),
                target: None,
                effects: vec![EffectKind::WriteFilesystem],
            },
            decision: DecisionSummary {
                kind: event
                    .reason_code
                    .map(prompt_safety_reason_projection_kind)
                    .unwrap_or_else(|| prompt_safety_event_kind_label(event.kind))
                    .to_string(),
                reason: None,
                actor: None,
            },
            result: Some(ActionResultSummary {
                success: event.kind != PromptWriteSafetyEventKind::Rejected,
                status: Some(encode_prompt_safety_metadata(&event)),
                output_bytes: None,
            }),
        };
        self.audit_sink
            .emit_audit(record)
            .await
            // security: the audit-sink error may carry backend paths/details, so it
            // is not stringified into the cross-layer sink error. The upstream
            // `PromptWriteSafetyEventUnavailable` reason is the actionable signal.
            .map_err(|_error| MemoryEventSinkError::new("audit sink unavailable"))
    }
}

fn prompt_write_action_kind(operation: PromptWriteOperation) -> &'static str {
    match operation {
        PromptWriteOperation::Write => "write_file",
        PromptWriteOperation::Append => "append_file",
        PromptWriteOperation::Patch => "patch_file",
        PromptWriteOperation::Import => "memory_import",
        PromptWriteOperation::Seed => "memory_seed",
        PromptWriteOperation::ProfileUpdate => "profile_update",
        PromptWriteOperation::AdminSystemPromptUpdate => "admin_system_prompt_update",
    }
}

fn prompt_safety_reason_projection_kind(reason: PromptSafetyReasonCode) -> &'static str {
    match reason {
        PromptSafetyReasonCode::HighRiskPromptInjection => "prompt_high_risk",
        PromptSafetyReasonCode::CriticalPromptInjection => "prompt_critical",
        PromptSafetyReasonCode::PromptWritePolicyUnavailable => "prompt_policy_unavailable",
        PromptSafetyReasonCode::PromptWritePolicyMisconfigured => "prompt_policy_misconfigured",
        PromptSafetyReasonCode::ProtectedPathRegistryUnavailable => "protected_registry_missing",
        PromptSafetyReasonCode::PromptWriteBypassNotAllowed => "prompt_bypass_denied",
        PromptSafetyReasonCode::PromptWriteSafetyEventUnavailable => "prompt_event_unavailable",
    }
}

fn prompt_safety_event_kind_label(kind: PromptWriteSafetyEventKind) -> &'static str {
    match kind {
        PromptWriteSafetyEventKind::Checked => "prompt_write_safety_checked",
        PromptWriteSafetyEventKind::Warned => "prompt_write_safety_warned",
        PromptWriteSafetyEventKind::Rejected => "prompt_write_safety_rejected",
        PromptWriteSafetyEventKind::BypassAllowed => "prompt_write_safety_bypass_allowed",
    }
}

fn encode_prompt_safety_metadata(event: &PromptWriteSafetyEvent) -> String {
    let mut pairs = vec![format!(
        "status={}",
        match event.kind {
            PromptWriteSafetyEventKind::Checked => "checked",
            PromptWriteSafetyEventKind::Warned => "warned",
            PromptWriteSafetyEventKind::Rejected => "rejected",
            PromptWriteSafetyEventKind::BypassAllowed => "bypass_allowed",
        }
    )];
    if let Some(path_hash) = &event.relative_path_hash {
        pairs.push(format!("path_hash={path_hash}"));
    }
    if let Some(path_class) = &event.protected_path_class {
        pairs.push(format!("protected_path_class={}", path_class.as_str()));
    }
    if let Some(reason) = event.reason_code {
        pairs.push(format!("reason={}", reason.as_str()));
    }
    if let Some(severity) = event.severity {
        pairs.push(format!("severity={}", severity.as_str()));
    }
    if event.finding_count > 0 {
        pairs.push(format!("findings={}", event.finding_count));
    }
    format!("memory_prompt_safety:v1;{}", pairs.join(";"))
}

pub(super) fn ensure_memory_mount(
    request: &FirstPartyCapabilityRequest,
    write: bool,
) -> Result<(), FirstPartyCapabilityError> {
    let Some(mounts) = &request.mounts else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    };
    let Some(grant) = mounts
        .mounts
        .iter()
        .find(|grant| grant.alias.as_str() == "/memory" && grant.target.as_str() == "/memory")
    else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    };
    let permissions = &grant.permissions;
    if !permissions.read
        || !permissions.list
        || (write && (!permissions.write || !permissions.delete))
    {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    }
    Ok(())
}

pub(super) fn invocation_for_request(request: &FirstPartyCapabilityRequest) -> MemoryInvocation {
    MemoryInvocation {
        scope: request.scope.clone(),
        correlation_id: CorrelationId::new(),
    }
}

pub(super) fn map_memory_service_error(error: MemoryServiceError) -> FirstPartyCapabilityError {
    match error.kind() {
        MemoryServiceErrorKind::Input => input_error(),
        MemoryServiceErrorKind::Operation | MemoryServiceErrorKind::Unavailable => {
            // Log only the sanitized error kind — the `Display`/`source` chain
            // can carry backend details or host paths.
            tracing::debug!(
                error_kind = ?error.kind(),
                "memory service operation failed"
            );
            operation_error()
        }
    }
}

async fn dispatch_search(
    services: &MemoryServices,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let request =
        MemoryServiceSearchRequest::from_tool_input(input).map_err(map_memory_service_error)?;
    let response = services
        .memory_service
        .search(services.invocation.clone(), request)
        .await
        .map_err(map_memory_service_error)?;
    Ok(search_response_to_value(response))
}

async fn dispatch_write(
    services: &MemoryServices,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let request =
        MemoryServiceWriteRequest::from_tool_input(input).map_err(map_memory_service_error)?;
    let response = services
        .memory_service
        .write(services.invocation.clone(), request)
        .await
        .map_err(map_memory_service_error)?;
    Ok(write_response_to_value(response))
}

async fn dispatch_read(
    services: &MemoryServices,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let request =
        MemoryServiceReadRequest::from_tool_input(input).map_err(map_memory_service_error)?;
    let response = services
        .memory_service
        .read(services.invocation.clone(), request)
        .await
        .map_err(map_memory_service_error)?;
    Ok(read_response_to_value(response))
}

async fn dispatch_tree(
    services: &MemoryServices,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let request =
        MemoryServiceTreeRequest::from_tool_input(input).map_err(map_memory_service_error)?;
    let response = services
        .memory_service
        .tree(services.invocation.clone(), request)
        .await
        .map_err(map_memory_service_error)?;
    Ok(tree_response_to_value(response))
}

fn search_response_to_value(response: MemoryServiceSearchResponse) -> Value {
    let results = response
        .results
        .into_iter()
        .map(|result| {
            json!({
                "content": result.content,
                "score": result.score,
                "path": result.path,
                "is_hybrid_match": result.is_hybrid_match,
            })
        })
        .collect::<Vec<_>>();
    let result_count = results.len();
    json!({
        "query": response.query,
        "results": results,
        "result_count": result_count,
        "search_scope": MEMORY_SEARCH_SCOPE,
        "external_services_searched": false,
    })
}

fn write_response_to_value(response: MemoryServiceWriteResponse) -> Value {
    // Exhaustive over `MemoryWriteStatus`; the `"status"` field still serializes
    // to the same snake_case wire strings (`cleared`/`patched`/`written`).
    match response.status {
        MemoryWriteStatus::Cleared => json!({
            "status": response.status,
            "path": response.path,
            "message": response.message.unwrap_or_default(),
        }),
        MemoryWriteStatus::Patched => json!({
            "status": response.status,
            "path": response.path,
            "replacements": response.replacements.unwrap_or(0),
            "content_length": response.content_length,
        }),
        MemoryWriteStatus::Written => json!({
            "status": response.status,
            "path": response.path,
            "append": response.append,
            "content_length": response.content_length,
        }),
    }
}

fn read_response_to_value(response: MemoryServiceReadResponse) -> Value {
    json!({
        "path": response.path,
        "content": response.content,
        "word_count": response.word_count,
    })
}

fn tree_response_to_value(response: MemoryServiceTreeResponse) -> Value {
    Value::Array(response.entries)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        CapabilityId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
        ResourceScope, TenantId, ThreadId, UserId, VirtualPath,
    };
    use ironclaw_memory::{
        MemoryServiceSearchRequest, MemoryServiceSearchResponse, MemoryServiceSearchResult,
    };

    use crate::memory_binding::MEMORY_DISABLED_BINDING_SENTINEL;
    use crate::{FirstPartyCapabilityRequest, HostProcessPort, InvocationServices};

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingMemoryService {
        seen: Mutex<Vec<(MemoryInvocation, MemoryServiceSearchRequest)>>,
    }

    #[async_trait]
    impl MemoryService for RecordingMemoryService {
        async fn search(
            &self,
            invocation: MemoryInvocation,
            request: MemoryServiceSearchRequest,
        ) -> Result<MemoryServiceSearchResponse, MemoryServiceError> {
            self.seen
                .lock()
                .expect("recording memory service lock should not be poisoned")
                .push((invocation, request));
            Ok(MemoryServiceSearchResponse {
                query: "search marker".to_string(),
                results: vec![MemoryServiceSearchResult {
                    content: "captured through IronClaw memory".to_string(),
                    score: 1.0,
                    path: "notes/alpha.md".to_string(),
                    is_hybrid_match: false,
                }],
            })
        }
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-memory-service").unwrap(),
            user_id: UserId::new("user-memory-service").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: Some(ThreadId::new("thread-memory-service").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    fn memory_mount() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/memory").unwrap(),
            VirtualPath::new("/memory").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap()
    }

    fn memory_request(capability_id: &'static str, input: Value) -> FirstPartyCapabilityRequest {
        FirstPartyCapabilityRequest {
            origin: None,
            run_id: None,
            capability_id: CapabilityId::new(capability_id).unwrap(),
            scope: sample_scope(),
            authenticated_actor_user_id: None,
            estimate: ironclaw_host_api::ResourceEstimate::default(),
            mounts: Some(memory_mount()),
            services: InvocationServices {
                filesystem: Arc::new(InMemoryBackend::new()),
                runtime_http_egress: None,
                tool_call_http_egress: None,
                runtime_secret_material_stager: None,
                process: Arc::new(HostProcessPort::new()),
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
                post_edit_check: None,
            },
            input,
        }
    }

    #[tokio::test]
    async fn native_memory_search_dispatches_through_memory_service_facade() {
        let memory_service = Arc::new(RecordingMemoryService::default());
        let state = MemoryCapabilityState::with_memory_service_for_test(memory_service.clone());
        let request = memory_request(
            MEMORY_SEARCH_CAPABILITY_ID,
            json!({"query": "search marker", "limit": 3}),
        );

        let result = dispatch(&state, &request)
            .await
            .expect("memory_search should succeed through IronClaw memory facade");

        assert_eq!(result.output["result_count"], 1);
        assert_eq!(
            result.output["search_scope"],
            "reborn_internal_persistent_memory"
        );
        assert_eq!(result.output["external_services_searched"], false);
        assert_eq!(
            result.output["results"][0]["content"],
            "captured through IronClaw memory"
        );
        let seen = memory_service
            .seen
            .lock()
            .expect("recording memory service lock should not be poisoned");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0.scope.tenant_id.as_str(), "tenant-memory-service");
        assert_eq!(seen[0].0.scope.user_id.as_str(), "user-memory-service");
        assert_eq!(seen[0].1.query, "search marker");
        assert_eq!(seen[0].1.limit, 3);
    }

    #[tokio::test]
    async fn disabled_binding_fails_closed_at_dispatch() {
        // Drive the real caller (dispatch -> service_for -> resolver) with a
        // resolver whose document store is disabled (no test-override service),
        // proving it fails closed instead of silently building native.
        let state = MemoryCapabilityState::with_resolver(document_store_resolver(
            MEMORY_DISABLED_BINDING_SENTINEL,
        ));
        let request = memory_request(
            MEMORY_SEARCH_CAPABILITY_ID,
            json!({"query": "search marker", "limit": 3}),
        );

        let err = dispatch(&state, &request)
            .await
            .expect_err("disabled binding must fail closed");
        assert_eq!(
            err.kind(),
            Some(ironclaw_host_api::RuntimeDispatchErrorKind::OperationFailed)
        );
        assert_eq!(
            err.safe_summary(),
            Some("memory is unavailable for the configured provider binding")
        );
    }

    #[tokio::test]
    async fn third_party_binding_fails_closed_at_dispatch() {
        let state = MemoryCapabilityState::with_resolver(document_store_resolver("acme.honcho"));
        let request = memory_request(MEMORY_READ_CAPABILITY_ID, json!({"path": "notes/alpha.md"}));

        let err = dispatch(&state, &request)
            .await
            .expect_err("unimplemented third-party binding must fail closed");
        assert_eq!(
            err.kind(),
            Some(ironclaw_host_api::RuntimeDispatchErrorKind::OperationFailed)
        );
    }

    #[tokio::test]
    async fn third_party_binding_dispatches_to_registered_provider() {
        // The third-party binding is permitted, and a provider instance is
        // registered for its id, so the *model-facing memory tool* dispatches
        // through to that provider rather than failing closed — proving a mem0
        // (or any third-party) binding transparently swaps the service behind the
        // same `ironclaw.memory.*` tools, through the real resolver path.
        let provider = Arc::new(RecordingMemoryService::default());
        let resolver = document_store_resolver("acme.honcho")
            .with_third_party_document_store_provider(
                "acme.honcho",
                provider.clone() as Arc<dyn MemoryService>,
            );
        let state = MemoryCapabilityState::with_resolver(resolver);
        let request = memory_request(
            MEMORY_SEARCH_CAPABILITY_ID,
            json!({"query": "search marker", "limit": 3}),
        );

        let result = dispatch(&state, &request)
            .await
            .expect("registered third-party provider must dispatch");
        assert_eq!(
            result.output["results"][0]["content"],
            "captured through IronClaw memory"
        );
        // The dispatch actually reached the registered third-party provider.
        assert_eq!(
            provider
                .seen
                .lock()
                .expect("recording memory service lock should not be poisoned")
                .len(),
            1
        );
    }

    /// A resolver whose document-store profile is bound to `extension_id`
    /// (e.g. `memory.disabled` or a third party), for driving fail-closed
    /// dispatch through the real resolver path.
    fn document_store_resolver(extension_id: &str) -> MemoryServiceResolver {
        use crate::memory_binding::{
            MemoryBindingInput, MemoryBindingPolicy, MemoryDeploymentProfile,
        };
        let policy = MemoryBindingPolicy::resolve(MemoryBindingInput {
            provider: Some(extension_id.to_string()),
            ..MemoryBindingInput::native_default(MemoryDeploymentProfile::LocalDev)
        })
        .expect("policy resolves");
        MemoryServiceResolver::from_policy(policy)
    }
}
