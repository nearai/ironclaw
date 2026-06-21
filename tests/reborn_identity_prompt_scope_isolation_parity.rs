#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

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
use reborn_support::harness::{RebornBinaryE2EHarness, RecordingTestCapabilityPort};
use reborn_support::model_replay::RebornTraceReplayModelGateway;

const ALICE_IDENTITY: &str = "Alice is a software engineer who lives in Seattle.";
const BOB_IDENTITY: &str = "Bob is a marine biologist who lives in Miami.";

#[tokio::test]
async fn reborn_identity_prompt_scope_isolation_parity() {
    const SHARED_ROOM: &str = "room-identity-shared";

    let identity_source = Arc::new(ScopedIdentitySource::default());
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        HostManagedModelResponse::assistant_reply("alice scoped reply"),
        HostManagedModelResponse::assistant_reply("bob scoped reply"),
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway_identity_source_shared(
        SHARED_ROOM,
        model_gateway,
        RecordingTestCapabilityPort::echo(),
        identity_source.clone(),
    )
    .await
    .expect("harness");

    let alice = harness
        .submit_text_for_with_trigger(
            SHARED_ROOM,
            "alice",
            "event-identity-alice",
            "hello from alice",
            ProductTriggerReason::BotMention,
        )
        .await
        .expect("submit alice turn");
    identity_source.set_identity(alice.actor.user_id.as_str(), ALICE_IDENTITY);

    harness.start();
    harness
        .wait_for_submitted_status(&alice, TurnStatus::Completed)
        .await
        .expect("alice completed");

    let bob = harness
        .submit_text_for_with_trigger(
            SHARED_ROOM,
            "bob",
            "event-identity-bob",
            "hello from bob",
            ProductTriggerReason::BotMention,
        )
        .await
        .expect("submit bob turn");
    assert_ne!(
        alice.actor.user_id, bob.actor.user_id,
        "distinct external actors must resolve to distinct canonical users before identity can isolate"
    );
    assert_eq!(
        alice.thread_id, bob.thread_id,
        "BotMention submissions in the same shared room should bind to one shared thread"
    );
    assert_eq!(
        alice.thread_scope, bob.thread_scope,
        "shared-thread identity isolation must be exercised inside one thread scope"
    );
    identity_source.set_identity(bob.actor.user_id.as_str(), BOB_IDENTITY);
    harness
        .wait_for_submitted_status(&bob, TurnStatus::Completed)
        .await
        .expect("bob completed");

    let seen_users = identity_source.seen_users();
    assert_eq!(
        seen_users.len(),
        2,
        "identity source should see two canonical users"
    );
    assert!(seen_users.contains(&alice.actor.user_id.as_str().to_string()));
    assert!(seen_users.contains(&bob.actor.user_id.as_str().to_string()));

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
    let prompts: Vec<String> = requests.iter().map(system_prompt_text).collect();
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains(ALICE_IDENTITY) && !prompt.contains(BOB_IDENTITY)),
        "one model request should contain only Alice identity; prompts={prompts:#?}"
    );
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains(BOB_IDENTITY) && !prompt.contains(ALICE_IDENTITY)),
        "one model request should contain only Bob identity; prompts={prompts:#?}"
    );

    harness.assert_model_exhausted();
    harness.shutdown().await;
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

#[derive(Default)]
struct ScopedIdentitySource {
    identities: Mutex<HashMap<String, IdentityEntry>>,
    seen_users: Mutex<Vec<String>>,
}

impl ScopedIdentitySource {
    fn set_identity(&self, user_id: &str, content: &str) {
        let name = IdentityFileName::new("IDENTITY.md").expect("identity file name");
        let candidate = HostIdentityContextCandidate::new_installed_summary_only(
            name.clone(),
            content.to_string(),
            IdentityApplicability::Always,
        );
        self.identities
            .lock()
            .expect("identity source lock")
            .insert(user_id.to_string(), IdentityEntry { candidate });
    }

    fn seen_users(&self) -> Vec<String> {
        self.seen_users
            .lock()
            .expect("identity source lock")
            .clone()
    }

    fn identity_for_user(&self, user_id: &str) -> Option<IdentityEntry> {
        self.identities
            .lock()
            .expect("identity source lock")
            .get(user_id)
            .cloned()
    }
}

#[derive(Clone)]
struct IdentityEntry {
    candidate: HostIdentityContextCandidate,
}

#[async_trait]
impl HostIdentityContextSource for ScopedIdentitySource {
    async fn load_identity_candidates(
        &self,
        run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let actor = run_context
            .actor()
            .ok_or(HostIdentityContextBuildError::SourceUnavailable)?;
        let user_id = actor.user_id.as_str().to_string();
        {
            let mut seen = self.seen_users.lock().expect("identity source lock");
            if !seen.contains(&user_id) {
                seen.push(user_id.clone());
            }
        }
        for _ in 0..50 {
            if let Some(entry) = self.identity_for_user(&user_id) {
                return Ok(vec![entry.candidate]);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Err(HostIdentityContextBuildError::SourceUnavailable)
    }

    async fn resolve_identity_message_content(
        &self,
        run_context: &LoopRunContext,
        message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        let actor = run_context
            .actor()
            .ok_or(HostIdentityContextBuildError::SourceUnavailable)?;
        let _ = (actor, message_ref);
        Ok(None)
    }
}
