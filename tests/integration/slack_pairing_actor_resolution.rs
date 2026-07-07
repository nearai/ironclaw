//! W5-SLACK-PAIR scenarios 3+4 (narrowed): `SlackPairingActorResolver` actor
//! resolution. Does not follow build/submit_turn/assert (see CLAUDE.md) —
//! same rationale as `slack_pairing_redeem.rs`.
//!
//! A live `submit_turn` path is blocked on two harness gaps (no
//! `ResolveActor` binding-policy double; resolver wiring lives only behind
//! `build_reborn_runtime`). Proves `resolve_product_actor_user` resolves a
//! paired actor and auto-issues a challenge for an unpaired one (folds in
//! scenario 4).
//!
//! Uses `InMemoryBackend`, not `LocalFilesystem`: the consume path's
//! versioned-CAS write is `Unsupported` on `LocalFilesystem`. Durability is
//! proven by the sibling `slack_pairing_redeem.rs` over libSQL instead.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::{ExternalActorRef, ProductAdapterId};
use ironclaw_product_workflow::{ProductActorUserResolutionRequest, ProductActorUserResolver};
use ironclaw_reborn_composition::{
    SlackPairingActorResolver, SlackPersonalBindingPairingError,
    SlackPersonalBindingPairingNotification, SlackPersonalBindingPairingNotifier,
    SlackPersonalBindingPairingService, SlackPersonalBindingPrincipal,
    slack_serve::SlackUserId,
    test_support::{SlackHostStateTestParts, slack_host_state_for_test},
};
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};

#[path = "support/slack_pairing.rs"]
mod slack_pairing_fixtures;
use slack_pairing_fixtures::{
    binding_service, host_ids, installation_id, tenant_id, tenant_shared_mount_view,
};

/// Captures every code a pairing challenge was minted for — lets a test read
/// back the auto-issued code the resolver's own fallback path mints, without
/// reaching into crate-private storage.
#[derive(Default)]
struct CapturingPairingNotifier {
    codes: std::sync::Mutex<Vec<ironclaw_reborn_composition::SlackPersonalBindingPairingCode>>,
}

impl CapturingPairingNotifier {
    fn last_code(&self) -> ironclaw_reborn_composition::SlackPersonalBindingPairingCode {
        self.codes
            .lock()
            .expect("notifier lock")
            .last()
            .cloned()
            .expect("a pairing challenge must have been issued")
    }
}

#[async_trait]
impl SlackPersonalBindingPairingNotifier for CapturingPairingNotifier {
    async fn send_pairing_challenge(
        &self,
        notification: SlackPersonalBindingPairingNotification,
    ) -> Result<(), SlackPersonalBindingPairingError> {
        self.codes
            .lock()
            .expect("notifier lock")
            .push(notification.code);
        Ok(())
    }
}

fn slack_pairing_store_for_test() -> SlackHostStateTestParts {
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::default()),
        tenant_shared_mount_view(),
    ));
    let (user, agent, project) = host_ids();
    slack_host_state_for_test(scoped, tenant_id(), user, agent, project)
}

fn actor_resolution_request(slack_user_id: &SlackUserId) -> ProductActorUserResolutionRequest {
    ProductActorUserResolutionRequest::new(
        ProductAdapterId::new(SLACK_V2_ADAPTER_ID).expect("valid adapter id"),
        installation_id(),
        ExternalActorRef::new(SLACK_USER_ACTOR_KIND, slack_user_id.as_str(), None::<&str>)
            .expect("valid external actor ref"),
    )
}

#[tokio::test]
async fn slack_pairing_actor_resolver_resolves_paired_actor() {
    let SlackHostStateTestParts {
        challenges: challenge_store,
        bindings: binding_store,
        lookup,
    } = slack_pairing_store_for_test();
    let slack_user_id = SlackUserId::new("U-PAIRED");
    let bound_user_id = UserId::new("user:paired").expect("valid user id");

    // Seed a real binding through the real redeem path (the same production
    // wiring scenario 1 exercises), rather than writing the store directly.
    let pairing = SlackPersonalBindingPairingService::new(
        binding_service(binding_store),
        challenge_store.clone(),
        Arc::new(CapturingPairingNotifier::default()),
    );
    let issued = pairing
        .issue_challenge(installation_id(), slack_user_id.clone())
        .await
        .expect("issue_challenge must succeed");
    pairing
        .redeem_challenge(
            SlackPersonalBindingPrincipal {
                tenant_id: tenant_id(),
                user_id: bound_user_id.clone(),
            },
            issued.code,
        )
        .await
        .expect("redeem_challenge must succeed");

    let resolver = SlackPairingActorResolver::new(lookup, pairing);
    let resolved = resolver
        .resolve_product_actor_user(actor_resolution_request(&slack_user_id))
        .await
        .expect("resolver must not error");

    assert_eq!(
        resolved,
        Some(bound_user_id),
        "a paired Slack actor must resolve to its bound user id"
    );
}

#[tokio::test]
async fn slack_pairing_actor_resolver_issues_challenge_for_unpaired_actor() {
    let SlackHostStateTestParts {
        challenges: challenge_store,
        bindings: binding_store,
        lookup,
    } = slack_pairing_store_for_test();
    let slack_user_id = SlackUserId::new("U-UNPAIRED");
    let notifier = Arc::new(CapturingPairingNotifier::default());
    let pairing = SlackPersonalBindingPairingService::new(
        binding_service(binding_store),
        challenge_store.clone(),
        notifier.clone(),
    );

    let resolver = SlackPairingActorResolver::new(lookup, pairing);
    let resolved = resolver
        .resolve_product_actor_user(actor_resolution_request(&slack_user_id))
        .await
        .expect("resolver must not error");

    assert_eq!(
        resolved, None,
        "an unpaired Slack actor must not resolve to a user id"
    );

    // Side effect: the resolver's fallback minted a fresh pairing challenge
    // for the unpaired actor — read it back via the notifier and confirm the
    // real store now has a pending challenge for it.
    let code = notifier.last_code();
    let challenge = challenge_store
        .get_challenge(&code)
        .await
        .expect("auto-issued challenge must be pending in the real store");
    assert_eq!(
        challenge.slack_user_id.as_str(),
        slack_user_id.as_str(),
        "auto-issued challenge must be scoped to the unpaired actor"
    );
}
