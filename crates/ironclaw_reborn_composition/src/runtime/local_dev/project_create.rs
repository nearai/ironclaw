use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_loop_support::CapabilityResultWrite;
use ironclaw_product_workflow::{
    ProjectCaller, ProjectService, ProjectServiceError, RebornCreateProjectRequest,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailure, CapabilityFailureKind,
    CapabilityOutcome, CapabilityProgress, CapabilityResultMessage, ConcurrencyHint,
    LoopRunContext,
};

use crate::runtime::local_dev::synthetic_capability::{
    LocalDevSyntheticCapability, LocalDevSyntheticCapabilityDescriptor,
    LocalDevSyntheticCapabilityHandler, LocalDevSyntheticCapabilityInvocation,
};

pub(crate) const PROJECT_CREATE_CAPABILITY_ID: &str = "builtin.project_create";
const PROJECT_CREATE_PROVIDER_TOOL_NAME: &str = "builtin__project_create";
const PROJECT_CREATE_DESCRIPTION: &str = "Create a new first-class project owned by the current \
    user. Use this when the user asks to create, start, or set up a new project. The new project \
    appears in the Projects list once created.";
/// Mirrors `ironclaw_projects::MAX_PROJECT_NAME_BYTES`; surfaced in the schema so
/// the model self-limits before the service rejects an oversized name.
const MAX_PROJECT_NAME_BYTES: usize = 200;

pub(super) fn project_create_capability(
    project_service: Arc<dyn ProjectService>,
    fallback_user_id: UserId,
) -> Result<LocalDevSyntheticCapability, AgentLoopHostError> {
    Ok(LocalDevSyntheticCapability::new(
        LocalDevSyntheticCapabilityDescriptor::new(
            PROJECT_CREATE_CAPABILITY_ID,
            PROJECT_CREATE_PROVIDER_TOOL_NAME,
            PROJECT_CREATE_DESCRIPTION,
            ConcurrencyHint::Exclusive,
            project_create_input_schema(),
        )?,
        Arc::new(ProjectCreateHandler {
            project_service,
            fallback_user_id,
        }),
    ))
}

struct ProjectCreateHandler {
    project_service: Arc<dyn ProjectService>,
    fallback_user_id: UserId,
}

#[async_trait]
impl LocalDevSyntheticCapabilityHandler for ProjectCreateHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_project_create_input(arguments).map(|_| ())
    }

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let input = parse_project_create_input(&invocation.input)?;
        // Identity is authority-bearing: the caller is derived from the trusted
        // run scope, never from the model's arguments. The capability accepts
        // only presentation/content fields (name, description) — never
        // membership or ACL data, which stays control-plane and must never be
        // agent-writable.
        let caller = ProjectCaller {
            tenant_id: invocation.run_context.scope.tenant_id.clone(),
            user_id: effective_user_id(&invocation.run_context, &self.fallback_user_id),
        };
        let request = RebornCreateProjectRequest {
            name: input.name,
            description: input.description,
            icon: None,
            color: None,
            metadata: None,
        };
        let response = match self.project_service.create_project(caller, request).await {
            Ok(response) => response,
            Err(error) => return project_service_outcome(error),
        };
        let project = response.project;
        let output = serde_json::json!({
            "project_id": project.project_id,
            "name": project.name,
        });
        // The safe summary must not interpolate the raw, model-controlled project
        // name: a name containing a payload/path delimiter (`/ < > { } [ ] ` + "`"
        // + ` \`) fails `ToolResultSafeSummary` validation in
        // `append_capability_result_ref`, which surfaces as a terminal
        // `HostUnavailable` that kills the whole turn. The model still gets the
        // name and id from the result `output`; the summary stays a fixed,
        // delimiter-free string.
        let safe_summary = "created project".to_string();
        let write_result = invocation
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &invocation.run_context,
                input_ref: invocation.effective_input_ref(),
                invocation_id: invocation.invocation_id(),
                capability_id: &invocation.request.capability_id,
                output,
                display_preview: None,
            })
            .await?;
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: write_result.result_ref,
            safe_summary,
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: write_result.byte_len,
            output_digest: write_result.output_digest,
        }))
    }
}

#[derive(Debug)]
struct ProjectCreateInput {
    name: String,
    description: String,
}

fn parse_project_create_input(
    input: &serde_json::Value,
) -> Result<ProjectCreateInput, AgentLoopHostError> {
    let name = input
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "project_create requires a non-empty name",
            )
        })?
        .to_string();
    // `description` is optional; default to empty. A non-string `description`
    // is a malformed argument rather than an omitted one.
    let description = match input.get("description") {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(description)) => description.trim().to_string(),
        Some(_) => {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "project_create description must be a string",
            ));
        }
    };
    Ok(ProjectCreateInput { name, description })
}

fn project_create_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_PROJECT_NAME_BYTES,
                "description": "Short, human-readable project name"
            },
            "description": {
                "type": "string",
                "description": "Optional one-line description of the project's purpose"
            }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

/// Disposition a project-service failure into either a model-visible,
/// recoverable capability failure or a terminal host error.
///
/// As with skill activation, the two arms map onto the executor's two failure
/// paths: `CapabilityOutcome::Failed` is handed back to the model and the run
/// continues (so the model can fix its input or tell the user), while an
/// `Err(AgentLoopHostError)` becomes a run-ending `HostUnavailable`. Only a
/// genuine internal bug stays terminal — invalid input, conflicts, denials, and
/// transient unavailability are all surfaced to the model instead of killing
/// the turn.
fn project_service_outcome(
    error: ProjectServiceError,
) -> Result<CapabilityOutcome, AgentLoopHostError> {
    let (error_kind, safe_summary) = match error {
        // Keep the safe summary fixed and host-authored — `field` is a
        // free-form `String` and could carry a forbidden delimiter/marker
        // that would remap this recoverable arm into a terminal
        // `HostUnavailable` (see .claude/rules/agent-loop-capabilities.md,
        // Invariant 2). The offending field name is the model's own input,
        // which it already has; it does not belong in the summary.
        ProjectServiceError::InvalidInput { .. } => (
            CapabilityFailureKind::InvalidInput,
            "invalid project input".to_string(),
        ),
        ProjectServiceError::Conflict => (
            CapabilityFailureKind::OperationFailed,
            "a project with that identity already exists".to_string(),
        ),
        ProjectServiceError::Denied => (
            CapabilityFailureKind::PolicyDenied,
            "not permitted to create this project".to_string(),
        ),
        ProjectServiceError::NotFound => (
            CapabilityFailureKind::OperationFailed,
            "project creation failed".to_string(),
        ),
        ProjectServiceError::Unavailable => (
            CapabilityFailureKind::Unavailable,
            "project service temporarily unavailable".to_string(),
        ),
        ProjectServiceError::Internal => {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "project creation failed",
            ));
        }
    };
    Ok(CapabilityOutcome::Failed(CapabilityFailure {
        error_kind,
        safe_summary,
        detail: None,
    }))
}

/// Resolve the user the run acts on behalf of: the explicit thread owner, else
/// the run actor, else the local-dev fallback. Mirrors the same resolution used
/// by the outbound-delivery capabilities so all local-dev synthetic
/// capabilities scope to one identity.
fn effective_user_id(run_context: &LoopRunContext, fallback_user_id: &UserId) -> UserId {
    run_context
        .scope
        .explicit_owner_user_id()
        .cloned()
        .or_else(|| {
            run_context
                .actor
                .as_ref()
                .map(|actor| actor.user_id.clone())
        })
        .unwrap_or_else(|| fallback_user_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use ironclaw_host_api::{
        AgentId, ApprovalRequestId, CapabilityId, CorrelationId, ProjectId, ResourceEstimate,
        TenantId, ThreadId,
    };
    use ironclaw_loop_support::{
        CapabilityWriteResult, EmptyLoopCapabilityPort, LoopCapabilityInputResolver,
        LoopCapabilityResultWriter,
    };
    use ironclaw_product_workflow::{
        RebornAddMemberRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
        RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
        RebornListProjectsResponse, RebornProjectInfo, RebornProjectMemberInfo,
        RebornProjectResponse, RebornProjectRole, RebornProjectState, RebornRemoveMemberRequest,
        RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
    };
    use ironclaw_turns::{
        CapabilityActivityId, LoopResultRef, RunProfileResolutionRequest, RunProfileResolver,
        TurnId, TurnRunId, TurnScope,
        run_profile::{
            CapabilityApprovalResume, CapabilityInputRef, CapabilityInvocation,
            CapabilityResumeToken, InMemoryRunProfileResolver, ProviderToolCall,
            RegisterProviderToolCallRequest, VisibleCapabilityRequest,
        },
    };

    use crate::runtime::local_dev::synthetic_capability::wrap_local_dev_synthetic_capabilities;

    struct FixedProjectInputResolver {
        input_ref: CapabilityInputRef,
    }

    #[async_trait]
    impl LoopCapabilityInputResolver for FixedProjectInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            Ok(serde_json::json!({
                "name": "Original Project",
                "description": "from original input"
            }))
        }

        async fn register_provider_tool_call_input(
            &self,
            _run_context: &LoopRunContext,
            _tool_call: &ProviderToolCall,
        ) -> Result<CapabilityInputRef, AgentLoopHostError> {
            Ok(self.input_ref.clone())
        }
    }

    #[derive(Debug)]
    struct RecordedProjectResultWrite {
        input_ref: CapabilityInputRef,
        invocation_id: ironclaw_host_api::InvocationId,
        capability_id: CapabilityId,
        output: serde_json::Value,
    }

    struct RecordingProjectResultWriter {
        writes: Arc<std::sync::Mutex<Vec<RecordedProjectResultWrite>>>,
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for RecordingProjectResultWriter {
        async fn write_capability_result(
            &self,
            write: CapabilityResultWrite<'_>,
        ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
            self.writes
                .lock()
                .expect("project result writes lock")
                .push(RecordedProjectResultWrite {
                    input_ref: write.input_ref.clone(),
                    invocation_id: write.invocation_id,
                    capability_id: write.capability_id.clone(),
                    output: write.output.clone(),
                });
            Ok(CapabilityWriteResult::without_output_digest(
                LoopResultRef::new("result:project-create").expect("valid result ref"),
                0,
            ))
        }
    }

    struct FakeProjectService;

    #[async_trait]
    impl ProjectService for FakeProjectService {
        async fn list_projects(
            &self,
            _caller: ProjectCaller,
            _request: RebornListProjectsRequest,
        ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
            Ok(RebornListProjectsResponse { projects: vec![] })
        }

        async fn create_project(
            &self,
            _caller: ProjectCaller,
            request: RebornCreateProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            let now = Utc::now();
            Ok(RebornProjectResponse {
                project: RebornProjectInfo {
                    project_id: "project-created".to_string(),
                    name: request.name,
                    description: request.description,
                    icon: None,
                    color: None,
                    metadata: serde_json::json!({}),
                    state: RebornProjectState::Active,
                    role: RebornProjectRole::Owner,
                    created_at: now,
                    updated_at: now,
                },
            })
        }

        async fn get_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornGetProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            Err(ProjectServiceError::Unavailable)
        }

        async fn update_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornUpdateProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            Err(ProjectServiceError::Unavailable)
        }

        async fn delete_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornDeleteProjectRequest,
        ) -> Result<(), ProjectServiceError> {
            Err(ProjectServiceError::Unavailable)
        }

        async fn list_members(
            &self,
            _caller: ProjectCaller,
            _request: RebornListMembersRequest,
        ) -> Result<RebornListMembersResponse, ProjectServiceError> {
            Err(ProjectServiceError::Unavailable)
        }

        async fn add_member(
            &self,
            _caller: ProjectCaller,
            _request: RebornAddMemberRequest,
        ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
            Err(ProjectServiceError::Unavailable)
        }

        async fn update_member_role(
            &self,
            _caller: ProjectCaller,
            _request: RebornUpdateMemberRoleRequest,
        ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
            Err(ProjectServiceError::Unavailable)
        }

        async fn remove_member(
            &self,
            _caller: ProjectCaller,
            _request: RebornRemoveMemberRequest,
        ) -> Result<(), ProjectServiceError> {
            Err(ProjectServiceError::Unavailable)
        }
    }

    async fn project_run_context() -> LoopRunContext {
        let profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new("tenant-project-create").expect("tenant id"),
                Some(AgentId::new("agent-project-create").expect("agent id")),
                Some(ProjectId::new("project-existing").expect("project id")),
                ThreadId::new("thread-project-create").expect("thread id"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            profile,
        )
    }

    fn project_provider_tool_call() -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            turn_id: Some("turn-project-create".to_string()),
            id: "call-project-create".to_string(),
            name: ironclaw_host_api::ProviderToolName::new(PROJECT_CREATE_PROVIDER_TOOL_NAME)
                .expect("provider tool name"),
            arguments: serde_json::json!({
                "name": "Original Project",
                "description": "from original input"
            }),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    #[tokio::test]
    async fn project_create_resume_writes_result_under_effective_input_ref() {
        let run_context = project_run_context().await;
        let original_input_ref =
            CapabilityInputRef::new("input:project-create-original").expect("input ref");
        let resumed_input_ref =
            CapabilityInputRef::new("input:project-create-resumed").expect("input ref");
        let result_writes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let capability = project_create_capability(
            Arc::new(FakeProjectService),
            UserId::new("user-project-create").expect("user id"),
        )
        .expect("project create capability");
        let port = wrap_local_dev_synthetic_capabilities(
            Arc::new(EmptyLoopCapabilityPort),
            vec![capability],
            run_context.clone(),
            Arc::new(FixedProjectInputResolver {
                input_ref: original_input_ref,
            }),
            Arc::new(RecordingProjectResultWriter {
                writes: Arc::clone(&result_writes),
            }),
            None,
        )
        .expect("synthetic port");
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible capabilities");
        let activity_id = CapabilityActivityId::new();
        let candidate = port
            .register_provider_tool_call(RegisterProviderToolCallRequest::for_activity(
                project_provider_tool_call(),
                activity_id,
            ))
            .await
            .expect("provider call registers");

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                activity_id,
                surface_version: candidate.surface_version,
                capability_id: candidate.capability_id.clone(),
                input_ref: candidate.input_ref.clone(),
                approval_resume: Some(CapabilityApprovalResume {
                    approval_request_id: ApprovalRequestId::new(),
                    resume_token: CapabilityResumeToken::new(activity_id.to_string())
                        .expect("resume token"),
                    correlation_id: CorrelationId::new(),
                    input_ref: resumed_input_ref.clone(),
                    input: serde_json::json!({
                        "name": "Resumed Project",
                        "description": "from resumed input"
                    }),
                    estimate: ResourceEstimate::default(),
                }),
                auth_resume: None,
            })
            .await
            .expect("project create invocation completes");

        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        let writes = result_writes.lock().expect("project result writes lock");
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].input_ref, resumed_input_ref);
        assert_eq!(
            writes[0].invocation_id,
            ironclaw_host_api::InvocationId::from_uuid(activity_id.as_uuid())
        );
        assert_eq!(writes[0].capability_id, candidate.capability_id);
        assert_eq!(writes[0].output["name"], "Resumed Project");
    }

    #[test]
    fn parse_project_create_input_rejects_missing_name() {
        let error = parse_project_create_input(&serde_json::json!({}))
            .expect_err("missing name should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn parse_project_create_input_rejects_blank_name() {
        let error = parse_project_create_input(&serde_json::json!({"name": "   "}))
            .expect_err("blank name should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn parse_project_create_input_trims_and_defaults_description() {
        let input = parse_project_create_input(&serde_json::json!({"name": "  Build IronClaw  "}))
            .expect("valid name should parse");

        assert_eq!(input.name, "Build IronClaw");
        assert_eq!(input.description, "");
    }

    #[test]
    fn parse_project_create_input_rejects_non_string_description() {
        let error = parse_project_create_input(&serde_json::json!({"name": "x", "description": 7}))
            .expect_err("non-string description should fail");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn invalid_input_is_a_recoverable_tool_failure_not_terminal() {
        let outcome = project_service_outcome(ProjectServiceError::InvalidInput {
            field: "name".to_string(),
        })
        .expect("invalid input must be a model-visible failure, not terminal");

        match outcome {
            CapabilityOutcome::Failed(failure) => {
                assert_eq!(failure.error_kind, CapabilityFailureKind::InvalidInput);
            }
            other => panic!("expected CapabilityOutcome::Failed, got {other:?}"),
        }
    }

    #[test]
    fn unavailable_is_recoverable_not_terminal() {
        let outcome = project_service_outcome(ProjectServiceError::Unavailable)
            .expect("transient unavailability must not kill the run");

        match outcome {
            CapabilityOutcome::Failed(failure) => {
                assert_eq!(failure.error_kind, CapabilityFailureKind::Unavailable);
            }
            other => panic!("expected CapabilityOutcome::Failed, got {other:?}"),
        }
    }

    #[test]
    fn internal_error_stays_terminal() {
        let error = project_service_outcome(ProjectServiceError::Internal)
            .expect_err("internal bugs must stay terminal");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    }
}
