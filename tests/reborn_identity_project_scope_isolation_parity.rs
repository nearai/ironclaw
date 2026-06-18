#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostIdentityMessageContent, HostManagedModelMessageRole, HostManagedModelResponse,
    IdentityApplicability, IdentityFileName,
};
use ironclaw_turns::{
    LoopMessageRef, TurnStatus,
    run_profile::{LoopRunContext, PromptMode},
};
use reborn_support::harness::{
    RebornBinaryE2EHarness, RebornHarnessSharedStorage, RecordingTestCapabilityPort,
    test_product_scope,
};
use reborn_support::model_replay::RebornTraceReplayModelGateway;
use tokio::sync::RwLock;

const PROJECT_ALPHA_IDENTITY: &str = "Alice project alpha identity: carries amber notebook.";
const PROJECT_BETA_IDENTITY: &str = "Alice project beta identity: carries violet notebook.";

#[tokio::test]
async fn reborn_identity_project_scope_isolation_parity() {
    const ROOM: &str = "room-project-identity-shared";

    let shared_storage = RebornHarnessSharedStorage::new().expect("shared storage");
    let identity_source = Arc::new(ProjectIdentitySource::default());
    let project_alpha = test_product_scope(
        "tenant-project-identity-e2e",
        "host-user",
        "agent-e2e",
        Some("project-alpha-e2e"),
    );
    let project_beta = test_product_scope(
        "tenant-project-identity-e2e",
        "host-user",
        "agent-e2e",
        Some("project-beta-e2e"),
    );

    let mut alpha = RebornBinaryE2EHarness::with_model_gateway_scope_identity_source_trigger_installation_shared_storage_unscoped_worker(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([HostManagedModelResponse::assistant_reply(
            "project alpha identity reply",
        )]),
        RecordingTestCapabilityPort::echo(),
        project_alpha,
        identity_source.clone(),
        ironclaw_product_adapters::ProductTriggerReason::DirectChat,
        "reborn-test",
        "install-1",
        "alice",
        shared_storage.clone(),
    )
    .await
    .expect("project alpha harness");
    let mut beta = RebornBinaryE2EHarness::with_model_gateway_scope_identity_source_trigger_installation_shared_storage_unscoped_worker(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([HostManagedModelResponse::assistant_reply(
            "project beta identity reply",
        )]),
        RecordingTestCapabilityPort::echo(),
        project_beta,
        identity_source.clone(),
        ironclaw_product_adapters::ProductTriggerReason::DirectChat,
        "reborn-test",
        "install-1",
        "alice",
        shared_storage,
    )
    .await
    .expect("project beta harness");

    let alpha_turn = alpha
        .submit_text_for(ROOM, "alice", "event-project-alpha-identity", "alpha asks")
        .await
        .expect("submit project alpha turn");

    identity_source
        .set_identity(
            &ProjectIdentityKey::from_turn(&alpha_turn),
            PROJECT_ALPHA_IDENTITY,
        )
        .await;

    alpha.start();
    alpha
        .wait_for_submitted_status(&alpha_turn, TurnStatus::Completed)
        .await
        .expect("project alpha completed");
    alpha.shutdown().await;

    let beta_turn = beta
        .submit_text_for(ROOM, "alice", "event-project-beta-identity", "beta asks")
        .await
        .expect("submit project beta turn");
    identity_source
        .set_identity(
            &ProjectIdentityKey::from_turn(&beta_turn),
            PROJECT_BETA_IDENTITY,
        )
        .await;

    beta.start();
    beta.wait_for_submitted_status(&beta_turn, TurnStatus::Completed)
        .await
        .expect("project beta completed");

    assert_ne!(alpha_turn.scope.project_id, beta_turn.scope.project_id);

    let alpha_prompts: Vec<String> = alpha
        .model_requests()
        .iter()
        .map(system_prompt_text)
        .collect();
    let beta_prompts: Vec<String> = beta
        .model_requests()
        .iter()
        .map(system_prompt_text)
        .collect();

    assert_prompt_contains_only(
        &alpha_prompts,
        PROJECT_ALPHA_IDENTITY,
        PROJECT_BETA_IDENTITY,
        "project alpha",
    );
    assert_prompt_contains_only(
        &beta_prompts,
        PROJECT_BETA_IDENTITY,
        PROJECT_ALPHA_IDENTITY,
        "project beta",
    );

    let seen = identity_source.seen_keys().await;
    assert!(seen.contains(&ProjectIdentityKey::from_turn(&alpha_turn)));
    assert!(seen.contains(&ProjectIdentityKey::from_turn(&beta_turn)));

    alpha.assert_model_exhausted();
    beta.assert_model_exhausted();
    beta.shutdown().await;
}

fn system_prompt_text(request: &ironclaw_loop_support::HostManagedModelRequest) -> String {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_prompt_contains_only(prompts: &[String], expected: &str, forbidden: &str, label: &str) {
    assert!(
        !prompts.is_empty()
            && prompts
                .iter()
                .all(|prompt| prompt.contains(expected) && !prompt.contains(forbidden)),
        "{label} prompt should contain only matching project identity; prompts={prompts:#?}"
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProjectIdentityKey {
    tenant_id: String,
    user_id: String,
    project_id: Option<String>,
}

impl ProjectIdentityKey {
    fn from_turn(turn: &reborn_support::harness::SubmittedTurn) -> Self {
        Self {
            tenant_id: turn.scope.tenant_id.as_str().to_string(),
            user_id: turn.actor.user_id.as_str().to_string(),
            project_id: turn
                .scope
                .project_id
                .as_ref()
                .map(|id| id.as_str().to_string()),
        }
    }
}

#[derive(Default)]
struct ProjectIdentitySource {
    identities: RwLock<HashMap<ProjectIdentityKey, HostIdentityContextCandidate>>,
    seen: RwLock<Vec<ProjectIdentityKey>>,
}

impl ProjectIdentitySource {
    async fn set_identity(&self, key: &ProjectIdentityKey, content: &str) {
        let name = IdentityFileName::new("IDENTITY.md").expect("identity file name");
        let candidate = HostIdentityContextCandidate::new_installed_summary_only(
            name,
            content.to_string(),
            IdentityApplicability::Always,
        );
        self.identities.write().await.insert(key.clone(), candidate);
    }

    async fn seen_keys(&self) -> Vec<ProjectIdentityKey> {
        self.seen.read().await.clone()
    }
}

#[async_trait]
impl HostIdentityContextSource for ProjectIdentitySource {
    async fn load_identity_candidates(
        &self,
        run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let actor = run_context
            .actor()
            .ok_or(HostIdentityContextBuildError::SourceUnavailable)?;
        let key = ProjectIdentityKey {
            tenant_id: run_context.scope.tenant_id.as_str().to_string(),
            user_id: actor.user_id.as_str().to_string(),
            project_id: run_context
                .scope
                .project_id
                .as_ref()
                .map(|id| id.as_str().to_string()),
        };
        {
            let mut seen = self.seen.write().await;
            if !seen.contains(&key) {
                seen.push(key.clone());
            }
        }
        self.identities
            .read()
            .await
            .get(&key)
            .cloned()
            .map(|candidate| vec![candidate])
            .ok_or(HostIdentityContextBuildError::SourceUnavailable)
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        _message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        Ok(None)
    }
}
