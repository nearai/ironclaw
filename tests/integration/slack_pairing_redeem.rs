//! W5-SLACK-PAIR scenarios 1+2: Slack personal-binding pairing-code redeem.
//! Does not follow build/submit_turn/assert (see CLAUDE.md) — drives real
//! service logic + a real durable store, no live turn.
//!
//! Uses `LibSqlRootFilesystem`, not `LocalFilesystem`: the consume path's
//! versioned-CAS write is `Unsupported` on `LocalFilesystem` (see local.rs).
//!
//! Proves the redeem happy path, durability across a fresh-connection
//! reopen, and rejection of an unknown/expired code — the one seam wiring
//! the pairing service + store together with a real backend end-to-end.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_filesystem::{LibSqlRootFilesystem, ScopedFilesystem};
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::{ExternalActorRef, ProductAdapterId};
use ironclaw_product_workflow::{ProductActorUserResolutionRequest, ProductActorUserResolver};
use ironclaw_reborn_composition::{
    SlackPairingActorResolver, SlackPersonalBindingPairingCode, SlackPersonalBindingPairingError,
    SlackPersonalBindingPairingNotification, SlackPersonalBindingPairingNotifier,
    SlackPersonalBindingPairingService, SlackPersonalBindingPrincipal,
    slack_serve::SlackUserId,
    test_support::{
        SlackHostStateTestParts, SlackPairingTestClock, slack_host_state_for_test,
        slack_host_state_for_test_with_pairing_ttl,
        slack_host_state_for_test_with_pairing_ttl_and_clock,
    },
};
use ironclaw_slack_v2_adapter::{SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID};

#[path = "support/slack_pairing.rs"]
mod slack_pairing_fixtures;
use slack_pairing_fixtures::{
    binding_service, host_ids, installation_id, tenant_id, tenant_shared_mount_view,
};

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

fn slack_user_id() -> SlackUserId {
    SlackUserId::new("U123")
}

/// Opens (or reopens) a real libSQL database at `db_path` and mounts a real
/// `FilesystemSlackHostState` over it. Each call builds an independent
/// `libsql::Database`, so calling it twice on one path proves on-disk
/// durability, not shared in-process state.
async fn open_slack_pairing_store(
    db_path: &std::path::Path,
    pairing_ttl: Option<Duration>,
) -> SlackHostStateTestParts {
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
    let (user, agent, project) = host_ids();
    match pairing_ttl {
        Some(ttl) => {
            slack_host_state_for_test_with_pairing_ttl(scoped, tenant, user, agent, project, ttl)
        }
        None => slack_host_state_for_test(scoped, tenant, user, agent, project),
    }
}

/// Same as [`open_slack_pairing_store`] but with a caller-controlled
/// pairing-code TTL and clock, for deterministic expiry tests — see
/// [`SlackPairingTestClock`] for why this replaces sleeping.
async fn open_slack_pairing_store_with_clock(
    db_path: &std::path::Path,
    pairing_ttl: Duration,
    clock: &SlackPairingTestClock,
) -> SlackHostStateTestParts {
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
    let (user, agent, project) = host_ids();
    slack_host_state_for_test_with_pairing_ttl_and_clock(
        scoped,
        tenant,
        user,
        agent,
        project,
        pairing_ttl,
        clock,
    )
}

fn pairing_service(store: &SlackHostStateTestParts) -> SlackPersonalBindingPairingService {
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
/// (`SlackPairingActorResolver`), exercising the reopened store through a
/// fully public API surface.
async fn resolved_user_id(store: &SlackHostStateTestParts) -> Option<UserId> {
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

    // Reopen: drop the live store/service, then open a genuinely fresh
    // `libsql::Database` connection to the same on-disk file — data not
    // serialized to disk cannot appear, proving real durability.
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
    let clock = SlackPairingTestClock::new();
    let store =
        open_slack_pairing_store_with_clock(&db_path, Duration::from_millis(1), &clock).await;
    let service = pairing_service(&store);

    let issued = service
        .issue_challenge(installation_id(), slack_user_id())
        .await
        .expect("issue_challenge must succeed");
    // Advance the injected clock past the TTL directly — deterministic,
    // unlike racing a virtual `sleep` under `tokio::time::pause` against the
    // real wall clock the store's TTL check reads (see
    // `SlackPairingTestClock`).
    clock.advance(Duration::from_millis(5));

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
