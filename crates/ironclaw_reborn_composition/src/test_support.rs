//! Test-only helpers for driving budget E2E tests against
//! [`build_reborn_runtime`].
//!
//! Gated behind the `test-support` feature so production builds never pay
//! the cost of the mock gateway / introspection accessors. The shapes here
//! are deliberately small: a mock [`HostManagedModelGateway`] with
//! per-turn scripted responses (including token usage), plus its companion
//! cost-table helper. Tests inject these via
//! [`RebornRuntimeInput::with_model_gateway_override`] and
//! [`RebornRuntimeInput::with_model_cost_table_override`], which are
//! exposed under the same `test-support` feature.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::ThreadId;
#[cfg(feature = "github-issue-workflow-beta")]
use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, UserId};
#[cfg(feature = "github-issue-workflow-beta")]
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    SubagentDefinitionResolver, SubagentKindId,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
#[cfg(feature = "github-issue-workflow-beta")]
use ironclaw_turns::run_profile::LoopRunContext;
#[cfg(feature = "github-issue-workflow-beta")]
use ironclaw_turns::{
    CapabilitySurfaceProfileId, RunOriginAdapter, RunProfileRequest, RunProfileResolutionRequest,
    RunProfileResolver, TurnActor, TurnId, TurnOwner, TurnScope, TurnSurfaceType,
};
use ironclaw_turns::{
    TurnRunId, TurnStatus,
    run_profile::{LoopCapabilityPort, LoopModelUsage},
};
#[cfg(feature = "github-issue-workflow-beta")]
use serde_json::Value as JsonValue;

use crate::runtime::{AssistantReply, ConversationId};

/// Build the GitHub issue workflow stage-turn submitter for composition tests.
///
/// Production wiring constructs this adapter inside the composition graph once
/// the workflow runtime is enabled; this helper only lets integration tests
/// drive the same crate-private adapter over fake thread/turn services.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_stage_turn_submitter_for_test(
    thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    actor_user_id: ironclaw_host_api::UserId,
    default_agent_id: ironclaw_host_api::AgentId,
) -> Arc<dyn ironclaw_github_issue_workflow::StageTurnSubmitter> {
    Arc::new(
        crate::github_issue_workflow::IronClawStageTurnSubmitter::new(
            thread_service,
            turn_coordinator,
            actor_user_id,
            default_agent_id,
        ),
    )
}

#[cfg(feature = "github-issue-workflow-beta")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubIssueWorkflowCapabilityDispatchRequestForTest {
    pub capability_id: String,
    pub provider_account_ref: ironclaw_github_issue_workflow::GithubProviderAccountRef,
    pub input: JsonValue,
}

#[cfg(feature = "github-issue-workflow-beta")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubIssueWorkflowCapabilityDispatchErrorForTest {
    AuthRequired,
    ApprovalRequired,
    Backend { kind: String, message: String },
}

#[cfg(feature = "github-issue-workflow-beta")]
#[async_trait]
pub trait GithubIssueWorkflowCapabilityDispatcherForTest: Send + Sync {
    async fn dispatch(
        &self,
        request: GithubIssueWorkflowCapabilityDispatchRequestForTest,
    ) -> Result<JsonValue, GithubIssueWorkflowCapabilityDispatchErrorForTest>;
}

#[cfg(feature = "github-issue-workflow-beta")]
struct GithubIssueWorkflowCapabilityDispatcherTestAdapter<D> {
    inner: Arc<D>,
}

#[cfg(feature = "github-issue-workflow-beta")]
#[async_trait]
impl<D> crate::github_issue_workflow::GithubIssueWorkflowCapabilityDispatcher
    for GithubIssueWorkflowCapabilityDispatcherTestAdapter<D>
where
    D: GithubIssueWorkflowCapabilityDispatcherForTest,
{
    async fn dispatch(
        &self,
        request: crate::github_issue_workflow::GithubIssueWorkflowCapabilityDispatchRequest,
    ) -> Result<JsonValue, crate::github_issue_workflow::GithubIssueWorkflowCapabilityDispatchError>
    {
        let request = GithubIssueWorkflowCapabilityDispatchRequestForTest {
            capability_id: request.capability_id,
            provider_account_ref: request.provider_account_ref,
            input: request.input,
        };
        self.inner.dispatch(request).await.map_err(|error| match error {
            GithubIssueWorkflowCapabilityDispatchErrorForTest::AuthRequired => {
                crate::github_issue_workflow::GithubIssueWorkflowCapabilityDispatchError::AuthRequired
            }
            GithubIssueWorkflowCapabilityDispatchErrorForTest::ApprovalRequired => {
                crate::github_issue_workflow::GithubIssueWorkflowCapabilityDispatchError::ApprovalRequired
            }
            GithubIssueWorkflowCapabilityDispatchErrorForTest::Backend { kind, message } => {
                crate::github_issue_workflow::GithubIssueWorkflowCapabilityDispatchError::Backend {
                    kind,
                    message,
                }
            }
        })
    }
}

/// Build the GitHub issue workflow provider adapter for composition tests.
///
/// Production wiring binds this adapter to a host-runtime capability path; this
/// helper lets integration tests drive the same adapter over a fake dispatch seam.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_provider_port_for_test<D>(
    configured_provider_account_ref: ironclaw_github_issue_workflow::GithubProviderAccountRef,
    dispatcher: Arc<D>,
) -> Arc<dyn ironclaw_github_issue_workflow::GithubIssueWorkflowPort>
where
    D: GithubIssueWorkflowCapabilityDispatcherForTest + 'static,
{
    let dispatcher =
        Arc::new(GithubIssueWorkflowCapabilityDispatcherTestAdapter { inner: dispatcher });
    Arc::new(
        crate::github_issue_workflow::IronClawGithubIssueWorkflowPort::new(
            configured_provider_account_ref,
            dispatcher,
        ),
    )
}

/// Test-only projection of one GitHub issue workflow capability profile.
///
/// Mirrors the composition-owned stage profile contract without exposing raw
/// runtime or host-runtime handles to integration tests.
#[cfg(feature = "github-issue-workflow-beta")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GithubIssueWorkflowCapabilityProfileForTest {
    pub profile_id: &'static str,
    pub allowed_capabilities: &'static [&'static str],
}

#[cfg(feature = "github-issue-workflow-beta")]
impl From<crate::github_issue_workflow::GithubIssueWorkflowCapabilityProfile>
    for GithubIssueWorkflowCapabilityProfileForTest
{
    fn from(profile: crate::github_issue_workflow::GithubIssueWorkflowCapabilityProfile) -> Self {
        Self {
            profile_id: profile.profile_id,
            allowed_capabilities: profile.allowed_capabilities,
        }
    }
}

/// Return the composition-owned GitHub issue workflow stage capability profiles.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_capability_profiles_for_test()
-> Vec<GithubIssueWorkflowCapabilityProfileForTest> {
    crate::github_issue_workflow::stage_capability_profiles()
        .iter()
        .copied()
        .map(Into::into)
        .collect()
}

/// Return the non-workflow default profile projection used to assert the stage
/// result sink is workflow-only.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_default_capability_profile_for_test()
-> GithubIssueWorkflowCapabilityProfileForTest {
    crate::github_issue_workflow::non_workflow_default_capability_profile().into()
}

/// Resolve a workflow stage capability-surface profile id to its allowlist.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_allowed_capabilities_for_profile_for_test(
    profile_id: &str,
) -> Option<std::collections::BTreeSet<String>> {
    let profile_id = CapabilitySurfaceProfileId::new(profile_id).ok()?;
    let allow_set =
        crate::github_issue_workflow::allowed_capabilities_for_stage_profile_id(&profile_id)
            .ok()
            .flatten()?;
    capability_allow_set_to_strings(allow_set)
}

/// Render the workflow-restricted `builtin.spawn_subagent` parameters schema.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_spawn_subagent_schema_for_test() -> serde_json::Value {
    crate::github_issue_workflow::workflow_spawn_subagent_schema()
}

/// Resolve a workflow subagent flavor to its requested run profile.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_subagent_definition_profile_for_test(kind: &str) -> Option<String> {
    let kind = SubagentKindId::new(kind).ok()?;
    let resolver = crate::github_issue_workflow::GithubIssueWorkflowSubagentDefinitionResolver;
    futures::executor::block_on(resolver.resolve_kind(&kind))
        .ok()
        .flatten()
        .map(|definition| definition.requested_run_profile.as_str().to_string())
}

/// Resolve the workflow-inherited child subagent capability surface for a
/// concrete flavor. This mirrors the base resolver path that the production
/// subagent surface intersection consumes before flavor material is applied.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_subagent_allowed_capabilities_for_test(
    kind: &str,
) -> Option<std::collections::BTreeSet<String>> {
    let kind = SubagentKindId::new(kind).ok()?;
    let definition_resolver =
        crate::github_issue_workflow::GithubIssueWorkflowSubagentDefinitionResolver;
    let definition = futures::executor::block_on(definition_resolver.resolve_kind(&kind))
        .ok()
        .flatten()?;
    let run_profile_resolver =
        crate::github_issue_workflow::planned_run_profile_resolver_with_stage_profiles()
            .expect("workflow stage planned run-profile resolver builds");
    let resolved = futures::executor::block_on(
        run_profile_resolver.resolve_run_profile(
            RunProfileResolutionRequest::interactive_default()
                .with_requested_run_profile(definition.requested_run_profile),
        ),
    )
    .expect("workflow subagent profile resolves");
    let owner = UserId::new("workflow-subagent-owner").expect("owner user id");
    let scope = TurnScope::new_with_owner(
        TenantId::new("workflow-subagent-tenant").expect("tenant id"),
        Some(AgentId::new("workflow-subagent-agent").expect("agent id")),
        Some(ProjectId::new("workflow-subagent-project").expect("project id")),
        ThreadId::new("workflow-subagent-thread").expect("thread id"),
        Some(owner.clone()),
    );
    let product_context = ironclaw_product_context::resolve_inbound(
        ironclaw_product_context::InboundClassification::TrustedOther,
        RunOriginAdapter::new("github_issue_workflow").expect("workflow adapter"),
        Some(TurnSurfaceType::Direct),
        TurnOwner::Personal {
            user: owner.clone(),
        },
    );
    let context = LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
        .with_actor(TurnActor::new(owner))
        .with_product_context(product_context);
    let resolver = crate::github_issue_workflow::GithubIssueWorkflowCapabilitySurfaceResolver::new(
        Arc::new(AllowAllCapabilitySurfaceResolverForTest),
    );
    let allow_set = futures::executor::block_on(resolver.resolve(&context)).ok()?;
    capability_allow_set_to_strings(allow_set)
}

/// Return the workflow-enabled composition built-in package capability ids.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_builtin_package_capabilities_for_test()
-> std::collections::BTreeSet<String> {
    crate::factory::builtin_extension_registry(true)
        .expect("workflow-enabled built-in extension registry builds")
        .capabilities()
        .map(|capability| capability.id.as_str().to_string())
        .collect()
}

/// Return the workflow-disabled composition built-in package capability ids.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_disabled_builtin_package_capabilities_for_test()
-> std::collections::BTreeSet<String> {
    crate::factory::builtin_extension_registry(false)
        .expect("workflow-disabled built-in extension registry builds")
        .capabilities()
        .map(|capability| capability.id.as_str().to_string())
        .collect()
}

/// Return the default host-runtime built-in package capability ids.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_default_builtin_package_capabilities_for_test()
-> std::collections::BTreeSet<String> {
    ironclaw_host_runtime::builtin_first_party_package()
        .expect("default built-in package builds")
        .capabilities
        .into_iter()
        .map(|capability| capability.id.as_str().to_string())
        .collect()
}

/// Return the workflow-enabled composition first-party handler ids that are
/// relevant to the workflow result tool contract.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_first_party_handler_capabilities_for_test()
-> std::collections::BTreeSet<String> {
    let mut registry = ironclaw_host_runtime::builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))
    .expect("default built-in handlers build");
    crate::github_issue_workflow::insert_workflow_stage_result_handler(
        &mut registry,
        Arc::new(ironclaw_triggers::InMemoryTriggerRepository::default()),
        Arc::new(crate::github_issue_workflow::WorkflowStageResultSinkSlot::new()),
    )
    .expect("workflow stage result handler installs");
    workflow_result_handler_ids(&registry)
}

/// Return the default host-runtime first-party handler ids that are relevant to
/// the workflow result tool contract.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_default_first_party_handler_capabilities_for_test()
-> std::collections::BTreeSet<String> {
    let registry = ironclaw_host_runtime::builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))
    .expect("default built-in handlers build");
    workflow_result_handler_ids(&registry)
}

/// Resolve every workflow stage profile through the composition planned
/// run-profile registry and return the stage profile ids that round-tripped.
#[cfg(feature = "github-issue-workflow-beta")]
pub fn github_issue_workflow_resolved_stage_profile_ids_for_test()
-> std::collections::BTreeSet<&'static str> {
    let resolver = crate::github_issue_workflow::planned_run_profile_resolver_with_stage_profiles()
        .expect("workflow stage planned run-profile resolver builds");
    crate::github_issue_workflow::stage_capability_profiles()
        .iter()
        .filter_map(|profile| {
            let request = RunProfileRequest::new(profile.profile_id)
                .expect("static workflow stage profile id is valid");
            let resolved = futures::executor::block_on(
                resolver.resolve_run_profile(
                    RunProfileResolutionRequest::interactive_default()
                        .with_requested_run_profile(request),
                ),
            )
            .expect("workflow stage profile resolves");
            (resolved.profile_id.as_str() == profile.profile_id).then_some(profile.profile_id)
        })
        .collect()
}

#[cfg(feature = "github-issue-workflow-beta")]
fn capability_allow_set_to_strings(
    allow_set: CapabilityAllowSet,
) -> Option<std::collections::BTreeSet<String>> {
    match allow_set {
        CapabilityAllowSet::All => None,
        CapabilityAllowSet::Allowlist(capabilities) => Some(
            capabilities
                .into_iter()
                .map(|capability| capability.as_str().to_string())
                .collect(),
        ),
        _ => None,
    }
}

#[cfg(feature = "github-issue-workflow-beta")]
fn workflow_result_handler_ids(
    registry: &ironclaw_host_runtime::FirstPartyCapabilityRegistry,
) -> std::collections::BTreeSet<String> {
    let capability_id =
        CapabilityId::new(ironclaw_host_runtime::WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID)
            .expect("workflow result capability id");
    [capability_id]
        .into_iter()
        .filter(|capability_id| registry.contains_handler(capability_id))
        .map(|capability_id| capability_id.as_str().to_string())
        .collect()
}

#[cfg(feature = "github-issue-workflow-beta")]
struct AllowAllCapabilitySurfaceResolverForTest;

#[cfg(feature = "github-issue-workflow-beta")]
#[async_trait]
impl CapabilitySurfaceProfileResolver for AllowAllCapabilitySurfaceResolverForTest {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(CapabilityAllowSet::All)
    }
}

/// Build a terminal/no-text assistant reply for CLI and product-surface tests.
///
/// Kept behind `test-support` so downstream crates can exercise presentation
/// paths without depending directly on lower-level turn/thread crates.
pub fn assistant_reply_without_text_for_test(
    status: TurnStatus,
    failure_category: Option<&str>,
) -> AssistantReply {
    AssistantReply {
        conversation: ConversationId(
            ThreadId::new("test-assistant-reply").expect("static test thread id"), // safety: static test helper id is a valid thread id literal.
        ),
        run_id: TurnRunId::new(),
        status,
        failure_category: failure_category.map(str::to_owned),
        text: None,
    }
}

/// One scripted reply from the mock LLM.
///
/// `usage` is forwarded into [`HostManagedModelResponse::usage`] so the
/// budget accountant reconciles against real provider numbers, not the
/// reservation estimate.
#[derive(Debug, Clone)]
pub struct ScriptedReply {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl ScriptedReply {
    pub fn new(text: impl Into<String>, input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            text: text.into(),
            input_tokens,
            output_tokens,
        }
    }

    fn into_response(self) -> HostManagedModelResponse {
        HostManagedModelResponse::assistant_reply(self.text).with_usage(LoopModelUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }
}

/// Mock [`HostManagedModelGateway`] that returns scripted assistant
/// replies with configurable token usage.
///
/// Replies are consumed in FIFO order. When the script runs out the
/// gateway falls back to a sentinel reply with zero tokens — tests that
/// drive multiple turns should pre-load the matching number of
/// [`ScriptedReply`] entries.
///
/// Every `stream_model` call is recorded so tests can assert the call
/// count after the run completes.
#[derive(Debug, Default)]
pub struct BudgetTestGateway {
    replies: Mutex<Vec<ScriptedReply>>,
    fallback: Option<ScriptedReply>,
    calls: Mutex<Vec<HostManagedModelRequest>>,
}

impl BudgetTestGateway {
    pub fn new() -> Self {
        Self::default()
    }

    /// Single-reply convenience: every model call returns the same
    /// assistant text with the given token counts.
    pub fn with_constant(text: impl Into<String>, input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            replies: Mutex::new(Vec::new()),
            fallback: Some(ScriptedReply::new(text, input_tokens, output_tokens)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Push one scripted reply. Replies are consumed in FIFO order.
    pub fn push(&self, reply: ScriptedReply) {
        self.replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(reply);
    }

    /// Number of `stream_model` calls observed so far.
    pub fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    fn next_reply(&self) -> ScriptedReply {
        let mut script = self
            .replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if script.is_empty() {
            return self
                .fallback
                .clone()
                .unwrap_or_else(|| ScriptedReply::new("budget test fallback reply", 0, 0));
        }
        script.remove(0)
    }
}

#[async_trait]
impl HostManagedModelGateway for BudgetTestGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        Ok(self.next_reply().into_response())
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // The budget tests don't need capability dispatch — fall through
        // to the plain stream path. If a future test needs tool calls,
        // extend this with a separate scripted-tool-call queue.
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        Ok(self.next_reply().into_response())
    }
}

/// Mock gateway that always fails with the given error kind. Useful for
/// driving the cancellation / provider-error paths in budget tests
/// without depending on tokio cancel semantics.
#[derive(Debug)]
pub struct FailingTestGateway {
    pub kind: HostManagedModelErrorKind,
    pub summary: String,
}

impl FailingTestGateway {
    pub fn new(kind: HostManagedModelErrorKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
        }
    }
}

#[async_trait]
impl HostManagedModelGateway for FailingTestGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(self.kind, self.summary.clone()))
    }
}
