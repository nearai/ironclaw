//! Reborn integration-test framework — W5-SLACK-PAIR scenarios 1+2: Slack
//! personal-binding pairing-code redeem.
//!
//! Deliberately does not follow the `build → submit_turn → assert` shape
//! every other `reborn_integration_*.rs` binary uses (see
//! `tests/integration/CLAUDE.md`): this scenario drives real service logic +
//! a real durable store, but never a live turn. Same spirit as
//! `oauth_connect.rs` (assertion-focused, no harness), and this binary class
//! is the one the `reborn-integration-coverage` job executes, so it is the
//! vehicle chosen to move Slack host-beta pairing off its
//! previously-invisible 0% (see the W5-SLACK-PAIR plan's
//! "coverage-instrumentation scope" finding).
//!
//! Wires the real `FilesystemSlackHostState` (via the test-support Enabler §1
//! accessor — the only production-crate addition this file depends on; no
//! behavior change) behind the real `SlackPersonalUserBindingService` /
//! `SlackPersonalBindingPairingService`, over a real on-disk
//! `LibSqlRootFilesystem` — the same durable-backend class production mounts
//! this store on (`slack_host_state.rs` module docs: "backed by the selected
//! durable root filesystem in libSQL/Postgres builds"; a plain
//! `LocalFilesystem` cannot back it — the consume path's versioned-CAS write
//! is `Unsupported` there). Proves the redeem happy path, that the resulting
//! binding survives a genuinely fresh database connection to the same
//! on-disk file (real durability, mirroring the harness's
//! `assert_reply_persists_after_reopen` recipe), and that an unknown or
//! expired code is rejected — the one seam nothing else in the codebase
//! proves end-to-end with a real store (per the plan's coverage inventory:
//! the pairing service and HTTP route are each already exhaustively covered
//! crate-tier with fakes, and the store is covered crate-tier in isolation,
//! but never all three wired together).

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_filesystem::{LibSqlRootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, UserId,
    VirtualPath,
};
use ironclaw_product_adapters::{AdapterInstallationId, ExternalActorRef, ProductAdapterId};
use ironclaw_product_workflow::{ProductActorUserResolutionRequest, ProductActorUserResolver};
use ironclaw_reborn_composition::{
    RebornUserIdentityBindingStore, RebornUserIdentityLookup, SlackPairingActorResolver,
    SlackPersonalBindingInstallation, SlackPersonalBindingPairingChallengeStore,
    SlackPersonalBindingPairingCode, SlackPersonalBindingPairingError,
    SlackPersonalBindingPairingNotification, SlackPersonalBindingPairingNotifier,
    SlackPersonalBindingPairingService, SlackPersonalBindingPrincipal,
    SlackPersonalUserBindingService,
    slack_serve::{SlackApiAppId, SlackInstallationSelector, SlackTeamId, SlackUserId},
    test_support::{slack_host_state_for_test, slack_host_state_for_test_with_pairing_ttl},
};
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};

/// A `SlackPersonalBindingPairingNotifier` that never fails — pairing-code
/// delivery (the real DM send) is not this scenario's subject.
#[derive(Default)]
struct NoopPairingNotifier;

#[async_trait]
impl SlackPersonalBindingPairingNotifier for NoopPairingNotifier {
    async fn send_pairing_challenge(
        &self,
        _notification: SlackPersonalBindingPairingNotification,
    ) -> Result<(), SlackPersonalBindingPairingError> {
        Ok(())
    }
}

/// The three trait facets of one real `FilesystemSlackHostState` a pairing
/// test needs — the concrete type is deliberately not nameable outside the
/// composition crate, so tests hold only these trait objects, exactly as
/// production wiring does at `slack_host_beta.rs`.
struct SlackPairingStore {
    challenges: Arc<dyn SlackPersonalBindingPairingChallengeStore>,
    bindings: Arc<dyn RebornUserIdentityBindingStore>,
    lookup: Arc<dyn RebornUserIdentityLookup>,
}

fn tenant_id() -> TenantId {
    TenantId::new("tenant-alpha").expect("valid tenant id")
}

fn installation_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install-alpha").expect("valid installation id")
}

fn slack_user_id() -> SlackUserId {
    SlackUserId::new("U123")
}

fn tenant_shared_mount_view() -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/tenant-shared").expect("valid mount alias"),
        VirtualPath::new("/tenants/tenant-alpha/shared").expect("valid virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("valid mount view")
}

/// Opens (or reopens) a real libSQL database at `db_path` and mounts a real
/// `FilesystemSlackHostState` over it. Mirrors the crate-tier `libsql_state()`
/// helper and the harness's `assert_reply_persists_after_reopen`
/// fresh-connection recipe: each call builds an independent
/// `libsql::Database`, so calling it twice on one path proves on-disk
/// durability, not shared in-process state. Migrations are idempotent.
async fn open_slack_pairing_store(
    db_path: &std::path::Path,
    pairing_ttl: Option<Duration>,
) -> SlackPairingStore {
    let db = Arc::new(
        libsql::Builder::new_local(db_path)
            .build()
            .await
            .expect("build libsql database"),
    );
    let root = Arc::new(LibSqlRootFilesystem::new(db));
    root.run_migrations().await.expect("run libsql migrations");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        root,
        tenant_shared_mount_view(),
    ));
    let tenant = tenant_id();
    let user = UserId::new("user:host").expect("valid user id");
    let agent = AgentId::new("agent:host").expect("valid agent id");
    let project = Some(ProjectId::new("project:host").expect("valid project id"));
    let store = match pairing_ttl {
        Some(ttl) => {
            slack_host_state_for_test_with_pairing_ttl(scoped, tenant, user, agent, project, ttl)
        }
        None => slack_host_state_for_test(scoped, tenant, user, agent, project),
    };
    SlackPairingStore {
        challenges: store.clone(),
        bindings: store.clone(),
        lookup: store,
    }
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

fn pairing_service(store: &SlackPairingStore) -> SlackPersonalBindingPairingService {
    SlackPersonalBindingPairingService::new(
        binding_service(store.bindings.clone()),
        store.challenges.clone(),
        Arc::new(NoopPairingNotifier),
    )
}

fn principal(user_id: UserId) -> SlackPersonalBindingPrincipal {
    SlackPersonalBindingPrincipal {
        tenant_id: tenant_id(),
        user_id,
    }
}

/// Resolves the bound user through the same public path scenario 3/4 uses
/// (`SlackPairingActorResolver`), rather than reaching for the crate-private
/// identity-provider string constant — this exercises the reopened store
/// through a fully public API surface.
async fn resolved_user_id(store: &SlackPairingStore) -> Option<UserId> {
    let resolver = SlackPairingActorResolver::new(store.lookup.clone(), pairing_service(store));
    let request = ProductActorUserResolutionRequest::new(
        ProductAdapterId::new(SLACK_V2_ADAPTER_ID).expect("valid adapter id"),
        installation_id(),
        ExternalActorRef::new(
            SLACK_USER_ACTOR_KIND,
            slack_user_id().as_str(),
            None::<&str>,
        )
        .expect("valid external actor ref"),
    );
    resolver
        .resolve_product_actor_user(request)
        .await
        .expect("resolver must not error")
}

#[tokio::test]
async fn slack_pairing_redeem_binds_and_persists_identity_across_reopen() {
    let root = tempfile::tempdir().expect("tempdir");
    let db_path = root.path().join("slack-host-state.db");
    let authenticated_user_id = UserId::new("user:alice").expect("valid user id");

    let store = open_slack_pairing_store(&db_path, None).await;
    let service = pairing_service(&store);
    let issued = service
        .issue_challenge(installation_id(), slack_user_id())
        .await
        .expect("issue_challenge must succeed");

    let binding = service
        .redeem_challenge(
            principal(authenticated_user_id.clone()),
            issued.code.clone(),
        )
        .await
        .expect("redeem_challenge must succeed");
    assert_eq!(
        binding.user_id, authenticated_user_id,
        "redeemed binding must bind the authenticated principal"
    );

    let second_redeem = service
        .redeem_challenge(principal(authenticated_user_id.clone()), issued.code)
        .await;
    assert!(
        matches!(
            second_redeem,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ),
        "a consumed code must not be redeemable a second time; got {second_redeem:?}"
    );

    // Reopen: drop the live store/service entirely, then open a genuinely
    // fresh `libsql::Database` connection to the same on-disk file — data not
    // serialized to disk cannot appear, proving real durability (the same
    // recipe as the harness's `assert_reply_persists_after_reopen`).
    drop(service);
    drop(store);
    let reopened = open_slack_pairing_store(&db_path, None).await;
    let resolved = resolved_user_id(&reopened).await;
    assert_eq!(
        resolved,
        Some(authenticated_user_id),
        "identity binding must resolve after a fresh-connection store reopen"
    );
}

#[tokio::test]
async fn slack_pairing_redeem_rejects_unknown_code() {
    let root = tempfile::tempdir().expect("tempdir");
    let db_path = root.path().join("slack-host-state.db");
    let store = open_slack_pairing_store(&db_path, None).await;
    let service = pairing_service(&store);

    let result = service
        .redeem_challenge(
            principal(UserId::new("user:bob").expect("valid user id")),
            SlackPersonalBindingPairingCode::new("NEVERISSUED1").expect("valid code shape"),
        )
        .await;

    assert!(
        matches!(
            result,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ),
        "an unknown pairing code must be rejected as not found; got {result:?}"
    );
}

#[tokio::test]
async fn slack_pairing_redeem_rejects_expired_code() {
    let root = tempfile::tempdir().expect("tempdir");
    let db_path = root.path().join("slack-host-state.db");
    let store = open_slack_pairing_store(&db_path, Some(Duration::from_millis(1))).await;
    let service = pairing_service(&store);

    let issued = service
        .issue_challenge(installation_id(), slack_user_id())
        .await
        .expect("issue_challenge must succeed");
    tokio::time::sleep(Duration::from_millis(25)).await;

    let result = service
        .redeem_challenge(
            principal(UserId::new("user:carol").expect("valid user id")),
            issued.code,
        )
        .await;

    assert!(
        matches!(
            result,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ),
        "an expired pairing code must be rejected as not found through the real service path \
         (the store's own TTL check is already crate-tier covered; this is the only place it is \
         proven through the wired service); got {result:?}"
    );
}
