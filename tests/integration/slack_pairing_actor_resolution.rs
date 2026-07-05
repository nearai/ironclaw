//! Reborn integration-test framework — W5-SLACK-PAIR scenarios 3+4 (narrowed):
//! `SlackPairingActorResolver` actor resolution.
//!
//! Deliberately does not follow the `build → submit_turn → assert` shape
//! (see `tests/integration/CLAUDE.md`) — same rationale as
//! `slack_pairing_redeem.rs`, whose header comment this mirrors.
//!
//! Per the W5-SLACK-PAIR plan, driving a paired Slack actor's message all
//! the way through a live `submit_turn` is blocked on two independent gaps
//! (the harness's `ConversationBindingService` double has no
//! `ProductActorBindingPolicy::ResolveActor` concept, and the resolver-bearing
//! wiring lives only behind `ironclaw_reborn_composition::build_reborn_runtime`,
//! which the int-tier harness never calls). The substitute proven here is
//! fully reachable today and honestly scoped: it proves
//! `SlackPairingActorResolver::resolve_product_actor_user` resolves a paired
//! actor to the right `UserId`, and that an unpaired actor resolves to `None`
//! while auto-issuing a pairing challenge as a side effect (the "unpaired ⇒
//! rejected" scenario 4 folds into this arm, per the plan — the caller-side
//! `ProductWorkflowError::BindingRequired` mapping lives in a private
//! `ironclaw_product_workflow` free function and is out of scope here).
//!
//! Store backend: the real `FilesystemSlackHostState` over a CAS-capable
//! `InMemoryBackend` (a plain `LocalFilesystem` cannot back this store — the
//! redeem/consume path's versioned-CAS write is `Unsupported` there). The
//! sibling `slack_pairing_redeem.rs` binary owns the on-disk durability
//! proof over libSQL; re-proving durability here would be redundant, and
//! staying on `InMemoryBackend` keeps this binary free of the `libsql`
//! feature requirement.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, UserId,
    VirtualPath,
};
use ironclaw_product_adapters::{AdapterInstallationId, ExternalActorRef, ProductAdapterId};
use ironclaw_product_workflow::{ProductActorUserResolutionRequest, ProductActorUserResolver};
use ironclaw_reborn_composition::{
    RebornUserIdentityBindingStore, RebornUserIdentityLookup, SlackPairingActorResolver,
    SlackPersonalBindingInstallation, SlackPersonalBindingPairingChallengeStore,
    SlackPersonalBindingPairingError, SlackPersonalBindingPairingNotification,
    SlackPersonalBindingPairingNotifier, SlackPersonalBindingPairingService,
    SlackPersonalBindingPrincipal, SlackPersonalUserBindingService,
    slack_serve::{SlackApiAppId, SlackInstallationSelector, SlackTeamId, SlackUserId},
    test_support::slack_host_state_for_test,
};
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};

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

fn tenant_id() -> TenantId {
    TenantId::new("tenant-alpha").expect("valid tenant id")
}

fn installation_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install-alpha").expect("valid installation id")
}

fn tenant_shared_mount_view() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/tenant-shared").expect("valid mount alias"),
        VirtualPath::new("/tenants/tenant-alpha/shared").expect("valid virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("valid mount view")
}

#[allow(clippy::type_complexity)]
fn slack_pairing_store_for_test() -> (
    Arc<dyn SlackPersonalBindingPairingChallengeStore>,
    Arc<dyn RebornUserIdentityBindingStore>,
    Arc<dyn RebornUserIdentityLookup>,
) {
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::default()),
        tenant_shared_mount_view(),
    ));
    let store = slack_host_state_for_test(
        scoped,
        tenant_id(),
        UserId::new("user:host").expect("valid user id"),
        AgentId::new("agent:host").expect("valid agent id"),
        Some(ProjectId::new("project:host").expect("valid project id")),
    );
    (store.clone(), store.clone(), store)
}

fn binding_service(
    binding_store: Arc<dyn RebornUserIdentityBindingStore>,
) -> SlackPersonalUserBindingService {
    SlackPersonalUserBindingService::new(
        [SlackPersonalBindingInstallation {
            tenant_id: tenant_id(),
            installation_id: installation_id(),
            selector: SlackInstallationSelector::AppTeam {
                api_app_id: SlackApiAppId::new("A123"),
                team_id: SlackTeamId::new("T123"),
            },
        }],
        binding_store,
    )
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
    let (challenge_store, binding_store, lookup) = slack_pairing_store_for_test();
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
    let (challenge_store, binding_store, lookup) = slack_pairing_store_for_test();
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
    // for the unpaired actor (the "auto-issue on first unpaired message"
    // behavior) — read the code back via the notifier and confirm the real
    // store now has a pending challenge for it.
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
