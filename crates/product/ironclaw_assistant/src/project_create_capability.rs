//! Loop-facing project creation capability.

use std::sync::Arc;

use crate::{ProjectCaller, ProjectService, ProjectServiceError, RebornCreateProjectRequest};
use async_trait::async_trait;
use ironclaw_host_api::{
    ids::{InvocationId, UserId},
    resolution::Resolution,
    result_meta::FailureKind,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailureDetail, CapabilityProgress,
    ConcurrencyHint, LoopRunContext, resolution,
};
use ironclaw_loop_host::{
    CapabilityResultWrite, DurablePersistence, SyntheticCapability, SyntheticCapabilityDescriptor,
    SyntheticCapabilityHandler, SyntheticCapabilityInvocation,
};

pub const PROJECT_CREATE_CAPABILITY_ID: &str = "builtin.project_create";
const PROJECT_CREATE_PROVIDER_TOOL_NAME: &str = "builtin__project_create";
const PROJECT_CREATE_DESCRIPTION: &str = "Create a new first-class project owned by the current \
    user. Use this when the user asks to create, start, or set up a new project. The new project \
    appears in the Projects list once created.";
/// Mirrors `ironclaw_identity::projects::MAX_PROJECT_NAME_BYTES`; surfaced in the schema so
/// the model self-limits before the service rejects an oversized name.
const MAX_PROJECT_NAME_BYTES: usize = 200;

pub fn project_create_capability(
    project_service: Arc<dyn ProjectService>,
    fallback_user_id: UserId,
) -> Result<SyntheticCapability, AgentLoopHostError> {
    Ok(SyntheticCapability::new(
        SyntheticCapabilityDescriptor::new(
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
impl SyntheticCapabilityHandler for ProjectCreateHandler {
    fn validate_provider_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        parse_project_create_input(arguments).map(|_| ())
    }

    async fn invoke(
        &self,
        invocation: SyntheticCapabilityInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
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
                input_ref: &invocation.request.input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &invocation.request.capability_id,
                output,
                display_preview: None,
                durable_persistence: DurablePersistence::Persist,
            })
            .await?;
        Ok(resolution::completed(
            write_result.result_ref,
            safe_summary,
            CapabilityProgress::MadeProgress,
            false,
            write_result.byte_len,
            write_result.output_digest,
            write_result.model_observation,
        ))
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
fn project_service_outcome(error: ProjectServiceError) -> Result<Resolution, AgentLoopHostError> {
    let (error_kind, safe_summary) = match error {
        // Keep the safe summary fixed and host-authored — `field` is a
        // free-form `String` and could carry a forbidden delimiter/marker
        // that would remap this recoverable arm into a terminal
        // `HostUnavailable` (see the capability-access contract,
        // Invariant 2). The offending field name is the model's own input,
        // which it already has; it does not belong in the summary.
        ProjectServiceError::InvalidInput { .. } => (
            FailureKind::InputEncode,
            "invalid project input".to_string(),
        ),
        ProjectServiceError::Conflict => (
            FailureKind::OperationFailed,
            "a project with that identity already exists".to_string(),
        ),
        ProjectServiceError::Denied => (
            FailureKind::PolicyDenied,
            "not permitted to create this project".to_string(),
        ),
        ProjectServiceError::NotFound => (
            FailureKind::OperationFailed,
            "project creation failed".to_string(),
        ),
        ProjectServiceError::Unavailable => (
            FailureKind::Unavailable,
            "project service temporarily unavailable".to_string(),
        ),
        ProjectServiceError::Internal => {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "project creation failed",
            ));
        }
    };
    Ok(diagnostic_failure(error_kind, safe_summary))
}

fn diagnostic_failure(error_kind: FailureKind, safe_summary: String) -> Resolution {
    resolution::failed(
        error_kind,
        safe_summary.clone(),
        CapabilityFailureDetail::Diagnostic { text: safe_summary },
    )
}

/// Resolve the user the run acts on behalf of: the explicit thread owner, else
/// the run actor, else the configured fallback. Mirrors the same resolution used
/// by the outbound-delivery capabilities so all capability-host synthetic
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
    fn assert_recoverable_failure(
        resolution: &Resolution,
        expected_kind: ironclaw_host_api::result_meta::FailureKind,
    ) {
        match resolution {
            Resolution::Done(outcome) => {
                assert_eq!(outcome.verdict.error_kind(), Some(&expected_kind));
                assert!(
                    outcome.verdict.diagnostic().is_some(),
                    "recoverable failures must carry a model-visible cause"
                );
            }
            other => panic!("expected recoverable failure, got {other:?}"),
        }
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

        assert_recoverable_failure(
            &outcome,
            ironclaw_host_api::result_meta::FailureKind::InputEncode,
        );
    }

    #[test]
    fn unavailable_is_recoverable_not_terminal() {
        let outcome = project_service_outcome(ProjectServiceError::Unavailable)
            .expect("transient unavailability must not kill the run");

        assert_recoverable_failure(
            &outcome,
            ironclaw_host_api::result_meta::FailureKind::Unavailable,
        );
    }

    #[test]
    fn internal_error_stays_terminal() {
        let error = project_service_outcome(ProjectServiceError::Internal)
            .expect_err("internal bugs must stay terminal");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Internal);
    }
}
