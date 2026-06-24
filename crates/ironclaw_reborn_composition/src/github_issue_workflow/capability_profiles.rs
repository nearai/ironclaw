//! Stage capability profiles, run-profile registration, and the subagent
//! definition/flavor resolvers for the GitHub issue workflow.
//!
//! This module owns the static per-stage capability allowlists, the
//! [`CapabilitySurfaceProfileResolver`] that narrows a stage turn's capability
//! surface, the run-profile registration that marks workflow stage turns as
//! non-interactive for budget, and the subagent flavor catalog / definition
//! resolver. The shared capability-id and capability-array consts and the
//! per-stage profile-id consts live in the parent module and are referenced
//! here via `super::`.

use std::sync::Arc;

use async_trait::async_trait;
#[cfg(any(test, feature = "test-support"))]
use ironclaw_loop_support::build_spawn_subagent_parameters_schema;
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    SpawnSubagentFlavorDescriptor, SubagentDefinition, SubagentDefinitionResolver, SubagentKindId,
};
use ironclaw_turns::{
    RunProfileRequest,
    run_profile::{
        CapabilitySurfaceProfileId, InMemoryRunProfileRegistry, InMemoryRunProfileResolver,
        LoopRunContext, RunProfileDefinition, RunProfileRegistryError,
    },
};

use ironclaw_github_issue_workflow::GithubIssueStage;
use ironclaw_host_api::CapabilityId;

#[cfg(any(test, feature = "test-support"))]
use super::NON_WORKFLOW_DEFAULT_CAPABILITIES;
use super::{
    GITHUB_BUG_CI_REPAIR_PROFILE_ID, GITHUB_BUG_IMPLEMENTATION_PROFILE_ID,
    GITHUB_BUG_PLANNING_PROFILE_ID, GITHUB_BUG_PR_SYNTHESIS_PROFILE_ID,
    GITHUB_BUG_REVIEW_RESPONSE_PROFILE_ID, GITHUB_BUG_TRIAGE_PROFILE_ID,
    IMPLEMENTATION_CAPABILITIES, PR_SYNTHESIS_CAPABILITIES, SUBAGENT_RUN_PROFILE_ID,
    TRIAGE_PLANNING_CAPABILITIES, WORKFLOW_ADAPTER_ID, WORKFLOW_SUBAGENT_CAPABILITIES,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GithubIssueWorkflowCapabilityProfile {
    pub(crate) profile_id: &'static str,
    pub(crate) allowed_capabilities: &'static [&'static str],
}

const GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES: &[GithubIssueWorkflowCapabilityProfile] = &[
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_TRIAGE_PROFILE_ID,
        allowed_capabilities: TRIAGE_PLANNING_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_PLANNING_PROFILE_ID,
        allowed_capabilities: TRIAGE_PLANNING_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_IMPLEMENTATION_PROFILE_ID,
        allowed_capabilities: IMPLEMENTATION_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_PR_SYNTHESIS_PROFILE_ID,
        allowed_capabilities: PR_SYNTHESIS_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_CI_REPAIR_PROFILE_ID,
        allowed_capabilities: IMPLEMENTATION_CAPABILITIES,
    },
    GithubIssueWorkflowCapabilityProfile {
        profile_id: GITHUB_BUG_REVIEW_RESPONSE_PROFILE_ID,
        allowed_capabilities: PR_SYNTHESIS_CAPABILITIES,
    },
];

#[cfg(any(test, feature = "test-support"))]
const NON_WORKFLOW_DEFAULT_PROFILE: GithubIssueWorkflowCapabilityProfile =
    GithubIssueWorkflowCapabilityProfile {
        profile_id: ironclaw_reborn::planned_driver_factory::PLANNED_DEFAULT_PROFILE_ID,
        allowed_capabilities: NON_WORKFLOW_DEFAULT_CAPABILITIES,
    };

pub(crate) fn stage_capability_profile_id(stage: &GithubIssueStage) -> &'static str {
    match stage {
        GithubIssueStage::Triage => GITHUB_BUG_TRIAGE_PROFILE_ID,
        GithubIssueStage::Planning => GITHUB_BUG_PLANNING_PROFILE_ID,
        GithubIssueStage::Implementation => GITHUB_BUG_IMPLEMENTATION_PROFILE_ID,
        GithubIssueStage::PrSynthesis => GITHUB_BUG_PR_SYNTHESIS_PROFILE_ID,
        GithubIssueStage::CiRepair => GITHUB_BUG_CI_REPAIR_PROFILE_ID,
        GithubIssueStage::ReviewResponse => GITHUB_BUG_REVIEW_RESPONSE_PROFILE_ID,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn stage_capability_profiles() -> &'static [GithubIssueWorkflowCapabilityProfile] {
    GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn non_workflow_default_capability_profile() -> GithubIssueWorkflowCapabilityProfile {
    NON_WORKFLOW_DEFAULT_PROFILE
}

fn stage_profile_for_surface_id(
    profile_id: &CapabilitySurfaceProfileId,
) -> Option<&'static GithubIssueWorkflowCapabilityProfile> {
    GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES
        .iter()
        .find(|profile| profile.profile_id == profile_id.as_str())
}

fn capability_allow_set_for_profile(
    profile: &GithubIssueWorkflowCapabilityProfile,
) -> Result<CapabilityAllowSet, CapabilityResolveError> {
    let ids = profile.allowed_capabilities.iter().map(|capability| {
        CapabilityId::new(*capability).map_err(|reason| {
            CapabilityResolveError::internal(format!(
                "invalid static GitHub issue workflow capability id {capability}: {reason}"
            ))
        })
    });
    ids.collect::<Result<Vec<_>, _>>()
        .map(CapabilityAllowSet::allowlist)
}

pub(crate) fn allowed_capabilities_for_stage_profile_id(
    profile_id: &CapabilitySurfaceProfileId,
) -> Result<Option<CapabilityAllowSet>, CapabilityResolveError> {
    stage_profile_for_surface_id(profile_id)
        .map(capability_allow_set_for_profile)
        .transpose()
}

pub(crate) fn allowed_capabilities_for_workflow_subagent_profile_id(
    profile_id: &CapabilitySurfaceProfileId,
) -> Result<Option<CapabilityAllowSet>, CapabilityResolveError> {
    if profile_id.as_str()
        != ironclaw_reborn::planned_driver_factory::SUBAGENT_CAPABILITY_SURFACE_PROFILE_ID
    {
        return Ok(None);
    }
    capability_allow_set_for_ids(WORKFLOW_SUBAGENT_CAPABILITIES).map(Some)
}

fn capability_allow_set_for_ids(
    capability_ids: &[&str],
) -> Result<CapabilityAllowSet, CapabilityResolveError> {
    let ids = capability_ids.iter().map(|capability| {
        CapabilityId::new(*capability).map_err(|reason| {
            CapabilityResolveError::internal(format!(
                "invalid static GitHub issue workflow capability id {capability}: {reason}"
            ))
        })
    });
    ids.collect::<Result<Vec<_>, _>>()
        .map(CapabilityAllowSet::allowlist)
}

pub(crate) struct GithubIssueWorkflowCapabilitySurfaceResolver {
    inner: Arc<dyn CapabilitySurfaceProfileResolver>,
}

impl GithubIssueWorkflowCapabilitySurfaceResolver {
    pub(crate) fn new(inner: Arc<dyn CapabilitySurfaceProfileResolver>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for GithubIssueWorkflowCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        if is_github_issue_workflow_context(run_context)
            && let Some(allow_set) = allowed_capabilities_for_workflow_subagent_profile_id(
                &run_context
                    .resolved_run_profile
                    .capability_surface_profile_id,
            )?
        {
            return Ok(allow_set);
        }
        if let Some(allow_set) = allowed_capabilities_for_stage_profile_id(
            &run_context
                .resolved_run_profile
                .capability_surface_profile_id,
        )? {
            return Ok(allow_set);
        }
        self.inner.resolve(run_context).await
    }
}

pub(crate) fn is_github_issue_workflow_context(run_context: &LoopRunContext) -> bool {
    run_context
        .product_context
        .as_ref()
        .and_then(|context| context.adapter.as_ref())
        .is_some_and(|adapter| adapter.as_ref() == WORKFLOW_ADAPTER_ID)
}

pub(crate) fn workflow_subagent_flavor_catalog() -> Vec<SpawnSubagentFlavorDescriptor> {
    [
        (
            "general",
            "read-only file exploration (read_file, list_dir, grep)",
        ),
        (
            "explorer",
            "read + glob over filesystem (read_file, list_dir, grep, glob)",
        ),
        (
            "planner",
            "read codebase + planning context, returns a structured implementation plan \
             (read_file, list_dir, grep, glob)",
        ),
    ]
    .into_iter()
    .map(|(id, summary)| SpawnSubagentFlavorDescriptor {
        id: SubagentKindId::new(id)
            .expect("static workflow subagent flavor id is a valid SubagentKindId"),
        summary: summary.to_string(),
    })
    .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn workflow_spawn_subagent_schema() -> serde_json::Value {
    build_spawn_subagent_parameters_schema(&workflow_subagent_flavor_catalog())
}

pub(crate) fn planned_run_profile_resolver_with_stage_profiles()
-> Result<InMemoryRunProfileResolver, RunProfileRegistryError> {
    let mut registry = InMemoryRunProfileRegistry::with_builtin_profiles();
    ironclaw_reborn::planned_driver_factory::register_default_planned_profile(&mut registry)?;
    ironclaw_reborn::planned_driver_factory::register_subagent_planned_profile(&mut registry)?;
    register_stage_run_profiles(&mut registry)?;
    let implicit_default = ironclaw_reborn::planned_driver_factory::planned_default_profile_id()
        .map_err(invalid_run_profile)?;
    Ok(InMemoryRunProfileResolver::new_with_implicit_default(
        registry,
        implicit_default,
    ))
}

fn register_stage_run_profiles(
    registry: &mut InMemoryRunProfileRegistry,
) -> Result<(), RunProfileRegistryError> {
    let descriptor = ironclaw_reborn::planned_driver_factory::planned_driver_descriptor()
        .map_err(invalid_run_profile)?;
    let checkpoint_schema_id =
        ironclaw_reborn::planned_driver_factory::planned_driver_checkpoint_schema_id()
            .map_err(invalid_run_profile)?;
    let checkpoint_schema_version =
        ironclaw_reborn::planned_driver_factory::planned_driver_checkpoint_schema_version();

    for profile in GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES {
        let profile_id =
            ironclaw_turns::RunProfileId::new(profile.profile_id).map_err(invalid_run_profile)?;
        let capability_surface_profile_id =
            CapabilitySurfaceProfileId::new(profile.profile_id).map_err(invalid_run_profile)?;
        registry.register(
            RunProfileDefinition::interactive_like(
                profile_id,
                descriptor.clone(),
                checkpoint_schema_id.clone(),
                checkpoint_schema_version,
                capability_surface_profile_id,
            )
            // Headless: the workflow poller drives these stage turns with no human
            // attached, so a budget-cap crossing must degrade to a recoverable
            // error rather than open an approval gate that strands the run.
            .with_non_interactive_budget(),
        )?;
    }
    Ok(())
}

fn invalid_run_profile(reason: String) -> RunProfileRegistryError {
    RunProfileRegistryError::InvalidProfile { reason }
}

#[derive(Default)]
pub(crate) struct GithubIssueWorkflowSubagentDefinitionResolver;

#[async_trait]
impl SubagentDefinitionResolver for GithubIssueWorkflowSubagentDefinitionResolver {
    async fn resolve_kind(
        &self,
        kind: &SubagentKindId,
    ) -> Result<Option<SubagentDefinition>, ironclaw_turns::run_profile::AgentLoopHostError> {
        if !matches!(kind.as_str(), "general" | "explorer" | "planner") {
            return Ok(None);
        }
        let requested_run_profile =
            RunProfileRequest::new(SUBAGENT_RUN_PROFILE_ID).map_err(|reason| {
                ironclaw_turns::run_profile::AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
                    reason,
                )
            })?;
        Ok(Some(SubagentDefinition {
            subagent_kind: kind.clone(),
            allow_nesting: false,
            requested_run_profile,
        }))
    }
}

#[cfg(test)]
mod stage_profile_budget_tests {
    use ironclaw_turns::run_profile::{BudgetApprovalMode, RunProfileResolutionRequest};
    use ironclaw_turns::{RunProfileRequest, RunProfileResolver};

    #[tokio::test]
    async fn github_workflow_stage_profiles_are_non_interactive_for_budget() {
        // The poller drives these stage turns with no human attached, so each
        // resolved profile must be NonInteractive — a budget-cap crossing then
        // degrades to a recoverable error instead of stranding the run on an
        // unanswerable approval gate.
        let resolver = super::planned_run_profile_resolver_with_stage_profiles()
            .expect("stage profile resolver");
        for profile in super::GITHUB_ISSUE_WORKFLOW_STAGE_PROFILES {
            let requested = RunProfileRequest::new(profile.profile_id).expect("profile id");
            let resolved = resolver
                .resolve_run_profile(
                    RunProfileResolutionRequest::interactive_default()
                        .with_requested_run_profile(requested),
                )
                .await
                .expect("resolve stage profile");
            assert_eq!(
                resolved.budget_approval_mode,
                BudgetApprovalMode::NonInteractive,
                "github stage profile `{}` must be non-interactive for budget approval",
                profile.profile_id
            );
        }
    }
}
