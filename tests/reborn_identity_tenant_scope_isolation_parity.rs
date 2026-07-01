#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::{collections::HashMap, sync::Arc};

use tokio::sync::{RwLock, watch};

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostIdentityMessageContent, HostManagedModelMessageRole, HostManagedModelResponse,
    IdentityApplicability, IdentityFileName,
};
use ironclaw_product_adapters::ProductTriggerReason;
use ironclaw_turns::{
    LoopMessageRef, TurnStatus,
    run_profile::{LoopRunContext, PromptMode},
};
use reborn_support::harness::{
    RebornBinaryE2EHarness, RebornHarnessSharedStorage, RecordingTestCapabilityPort,
    test_product_scope,
};
use reborn_support::model_replay::RebornTraceReplayModelGateway;

const TENANT_ALPHA_IDENTITY: &str = "Alice alpha tenant identity: likes rust ferris.";
const TENANT_BETA_IDENTITY: &str = "Alice beta tenant identity: likes neon orchids.";

#[tokio::test]
async fn reborn_identity_tenant_scope_isolation_parity() {
    const ROOM: &str = "room-tenant-identity-shared";

    let shared_storage = RebornHarnessSharedStorage::new().expect("shared storage");
    let identity_source = Arc::new(TenantIdentitySource::default());
    let tenant_alpha = test_product_scope(
        "tenant-alpha-identity-e2e",
        "host-user",
        "agent-e2e",
        Some("project-e2e"),
    );
    let tenant_beta = test_product_scope(
        "tenant-beta-identity-e2e",
        "host-user",
        "agent-e2e",
        Some("project-e2e"),
    );

    let mut alpha = RebornBinaryE2EHarness::with_model_gateway_scope_identity_source_trigger_installation_shared_storage(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([HostManagedModelResponse::assistant_reply(
            "tenant alpha identity reply",
        )]),
        RecordingTestCapabilityPort::echo(),
        tenant_alpha,
        identity_source.clone(),
        ProductTriggerReason::DirectChat,
        "reborn-test",
        "install-1",
        "alice",
        shared_storage.clone(),
    )
    .await
    .expect("tenant alpha harness");
    let mut beta = RebornBinaryE2EHarness::with_model_gateway_scope_identity_source_trigger_installation_shared_storage(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([HostManagedModelResponse::assistant_reply(
            "tenant beta identity reply",
        )]),
        RecordingTestCapabilityPort::echo(),
        tenant_beta,
        identity_source.clone(),
        ProductTriggerReason::DirectChat,
        "reborn-test",
        "install-1",
        "alice",
        shared_storage,
    )
    .await
    .expect("tenant beta harness");

    let alpha_turn = alpha
        .submit_text_for(ROOM, "alice", "event-tenant-alpha-identity", "alpha asks")
        .await
        .expect("submit tenant alpha turn");
    let beta_turn = beta
        .submit_text_for(ROOM, "alice", "event-tenant-beta-identity", "beta asks")
        .await
        .expect("submit tenant beta turn");

    identity_source
        .set_identity(
            alpha_turn.scope.tenant_id.as_str(),
            alpha_turn.actor.user_id.as_str(),
            TENANT_ALPHA_IDENTITY,
        )
        .await;
    identity_source
        .set_identity(
            beta_turn.scope.tenant_id.as_str(),
            beta_turn.actor.user_id.as_str(),
            TENANT_BETA_IDENTITY,
        )
        .await;

    alpha.start();
    beta.start();

    alpha
        .wait_for_submitted_status(&alpha_turn, TurnStatus::Completed)
        .await
        .expect("tenant alpha completed");
    beta.wait_for_submitted_status(&beta_turn, TurnStatus::Completed)
        .await
        .expect("tenant beta completed");

    assert_ne!(
        alpha_turn.scope.tenant_id, beta_turn.scope.tenant_id,
        "test must exercise distinct tenant scopes"
    );

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
        TENANT_ALPHA_IDENTITY,
        TENANT_BETA_IDENTITY,
        "tenant alpha",
    );
    assert_prompt_contains_only(
        &beta_prompts,
        TENANT_BETA_IDENTITY,
        TENANT_ALPHA_IDENTITY,
        "tenant beta",
    );

    let seen = identity_source.seen_keys().await;
    assert!(seen.contains(&TenantIdentityKey::from_turn(&alpha_turn)));
    assert!(seen.contains(&TenantIdentityKey::from_turn(&beta_turn)));

    alpha.assert_model_exhausted();
    beta.assert_model_exhausted();
    alpha.shutdown().await;
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
        "{label} prompt should contain only matching tenant identity; prompts={prompts:#?}"
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TenantIdentityKey {
    tenant_id: String,
    user_id: String,
}

impl TenantIdentityKey {
    fn from_turn(turn: &reborn_support::harness::SubmittedTurn) -> Self {
        Self {
            tenant_id: turn.scope.tenant_id.as_str().to_string(),
            user_id: turn.actor.user_id.as_str().to_string(),
        }
    }
}

struct TenantIdentitySource {
    identities: RwLock<HashMap<TenantIdentityKey, HostIdentityContextCandidate>>,
    seen: RwLock<Vec<TenantIdentityKey>>,
    identity_changed: watch::Sender<()>,
}

impl Default for TenantIdentitySource {
    fn default() -> Self {
        let (identity_changed, _) = watch::channel(());
        Self {
            identities: RwLock::new(HashMap::new()),
            seen: RwLock::new(Vec::new()),
            identity_changed,
        }
    }
}

impl TenantIdentitySource {
    async fn set_identity(&self, tenant_id: &str, user_id: &str, content: &str) {
        let name = IdentityFileName::new("IDENTITY.md").expect("identity file name");
        let candidate = HostIdentityContextCandidate::new_installed_summary_only(
            name,
            content.to_string(),
            IdentityApplicability::Always,
        );
        self.identities.write().await.insert(
            TenantIdentityKey {
                tenant_id: tenant_id.to_string(),
                user_id: user_id.to_string(),
            },
            candidate,
        );
        let _ = self.identity_changed.send(());
    }

    async fn seen_keys(&self) -> Vec<TenantIdentityKey> {
        self.seen.read().await.clone()
    }
}

#[async_trait]
impl HostIdentityContextSource for TenantIdentitySource {
    async fn load_identity_candidates(
        &self,
        run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let actor = run_context
            .actor()
            .ok_or(HostIdentityContextBuildError::SourceUnavailable)?;
        let key = TenantIdentityKey {
            tenant_id: run_context.scope.tenant_id.as_str().to_string(),
            user_id: actor.user_id.as_str().to_string(),
        };
        {
            let mut seen = self.seen.write().await;
            if !seen.contains(&key) {
                seen.push(key.clone());
            }
        }
        let mut changed = self.identity_changed.subscribe();
        loop {
            if let Some(candidate) = self.identities.read().await.get(&key).cloned() {
                return Ok(vec![candidate]);
            }
            changed
                .changed()
                .await
                .map_err(|_| HostIdentityContextBuildError::SourceUnavailable)?;
        }
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        _message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        Ok(None)
    }
}
