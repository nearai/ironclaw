//! `ironclaw_reborn_identity` on the int-tier coverage lane (enabler (a)).
//!
//! First scenario crossing an enumerated `--test` binary into the canonical
//! identity crate: the crate's own 790-line inline suite never runs under the
//! coverage-lane invocation (which passes only suite names, never `--lib`).
//!
//! Reaches the crate through the composition service only —
//! `open_reborn_identity_resolver` (the existing `test-support` gated
//! factory mirroring production's
//! `RebornRuntime::open_reborn_identity_resolver`) plus the re-exported
//! resolver vocabulary. Composition deliberately keeps the concrete
//! `FilesystemRebornIdentityStore` private ("keep lower substrate handles
//! private"), so this suite takes no direct `ironclaw_reborn_identity`
//! dependency. The factory's in-memory host filesystem replaces the plan's
//! "tempdir" wording — same store code path, no on-disk state.
//!
//! Flat suite, no harness mounts: identity resolution is a standalone store
//! round trip, not an agent turn (`RebornIntegrationHarness` models the
//! latter).

use ironclaw_reborn_composition::host_api::TenantId;
use ironclaw_reborn_composition::{
    ExternalSubjectId, ProviderKind, ResolveExternalIdentity, SurfaceKind,
    open_reborn_identity_resolver,
};

fn oauth_identity(tenant: &TenantId, subject: &str) -> ResolveExternalIdentity {
    ResolveExternalIdentity {
        tenant_id: tenant.clone(),
        surface_kind: SurfaceKind::Oauth,
        provider_kind: ProviderKind::new("google").expect("provider"),
        provider_instance_id: None,
        external_subject_id: ExternalSubjectId::new(subject).expect("subject"),
        email: Some("alice@example.com".to_string()),
        email_verified: true,
        display_name: Some("Alice".to_string()),
    }
}

/// First contact mints a user; re-resolving the SAME external identity
/// returns the SAME canonical `UserId` (the store's lookup-before-create
/// path), while a DIFFERENT subject mints a different user — so the
/// stability assertion discriminates on the identity key, not on a
/// constant return.
#[tokio::test]
async fn oauth_identity_resolves_to_stable_user_id() {
    let tenant = TenantId::new("tenant-identity-smoke").expect("tenant");
    let resolver = open_reborn_identity_resolver(&tenant);

    let minted = resolver
        .resolve_or_create(oauth_identity(&tenant, "google-sub-1"))
        .await
        .expect("first contact mints a user");
    let resolved = resolver
        .resolve_or_create(oauth_identity(&tenant, "google-sub-1"))
        .await
        .expect("re-resolution succeeds");
    assert_eq!(
        minted, resolved,
        "same external identity resolves to the same canonical user id"
    );

    let other = resolver
        .resolve_or_create(ResolveExternalIdentity {
            email: Some("bob@example.com".to_string()),
            display_name: Some("Bob".to_string()),
            ..oauth_identity(&tenant, "google-sub-2")
        })
        .await
        .expect("distinct subject mints its own user");
    assert_ne!(
        minted, other,
        "a different external subject must not collapse onto the first user"
    );
}
