//! Behavioral matrix for [`FilesystemRebornIdentityStore`](super::FilesystemRebornIdentityStore).
//!
//! These drive the store through its public resolver surface against an
//! in-memory backend, plus a two-store/shared-backend stand-in for two runtime
//! processes whose per-key locks do not serialize each other across the
//! durable substrate.

use super::*;
use crate::{
    ExternalSubjectId, ProviderInstanceId, ProviderKind, RebornUserDirectory,
    RebornUserProfileUpdate, RebornUserRole, RebornUserStatus, SurfaceKind,
};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

fn store_on(root: Arc<InMemoryBackend>) -> FilesystemRebornIdentityStore<InMemoryBackend> {
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        root,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/tenant-shared").unwrap(),
            VirtualPath::new("/tenants/host/shared").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap(),
    ));
    FilesystemRebornIdentityStore::new(
        scoped,
        TenantId::new("tenant-host").unwrap(),
        UserId::new("user:host").unwrap(),
        AgentId::new("agent:host").unwrap(),
        Some(ProjectId::new("project:host").unwrap()),
    )
}

fn store() -> FilesystemRebornIdentityStore<InMemoryBackend> {
    store_on(Arc::new(InMemoryBackend::default()))
}

/// Two stores over ONE shared backend with independent in-memory lock maps —
/// the in-test stand-in for two runtime processes whose per-key locks do not
/// serialize each other across the durable substrate.
fn store_pair() -> (
    FilesystemRebornIdentityStore<InMemoryBackend>,
    FilesystemRebornIdentityStore<InMemoryBackend>,
) {
    let root = Arc::new(InMemoryBackend::default());
    (store_on(Arc::clone(&root)), store_on(root))
}

fn tenant(id: &str) -> TenantId {
    TenantId::new(id).expect("tenant")
}

fn oauth(
    tenant: &TenantId,
    provider: &str,
    sub: &str,
    email: Option<&str>,
    verified: bool,
) -> ResolveExternalIdentity {
    ResolveExternalIdentity {
        tenant_id: tenant.clone(),
        surface_kind: SurfaceKind::Oauth,
        provider_kind: ProviderKind::new(provider).expect("provider"),
        provider_instance_id: None,
        external_subject_id: ExternalSubjectId::new(sub).expect("subject"),
        email: email.map(str::to_string),
        email_verified: verified,
        display_name: None,
    }
}

fn channel_actor(
    tenant: &TenantId,
    provider: &str,
    instance: &str,
    actor: &str,
) -> ResolveExternalIdentity {
    ResolveExternalIdentity {
        tenant_id: tenant.clone(),
        surface_kind: SurfaceKind::ChannelActor,
        provider_kind: ProviderKind::new(provider).expect("provider"),
        provider_instance_id: Some(ProviderInstanceId::new(instance).expect("instance")),
        external_subject_id: ExternalSubjectId::new(actor).expect("actor"),
        email: None,
        email_verified: false,
        display_name: None,
    }
}

fn channel_key(tenant: &TenantId, provider: &str, actor: &str) -> ExternalIdentityKey {
    ExternalIdentityKey {
        tenant_id: tenant.clone(),
        surface_kind: SurfaceKind::ChannelActor,
        provider_kind: ProviderKind::new(provider).expect("provider"),
        provider_instance_id: None,
        external_subject_id: ExternalSubjectId::new(actor).expect("actor"),
    }
}

fn channel_key_with_instance(
    tenant: &TenantId,
    provider: &str,
    instance: &str,
    actor: &str,
) -> ExternalIdentityKey {
    ExternalIdentityKey {
        tenant_id: tenant.clone(),
        surface_kind: SurfaceKind::ChannelActor,
        provider_kind: ProviderKind::new(provider).expect("provider"),
        provider_instance_id: Some(ProviderInstanceId::new(instance).expect("instance")),
        external_subject_id: ExternalSubjectId::new(actor).expect("actor"),
    }
}

#[tokio::test]
async fn same_identity_is_stable_across_logins() {
    let store = store();
    let t = tenant("t");
    let first = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
        .await
        .expect("resolve");
    let second = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
        .await
        .expect("resolve");
    assert_eq!(first.as_str(), second.as_str());
}

#[tokio::test]
async fn distinct_identities_get_distinct_users() {
    let store = store();
    let t = tenant("t");
    let a = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
        .await
        .expect("resolve");
    let b = store
        .resolve_or_create(oauth(&t, "google", "g-2", Some("b@x.com"), true))
        .await
        .expect("resolve");
    assert_ne!(
        a.as_str(),
        b.as_str(),
        "different people are different users"
    );
}

#[tokio::test]
async fn verified_email_links_across_oauth_providers() {
    let store = store();
    let t = tenant("t");
    let via_google = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("same@x.com"), true))
        .await
        .expect("resolve");
    let via_github = store
        .resolve_or_create(oauth(&t, "github", "gh-9", Some("same@x.com"), true))
        .await
        .expect("resolve");
    assert_eq!(
        via_google.as_str(),
        via_github.as_str(),
        "a verified shared email links both provider identities to one user"
    );
}

#[tokio::test]
async fn verified_email_link_is_case_insensitive() {
    let store = store();
    let t = tenant("t");
    let via_google = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("Alice@Example.COM"), true))
        .await
        .expect("resolve");
    let via_github = store
        .resolve_or_create(oauth(&t, "github", "gh-9", Some("alice@example.com"), true))
        .await
        .expect("resolve");
    assert_eq!(
        via_google.as_str(),
        via_github.as_str(),
        "verified-email linking must be case-insensitive across providers"
    );
}

#[tokio::test]
async fn unverified_email_does_not_link() {
    let store = store();
    let t = tenant("t");
    let verified = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("same@x.com"), true))
        .await
        .expect("resolve");
    let unverified = store
        .resolve_or_create(oauth(&t, "github", "gh-9", Some("same@x.com"), false))
        .await
        .expect("resolve");
    assert_ne!(
        verified.as_str(),
        unverified.as_str(),
        "an unverified email must never link to a verified account"
    );
}

#[tokio::test]
async fn different_tenant_does_not_collide_on_same_subject() {
    let store = store();
    let (a, b) = (tenant("tenant-a"), tenant("tenant-b"));
    let in_a = store
        .resolve_or_create(oauth(&a, "google", "g-1", Some("u@x.com"), true))
        .await
        .expect("resolve");
    let in_b = store
        .resolve_or_create(oauth(&b, "google", "g-1", Some("u@x.com"), true))
        .await
        .expect("resolve");
    assert_ne!(
        in_a.as_str(),
        in_b.as_str(),
        "the same provider subject in two tenants must be two users"
    );
}

#[tokio::test]
async fn verified_email_link_is_tenant_scoped() {
    let store = store();
    let (a, b) = (tenant("tenant-a"), tenant("tenant-b"));
    let in_a = store
        .resolve_or_create(oauth(&a, "google", "g-1", Some("same@x.com"), true))
        .await
        .expect("resolve");
    let in_b = store
        .resolve_or_create(oauth(&b, "github", "gh-9", Some("same@x.com"), true))
        .await
        .expect("resolve");
    assert_ne!(
        in_a.as_str(),
        in_b.as_str(),
        "a shared verified email must not link accounts across tenants"
    );
}

#[tokio::test]
async fn different_provider_instance_does_not_collide() {
    // provider_instance_id is part of the identity key: the same actor id
    // under two adapter installations addresses two distinct paths, so a
    // binding made under one installation is invisible under the other.
    let store = store();
    let t = tenant("t");
    store
        .bind(
            channel_key_with_instance(&t, "telegram", "inst-1", "actor-7"),
            &UserId::new("reborn-user-1").unwrap(),
        )
        .await
        .expect("bind");
    let under_other_instance = store
        .lookup(channel_key_with_instance(
            &t, "telegram", "inst-2", "actor-7",
        ))
        .await
        .expect("lookup");
    assert!(
        under_other_instance.is_none(),
        "the same actor id under a different installation must not collide"
    );
}

#[tokio::test]
async fn resolve_or_create_rejects_channel_actor() {
    // Channel actors are never mint-capable; resolve_or_create must reject
    // them so an unbound actor fails closed and channel adapters stay on
    // lookup/bind (the resolver contract).
    let store = store();
    let t = tenant("t");
    let result = store
        .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-1"))
        .await;
    assert!(
        matches!(result, Err(RebornIdentityError::ChannelActorNotMintable)),
        "resolve_or_create must reject channel-actor identities, got {result:?}"
    );
}

#[tokio::test]
async fn concurrent_first_logins_for_one_email_resolve_to_one_user() {
    let store = Arc::new(store());
    let (a, b) = (store.clone(), store.clone());
    let (ra, rb) = tokio::join!(
        tokio::spawn(async move {
            let t = tenant("t");
            a.resolve_or_create(oauth(&t, "google", "g-1", Some("dup@x.com"), true))
                .await
        }),
        tokio::spawn(async move {
            let t = tenant("t");
            b.resolve_or_create(oauth(&t, "github", "gh-1", Some("dup@x.com"), true))
                .await
        }),
    );
    let user_a = ra.expect("join").expect("resolve");
    let user_b = rb.expect("join").expect("resolve");
    assert_eq!(
        user_a.as_str(),
        user_b.as_str(),
        "concurrent first-logins for one verified email must share a user"
    );
}

#[tokio::test]
async fn concurrent_first_logins_for_same_identity_resolve_to_one_user() {
    // Same exact key (tenant, surface, provider, instance, subject) raced
    // twice: the in-lock re-check must let the loser observe the winner's
    // record instead of minting a second user.
    let store = Arc::new(store());
    let (a, b) = (store.clone(), store.clone());
    let (ra, rb) = tokio::join!(
        tokio::spawn(async move {
            let t = tenant("t");
            a.resolve_or_create(oauth(&t, "google", "same-sub", Some("a@x.com"), true))
                .await
        }),
        tokio::spawn(async move {
            let t = tenant("t");
            b.resolve_or_create(oauth(&t, "google", "same-sub", Some("a@x.com"), true))
                .await
        }),
    );
    let user_a = ra.expect("join").expect("resolve");
    let user_b = rb.expect("join").expect("resolve");
    assert_eq!(
        user_a.as_str(),
        user_b.as_str(),
        "concurrent first-logins for the same identity key must share a user"
    );
}

#[tokio::test]
async fn divergent_verified_emails_for_one_identity_never_orphan_an_index() {
    // Regression for the split-principal race: two concurrent first-logins for
    // the SAME external identity that present DIFFERENT verified emails.
    // Serializing on the identity key (not the email) makes the second login
    // observe the first's record and return its user, so it never publishes a
    // verified-email index for a user that lost the identity CAS. An
    // email-scoped lock would let both run concurrently and leave the loser's
    // email pointing at an orphan user, which a later different-provider login
    // with that email would link to and permanently split the principal.
    let store = Arc::new(store());
    let (a, b) = (store.clone(), store.clone());
    let (ra, rb) = tokio::join!(
        tokio::spawn(async move {
            let t = tenant("t");
            a.resolve_or_create(oauth(&t, "google", "g-1", Some("first@x.com"), true))
                .await
        }),
        tokio::spawn(async move {
            let t = tenant("t");
            b.resolve_or_create(oauth(&t, "google", "g-1", Some("second@x.com"), true))
                .await
        }),
    );
    let user_a = ra.expect("join").expect("resolve");
    let user_b = rb.expect("join").expect("resolve");
    assert_eq!(
        user_a.as_str(),
        user_b.as_str(),
        "the same identity with divergent emails must converge on one user"
    );

    // No verified-email index may point at an orphan: any index that exists
    // for either email must resolve to the surviving user.
    for email in ["first@x.com", "second@x.com"] {
        let index = store
            .read_record::<StoredVerifiedEmailIndex>(&verified_email_path("t", email).unwrap())
            .await
            .expect("read index");
        if let Some(index) = index {
            assert_eq!(
                index.user_id,
                user_a.as_str(),
                "verified-email index for {email} points at an orphan, not the surviving user"
            );
        }
    }
}

#[tokio::test]
async fn cross_process_first_logins_for_one_email_resolve_to_one_user() {
    // Two processes (separate lock maps, shared substrate) race a first login
    // for the same verified email through different providers. The per-key
    // lock is process-local, so both may pass the index read and reach the
    // create path; the verified-email index CAS is the cross-process arbiter,
    // and the loser must adopt the winner's user rather than returning its own
    // freshly minted one (a permanent split). Repeated rounds widen the race
    // window this guards.
    for round in 0..16 {
        let (p1, p2) = store_pair();
        let (p1, p2) = (Arc::new(p1), Arc::new(p2));
        let email = format!("dup{round}@x.com");
        let (e1, e2) = (email.clone(), email);
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move {
                let t = tenant("t");
                p1.resolve_or_create(oauth(&t, "google", "g-1", Some(&e1), true))
                    .await
            }),
            tokio::spawn(async move {
                let t = tenant("t");
                p2.resolve_or_create(oauth(&t, "github", "gh-1", Some(&e2), true))
                    .await
            }),
        );
        let user_1 = r1.expect("join").expect("resolve");
        let user_2 = r2.expect("join").expect("resolve");
        assert_eq!(
            user_1.as_str(),
            user_2.as_str(),
            "round {round}: cross-process first-logins for one verified email must not split"
        );
    }
}

#[tokio::test]
async fn resolve_writes_verified_email_index_before_returning() {
    // The index is written before the identity record, so a verified resolve
    // always leaves a readable index — the invariant the fast path relies on
    // to never return an identity with a missing index.
    let store = store();
    let t = tenant("t");
    store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("Indexed@X.com"), true))
        .await
        .expect("resolve");
    let index = store
        .read_record::<StoredVerifiedEmailIndex>(
            &verified_email_path("t", "indexed@x.com").unwrap(),
        )
        .await
        .expect("read index");
    assert!(
        index.is_some(),
        "a verified resolve must persist the canonical verified-email index"
    );
}

#[tokio::test]
async fn adopt_migrated_identity_preserves_user_and_links_verified_email() {
    let store = store();
    let t = tenant("t");
    // A legacy verified Google identity migrated with its original user id.
    store
        .adopt_migrated_identity(
            oauth(&t, "google", "g-legacy", Some("Legacy@X.com"), true),
            &UserId::new("legacy-user").unwrap(),
        )
        .await
        .expect("adopt");

    // Returning through the SAME legacy identity keeps the original id.
    let returning = store
        .resolve_or_create(oauth(&t, "google", "g-legacy", Some("legacy@x.com"), true))
        .await
        .expect("resolve");
    assert_eq!(returning.as_str(), "legacy-user");

    // A LATER login through a different provider with the same verified email
    // links to the migrated user via the seeded canonical index.
    let via_github = store
        .resolve_or_create(oauth(&t, "github", "gh-9", Some("legacy@x.com"), true))
        .await
        .expect("resolve");
    assert_eq!(
        via_github.as_str(),
        "legacy-user",
        "a migrated verified email must link a later different-provider login"
    );
}

#[tokio::test]
async fn adopt_migrated_identity_does_not_clobber_a_live_record() {
    let store = store();
    let t = tenant("t");
    // A user resolved live first, minting their canonical record.
    let live = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("live@x.com"), true))
        .await
        .expect("resolve");
    // A one-time fold then runs for the same key with a stale legacy id; the
    // live canonical record must win.
    store
        .adopt_migrated_identity(
            oauth(&t, "google", "g-1", Some("live@x.com"), true),
            &UserId::new("stale-legacy-user").unwrap(),
        )
        .await
        .expect("adopt");
    let again = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("live@x.com"), true))
        .await
        .expect("resolve");
    assert_eq!(
        again.as_str(),
        live.as_str(),
        "migration must not clobber a record a returning user already created"
    );
}

#[tokio::test]
async fn lookup_unbound_actor_returns_none() {
    let store = store();
    let resolved = store
        .lookup(channel_key(&tenant("t"), "slack", "U-unbound"))
        .await
        .expect("lookup");
    assert!(resolved.is_none(), "an unbound actor must fail closed");
}

#[tokio::test]
async fn bind_then_lookup_returns_bound_user() {
    let store = store();
    let t = tenant("t");
    let user = UserId::new("reborn-user-7").expect("user");
    store
        .bind(channel_key(&t, "slack", "U-1"), &user)
        .await
        .expect("bind");
    let resolved = store
        .lookup(channel_key(&t, "slack", "U-1"))
        .await
        .expect("lookup");
    assert_eq!(resolved.as_ref().map(UserId::as_str), Some("reborn-user-7"));
}

#[tokio::test]
async fn rebind_repoints_to_new_user() {
    let store = store();
    let t = tenant("t");
    store
        .bind(
            channel_key(&t, "slack", "U-1"),
            &UserId::new("user-a").unwrap(),
        )
        .await
        .expect("first bind");
    store
        .bind(
            channel_key(&t, "slack", "U-1"),
            &UserId::new("user-b").unwrap(),
        )
        .await
        .expect("rebind");
    let resolved = store
        .lookup(channel_key(&t, "slack", "U-1"))
        .await
        .expect("lookup");
    assert_eq!(
        resolved.as_ref().map(UserId::as_str),
        Some("user-b"),
        "re-binding the same key re-points it"
    );
}

#[tokio::test]
async fn bind_is_scoped_per_tenant() {
    let store = store();
    let user = UserId::new("user-a").expect("user");
    store
        .bind(channel_key(&tenant("tenant-a"), "slack", "U-1"), &user)
        .await
        .expect("bind");
    let other = store
        .lookup(channel_key(&tenant("tenant-b"), "slack", "U-1"))
        .await
        .expect("lookup");
    assert!(
        other.is_none(),
        "a binding in one tenant is invisible in another"
    );
}

#[tokio::test]
async fn concurrent_rebind_converges_and_a_later_bind_repoints() {
    // bind() reads the current version then writes with CAS::Version, falling
    // through to a CAS::Any overwrite on VersionMismatch to honor re-point
    // semantics under a lost race. Two processes (shared backend, independent
    // lock maps, so the per-key lock does not serialize them) rebind the SAME
    // channel key concurrently across several rounds to drive that overwrite
    // branch: every bind must succeed (never surface VersionMismatch) and
    // lookup must resolve to one of the two writers. A final explicit bind
    // then re-points deterministically and must be observed.
    let t = tenant("t");
    for round in 0..16 {
        let (p1, p2) = store_pair();
        let (p1, p2) = (Arc::new(p1), Arc::new(p2));
        let observer = Arc::clone(&p1);
        let (a, b) = (Arc::clone(&p1), Arc::clone(&p2));
        let (ka, kb) = (
            channel_key(&t, "slack", "U-1"),
            channel_key(&t, "slack", "U-1"),
        );
        let (ra, rb) = tokio::join!(
            tokio::spawn(async move { a.bind(ka, &UserId::new("user-a").unwrap()).await }),
            tokio::spawn(async move { b.bind(kb, &UserId::new("user-b").unwrap()).await }),
        );
        ra.expect("join")
            .unwrap_or_else(|err| panic!("round {round}: first concurrent bind errored: {err}"));
        rb.expect("join")
            .unwrap_or_else(|err| panic!("round {round}: second concurrent bind errored: {err}"));

        let raced = observer
            .lookup(channel_key(&t, "slack", "U-1"))
            .await
            .expect("lookup after race")
            .expect("a concurrent bind must leave the key bound");
        assert!(
            matches!(raced.as_str(), "user-a" | "user-b"),
            "round {round}: concurrent rebind must converge on a writer, got {}",
            raced.as_str()
        );

        observer
            .bind(
                channel_key(&t, "slack", "U-1"),
                &UserId::new("user-final").unwrap(),
            )
            .await
            .expect("final rebind");
        let resolved = observer
            .lookup(channel_key(&t, "slack", "U-1"))
            .await
            .expect("lookup after final rebind");
        assert_eq!(
            resolved.as_ref().map(UserId::as_str),
            Some("user-final"),
            "round {round}: a later explicit bind must re-point the key"
        );
    }
}

#[tokio::test]
async fn empty_verified_email_does_not_index_or_link() {
    // A verified but EMPTY email must not create a verified-email index — that
    // would key on the `segment("") == "_"` sentinel and wrongly collapse
    // unrelated future logins onto it. Two distinct identities presenting an
    // empty verified email stay distinct, and no index record exists for it.
    let store = store();
    let t = tenant("t");
    let a = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some(""), true))
        .await
        .expect("resolve");
    let b = store
        .resolve_or_create(oauth(&t, "github", "gh-1", Some(""), true))
        .await
        .expect("resolve");
    assert_ne!(
        a.as_str(),
        b.as_str(),
        "an empty verified email must not link two distinct identities"
    );
    let index = store
        .read_record::<StoredVerifiedEmailIndex>(&verified_email_path("t", "").unwrap())
        .await
        .expect("read index");
    assert!(
        index.is_none(),
        "an empty verified email must not create a verified-email index"
    );
}

#[tokio::test]
async fn resolve_or_create_keys_on_provider_instance() {
    // The resolve path normalizes a `None` instance to "" and keys on
    // provider_instance_id exactly like bind/lookup: the same subject with no
    // instance, with `inst-1`, and with `inst-2` must be three distinct users.
    // Emails are omitted so verified-email linking cannot mask the instance axis.
    let store = store();
    let t = tenant("t");
    let base = || ResolveExternalIdentity {
        tenant_id: t.clone(),
        surface_kind: SurfaceKind::Oauth,
        provider_kind: ProviderKind::new("google").expect("provider"),
        provider_instance_id: None,
        external_subject_id: ExternalSubjectId::new("sub-1").expect("subject"),
        email: None,
        email_verified: false,
        display_name: None,
    };
    let no_instance = store.resolve_or_create(base()).await.expect("resolve none");
    let inst_1 = store
        .resolve_or_create(ResolveExternalIdentity {
            provider_instance_id: Some(ProviderInstanceId::new("inst-1").expect("instance")),
            ..base()
        })
        .await
        .expect("resolve inst-1");
    let inst_2 = store
        .resolve_or_create(ResolveExternalIdentity {
            provider_instance_id: Some(ProviderInstanceId::new("inst-2").expect("instance")),
            ..base()
        })
        .await
        .expect("resolve inst-2");
    assert_ne!(no_instance.as_str(), inst_1.as_str());
    assert_ne!(inst_1.as_str(), inst_2.as_str());
    assert_ne!(
        no_instance.as_str(),
        inst_2.as_str(),
        "the provider instance is part of the resolve identity key"
    );
}

#[tokio::test]
async fn corrupt_persisted_user_id_surfaces_invalid_user_id() {
    // A persisted identity record whose `user_id` fails `UserId` validation on
    // read-back must surface `InvalidUserId` (backend inconsistency), never be
    // silently dropped. Drives both the `lookup` fast path and `resolve_or_create`.
    let store = store();
    let t = tenant("t");
    let path =
        identity_path(t.as_str(), SurfaceKind::Oauth, "google", "", "g-corrupt").expect("path");
    store
        .write_record(
            &path,
            &StoredExternalIdentity {
                user_id: String::new(), // empty — rejected by UserId::new on read-back
                email: None,
                email_verified: false,
                created_at: "2026-01-01T00:00:00Z".to_string(),
            },
            CasExpectation::Absent,
        )
        .await
        .expect("seed corrupt record");

    let via_lookup = store
        .lookup(ExternalIdentityKey {
            tenant_id: t.clone(),
            surface_kind: SurfaceKind::Oauth,
            provider_kind: ProviderKind::new("google").expect("provider"),
            provider_instance_id: None,
            external_subject_id: ExternalSubjectId::new("g-corrupt").expect("subject"),
        })
        .await;
    assert!(
        matches!(via_lookup, Err(RebornIdentityError::InvalidUserId(_))),
        "lookup of a corrupt persisted user id must surface InvalidUserId, got {via_lookup:?}"
    );

    let via_resolve = store
        .resolve_or_create(oauth(&t, "google", "g-corrupt", Some("a@x.com"), true))
        .await;
    assert!(
        matches!(via_resolve, Err(RebornIdentityError::InvalidUserId(_))),
        "resolve over a corrupt persisted user id must surface InvalidUserId, got {via_resolve:?}"
    );
}

#[tokio::test]
async fn corrupt_json_body_surfaces_backend_error() {
    // A stored body that is not valid JSON for the record type must surface
    // `Backend` (deserialize failure), not panic and not be swallowed.
    let store = store();
    let t = tenant("t");
    let path =
        identity_path(t.as_str(), SurfaceKind::Oauth, "google", "", "g-badjson").expect("path");
    store
        .filesystem
        .put(
            &store.scope,
            &path,
            Entry::bytes(b"{ this is not json".to_vec()).with_content_type(ContentType::json()),
            CasExpectation::Absent,
        )
        .await
        .expect("seed raw bytes");

    let result = store
        .lookup(ExternalIdentityKey {
            tenant_id: t.clone(),
            surface_kind: SurfaceKind::Oauth,
            provider_kind: ProviderKind::new("google").expect("provider"),
            provider_instance_id: None,
            external_subject_id: ExternalSubjectId::new("g-badjson").expect("subject"),
        })
        .await;
    assert!(
        matches!(result, Err(RebornIdentityError::Backend(_))),
        "a corrupt JSON body must surface a Backend error, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// RebornUserDirectory (admin surface)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn legacy_stored_user_json_deserializes_with_defaults() {
    // A record written before the admin fields existed carries only the
    // original four fields. It must load as an Active Member with no tenant —
    // the serde defaults — not fail to deserialize.
    let store = store();
    let legacy = br#"{"email":"a@x.com","display_name":"Ada","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
    store
        .filesystem
        .put(
            &store.scope,
            &user_path("legacy-user").unwrap(),
            Entry::bytes(legacy.to_vec()).with_content_type(ContentType::json()),
            CasExpectation::Absent,
        )
        .await
        .expect("seed legacy record");

    let user = store
        .get_user(&UserId::new("legacy-user").unwrap())
        .await
        .expect("get")
        .expect("present");
    assert_eq!(user.status, RebornUserStatus::Active);
    assert_eq!(user.role, RebornUserRole::Member);
    assert_eq!(user.email.as_deref(), Some("a@x.com"));
    assert!(user.tenant_id.is_none(), "legacy record has no tenant");
    // A legacy record (no tenant) is enumerated for the single configured
    // tenant.
    let listed = store
        .list_users(&tenant("t"), None, None, 10_000)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].user_id.as_str(), "legacy-user");
}

#[tokio::test]
async fn create_then_list_and_get_roundtrip() {
    let store = store();
    let t = tenant("acme");
    let admin = UserId::new("root-admin").unwrap();
    let created = store
        .create_user(
            &t,
            Some("new@acme.com".to_string()),
            Some("New User".to_string()),
            RebornUserRole::Member,
            &admin,
        )
        .await
        .expect("create");
    assert_eq!(created.status, RebornUserStatus::Active);
    assert_eq!(created.role, RebornUserRole::Member);
    assert_eq!(
        created.created_by.as_ref().map(UserId::as_str),
        Some("root-admin")
    );
    assert_eq!(
        created.tenant_id.as_ref().map(TenantId::as_str),
        Some("acme")
    );

    let fetched = store
        .get_user(&created.user_id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(fetched, created);

    let listed = store
        .list_users(&t, None, None, 10_000)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].user_id, created.user_id);

    // A different tenant does not see this user.
    let other = store
        .list_users(&tenant("other"), None, None, 10_000)
        .await
        .expect("list other");
    assert!(other.is_empty(), "users are tenant-scoped in enumeration");
}

#[tokio::test]
async fn list_users_paginates_by_user_id_cursor() {
    // Regression: the admin listing must return bounded pages and page through
    // the whole tenant via the cursor without gaps or duplicates, instead of
    // scanning and allocating every user in one unbounded response.
    let store = store();
    let t = tenant("acme");
    let by = UserId::new("bootstrap").unwrap();
    for _ in 0..5 {
        store
            .create_user(&t, None, None, RebornUserRole::Member, &by)
            .await
            .expect("create");
    }

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<UserId> = None;
    loop {
        let page = store
            .list_users(&t, None, cursor.as_ref(), 2)
            .await
            .expect("page");
        assert!(
            page.len() <= 2,
            "a page must never exceed the requested limit"
        );
        for window in page.windows(2) {
            assert!(
                window[0].user_id.as_str() < window[1].user_id.as_str(),
                "records within a page are user_id ascending"
            );
        }
        for user in &page {
            let id = user.user_id.as_str().to_string();
            assert!(!seen.contains(&id), "no user appears on two pages");
            seen.push(id);
        }
        if page.len() < 2 {
            break;
        }
        cursor = Some(page.last().expect("non-empty page").user_id.clone());
    }

    assert_eq!(seen.len(), 5, "pagination visits every user exactly once");
    let mut globally_sorted = seen.clone();
    globally_sorted.sort();
    assert_eq!(
        seen, globally_sorted,
        "the concatenation of pages is globally user_id ascending"
    );
}

#[tokio::test]
async fn update_status_role_and_profile() {
    let store = store();
    let t = tenant("acme");
    let admin = UserId::new("root-admin").unwrap();
    let user = store
        .create_user(
            &t,
            None,
            Some("Before".to_string()),
            RebornUserRole::Member,
            &admin,
        )
        .await
        .expect("create");

    let suspended = store
        .update_status(&user.user_id, RebornUserStatus::Suspended)
        .await
        .expect("suspend");
    assert_eq!(suspended.status, RebornUserStatus::Suspended);
    assert!(
        suspended.updated_at >= user.updated_at,
        "a mutation bumps updated_at"
    );

    let promoted = store
        .update_role(&user.user_id, RebornUserRole::Admin)
        .await
        .expect("promote");
    assert_eq!(promoted.role, RebornUserRole::Admin);

    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("team".to_string(), "platform".to_string());
    let profiled = store
        .update_profile(
            &user.user_id,
            RebornUserProfileUpdate {
                display_name: Some("After".to_string()),
                metadata: Some(metadata.clone()),
            },
        )
        .await
        .expect("profile");
    assert_eq!(profiled.display_name.as_deref(), Some("After"));
    assert_eq!(profiled.metadata, metadata);
    // Status and role set earlier survive an unrelated profile PATCH.
    assert_eq!(profiled.status, RebornUserStatus::Suspended);
    assert_eq!(profiled.role, RebornUserRole::Admin);
}

#[tokio::test]
async fn update_missing_user_surfaces_user_not_found() {
    let store = store();
    let result = store
        .update_status(&UserId::new("ghost").unwrap(), RebornUserStatus::Suspended)
        .await;
    assert!(
        matches!(result, Err(RebornIdentityError::UserNotFound(_))),
        "updating an absent user must surface UserNotFound, got {result:?}"
    );
}

#[tokio::test]
async fn count_active_admins_tracks_role_and_status() {
    let store = store();
    let t = tenant("acme");
    let by = UserId::new("bootstrap").unwrap();
    let admin_a = store
        .create_user(&t, None, None, RebornUserRole::Admin, &by)
        .await
        .expect("admin a");
    store
        .create_user(&t, None, None, RebornUserRole::Owner, &by)
        .await
        .expect("owner");
    store
        .create_user(&t, None, None, RebornUserRole::Member, &by)
        .await
        .expect("member");
    assert_eq!(
        store.count_active_admins(&t).await.expect("count"),
        2,
        "an owner and an admin both count as admins; a member does not"
    );

    // Suspending an admin drops the active-admin count.
    store
        .update_status(&admin_a.user_id, RebornUserStatus::Suspended)
        .await
        .expect("suspend");
    assert_eq!(
        store.count_active_admins(&t).await.expect("count"),
        1,
        "a suspended admin is not an active admin"
    );
}

#[tokio::test]
async fn resolve_or_create_refuses_a_suspended_account_a_fresh_session() {
    // Regression: suspending a user must lock them out of the product, not only
    // the admin routes. A fresh SSO login (resolve_or_create) for a suspended
    // account fails closed so the host adapter can map it to a 403 instead of
    // minting a new session.
    let store = store();
    let t = tenant("acme");
    let email = Some("alice@example.com");

    let user = store
        .resolve_or_create(oauth(&t, "google", "sub-1", email, true))
        .await
        .expect("first login mints an active user");

    store
        .update_status(&user, RebornUserStatus::Suspended)
        .await
        .expect("an admin suspends the account");

    let err = store
        .resolve_or_create(oauth(&t, "google", "sub-1", email, true))
        .await
        .expect_err("a suspended account must not resolve a fresh session");
    assert!(
        matches!(err, RebornIdentityError::UserSuspended(_)),
        "suspended re-login must fail closed with UserSuspended, got {err:?}"
    );

    // Reactivation restores login and converges on the SAME canonical user.
    store
        .update_status(&user, RebornUserStatus::Active)
        .await
        .expect("reactivate");
    let reactivated = store
        .resolve_or_create(oauth(&t, "google", "sub-1", email, true))
        .await
        .expect("a reactivated account logs in again");
    assert_eq!(reactivated, user, "reactivation resolves the same user");
}

#[tokio::test]
async fn resolve_or_create_backfills_tenant_onto_a_legacy_tenantless_record() {
    // Regression: records written before the admin surface existed carry
    // `tenant_id: None` and would otherwise be visible to every tenant's admin
    // listing. The owning user's next login backfills the resolving tenant so
    // the record stops being globally visible.
    let store = store();
    let t = tenant("acme");
    let email = Some("legacy@example.com");

    let user = store
        .resolve_or_create(oauth(&t, "google", "sub-legacy", email, true))
        .await
        .expect("mint");

    // Rewrite the user record as a pre-admin, tenantless legacy row.
    let path = user_path(user.as_str()).expect("user path");
    let mut stored = store
        .read_record::<StoredUser>(&path)
        .await
        .expect("read record")
        .expect("record exists");
    stored.tenant_id = None;
    store
        .write_record(&path, &stored, CasExpectation::Any)
        .await
        .expect("rewrite as tenantless");

    // The next login for the same identity resolves the user and backfills.
    let resolved = store
        .resolve_or_create(oauth(&t, "google", "sub-legacy", email, true))
        .await
        .expect("re-login");
    assert_eq!(resolved, user);

    let after = store
        .read_record::<StoredUser>(&path)
        .await
        .expect("read record")
        .expect("record")
        .tenant_id;
    assert_eq!(
        after.as_deref(),
        Some("acme"),
        "login must backfill the resolving tenant onto a legacy tenantless record"
    );
}

#[tokio::test]
async fn record_last_login_sets_field_without_bumping_updated_at() {
    let store = store();
    let t = tenant("acme");
    let user = store
        .create_user(
            &t,
            None,
            None,
            RebornUserRole::Member,
            &UserId::new("by").unwrap(),
        )
        .await
        .expect("create");
    assert!(user.last_login_at.is_none());

    store
        .record_last_login(&user.user_id, "2026-07-07T10:00:00Z".to_string())
        .await
        .expect("record login");
    let after = store
        .get_user(&user.user_id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(after.last_login_at.as_deref(), Some("2026-07-07T10:00:00Z"));
    assert_eq!(
        after.updated_at, user.updated_at,
        "a login must not bump updated_at (that tracks profile edits)"
    );
}

#[tokio::test]
async fn delete_user_cascades_external_identity_and_verified_email_index() {
    // An SSO login mints a user + its external-identity record + its
    // verified-email index. Deleting the user must remove all three, so a later
    // login through the same identity mints a FRESH user instead of resolving
    // the tombstoned id back to life.
    let store = store();
    let t = tenant("t");
    let original = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("gone@x.com"), true))
        .await
        .expect("resolve");

    store.delete_user(&t, &original).await.expect("delete");

    // User record gone.
    assert!(
        store.get_user(&original).await.expect("get").is_none(),
        "the user record must be deleted"
    );
    // Verified-email index gone.
    let index = store
        .read_record::<StoredVerifiedEmailIndex>(&verified_email_path("t", "gone@x.com").unwrap())
        .await
        .expect("read index");
    assert!(index.is_none(), "the verified-email index must be cascaded");
    // External identity gone: a re-login mints a new, different user.
    let after = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("gone@x.com"), true))
        .await
        .expect("re-resolve");
    assert_ne!(
        after.as_str(),
        original.as_str(),
        "the external identity must be cascaded so a re-login does not revive the deleted user"
    );
}

#[tokio::test]
async fn resolve_or_create_refuses_to_relink_a_user_mid_delete() {
    // Regression for the delete-vs-relink race: a delete cascade removes the
    // external identity before the verified-email index, so during that window
    // a concurrent login (here via a DIFFERENT provider for the SAME verified
    // email) reaches the email-link branch and would otherwise re-link a fresh
    // identity onto the id being deleted — reviving a ghost. The in-flight
    // tombstone makes the resolver fail closed with a retryable error instead.
    let store = store();
    let t = tenant("acme");
    let user = store
        .resolve_or_create(oauth(&t, "google", "g-1", Some("shared@x.com"), true))
        .await
        .expect("first login mints the user + verified-email index");

    // Simulate the mid-cascade state: the delete has written the tombstone but
    // not yet removed the verified-email index.
    store
        .write_record(
            &user_tombstone_path(user.as_str()).unwrap(),
            &StoredUserTombstone {
                deleted_at: "2026-07-07T00:00:00Z".to_string(),
            },
            CasExpectation::Any,
        )
        .await
        .expect("write tombstone");

    let err = store
        .resolve_or_create(oauth(&t, "github", "gh-9", Some("shared@x.com"), true))
        .await
        .expect_err("a login racing a delete must not re-link the tombstoned user");
    assert!(
        matches!(err, RebornIdentityError::Backend(_)),
        "the relink must fail closed with a retryable error, got {err:?}"
    );

    // Once the tombstone clears (cascade finished), logins mint fresh again.
    store
        .filesystem
        .delete(&store.scope, &user_tombstone_path(user.as_str()).unwrap())
        .await
        .expect("clear tombstone");
    store
        .resolve_or_create(oauth(&t, "github", "gh-9", Some("shared@x.com"), true))
        .await
        .expect("after the tombstone clears, the email-link path resolves again");
}
