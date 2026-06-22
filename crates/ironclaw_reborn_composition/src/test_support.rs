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
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
#[cfg(feature = "github-issue-workflow-beta")]
use ironclaw_loop_support::{SubagentDefinitionResolver, SubagentKindId};
#[cfg(feature = "github-issue-workflow-beta")]
use ironclaw_turns::{
    CapabilitySurfaceProfileId, RunProfileRequest, RunProfileResolutionRequest, RunProfileResolver,
};
use ironclaw_turns::{
    TurnRunId, TurnStatus,
    run_profile::{LoopCapabilityPort, LoopModelUsage},
};

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
    match allow_set {
        ironclaw_loop_support::CapabilityAllowSet::All => None,
        ironclaw_loop_support::CapabilityAllowSet::Allowlist(capabilities) => Some(
            capabilities
                .into_iter()
                .map(|capability| capability.as_str().to_string())
                .collect(),
        ),
        _ => None,
    }
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
