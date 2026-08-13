use std::sync::Arc;

use ironclaw_extension_registry::{CapabilityVisibility, SharedExtensionRegistry};
use ironclaw_host_api::{
    execution_policy::TurnExecutionPolicy,
    ids::ThreadId,
    resource::ResourceScope,
    turn::{ProductTurnContext, TurnId, TurnOriginKind, TurnOwner, TurnRunId, TurnScope},
};
use ironclaw_loop_contracts::{LoopRunContext, ResolvedRunProfile};
use ironclaw_triggers::{TriggerError, TriggerRecordValidationKind};

use super::ComposedSelectableSkillContextSource;
use crate::factory::TriggerExecutionPolicyPreflight;

pub(super) struct StructuredTriggerExecutionPreflight {
    registry: Arc<SharedExtensionRegistry>,
    skills: Option<Arc<ComposedSelectableSkillContextSource>>,
    resolved_run_profile: ResolvedRunProfile,
    /// Whether the runner will attach the tool-disclosure bridge decorator to
    /// fired runs (`ToolDisclosureMode::is_enabled()`). When it will not, a
    /// synthetic `ironclaw.tool_*` bridge id must fail preflight — accepting
    /// it would arm an allowlist whose only entries never exist at fire time.
    bridge_capabilities_enabled: bool,
}

impl StructuredTriggerExecutionPreflight {
    pub(super) fn new(
        registry: Arc<SharedExtensionRegistry>,
        skills: Option<Arc<ComposedSelectableSkillContextSource>>,
        resolved_run_profile: ResolvedRunProfile,
        bridge_capabilities_enabled: bool,
    ) -> Self {
        Self {
            registry,
            skills,
            resolved_run_profile,
            bridge_capabilities_enabled,
        }
    }
}

#[async_trait::async_trait]
impl TriggerExecutionPolicyPreflight for StructuredTriggerExecutionPreflight {
    async fn validate(
        &self,
        scope: &ResourceScope,
        policy: &TurnExecutionPolicy,
    ) -> Result<(), TriggerError> {
        if let Some(allowed) = &policy.allowed_capability_ids {
            let registry = self.registry.snapshot();
            for capability_id in allowed {
                // Fail closed: a capability with no visibility record is not
                // model-visible; assuming the permissive default here would
                // admit an allowlist entry that validation cannot vouch for.
                let registered_and_visible = registry.get_capability(capability_id).is_some()
                    && registry.capability_visibility(capability_id)
                        == Some(CapabilityVisibility::Model);
                let synthetic_bridge = self.bridge_capabilities_enabled
                    && ironclaw_loop_host::bridge_capability_ids()
                        .any(|bridge_id| bridge_id == *capability_id);
                if !registered_and_visible && !synthetic_bridge {
                    return Err(invalid_reference(format!(
                        "capability {capability_id} is not available on the model-visible surface"
                    )));
                }
            }
        }
        if policy.required_skills.is_empty() {
            return Ok(());
        }
        let skills = self.skills.as_ref().ok_or_else(|| {
            invalid_reference(
                "required-skill validation is unavailable for this runtime".to_string(),
            )
        })?;
        let thread_id = match scope.thread_id.clone() {
            Some(thread_id) => thread_id,
            None => ThreadId::new("structured-trigger-preflight")
                .map_err(|error| invalid_reference(error.to_string()))?,
        };
        let turn_scope = TurnScope::new_with_owner(
            scope.tenant_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            thread_id,
            Some(scope.user_id.clone()),
        );
        let mut run_context = LoopRunContext::new(
            turn_scope,
            TurnId::new(),
            TurnRunId::new(),
            self.resolved_run_profile.clone(),
        );
        let mut product_context = ProductTurnContext::new(
            TurnOriginKind::ScheduledTrigger,
            None,
            None,
            TurnOwner::Personal {
                user: scope.user_id.clone(),
            },
        );
        product_context.execution_policy = Some(policy.clone());
        run_context.product_context = Some(product_context);
        let required = policy
            .required_skills
            .iter()
            .map(|skill| skill.as_str().to_string())
            .collect::<Vec<_>>();
        skills
            .validate_required_skills(&run_context, &required)
            .await
            .map_err(|error| invalid_reference(error.to_string()))
    }
}

fn invalid_reference(reason: String) -> TriggerError {
    TriggerError::InvalidRecord {
        kind: TriggerRecordValidationKind::ExecutionSpecInvalid,
        reason,
    }
}
