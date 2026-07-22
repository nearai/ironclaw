//! Regression coverage for the durable `build_local_runtime` product-auth
//! branch. The old in-memory branch is gone from normal builds, so this test now
//! always compiles with the production-shaped durable service.

//! Regression (issue #5378): the local-dev / hosted-single-tenant product-auth
//! composition must wrap the credential-account service in
//! `ProviderBackedCredentialAccountService` so the runtime token-refresh path is
//! provider-backed.
//!
//! Before the fix, `build_local_runtime`'s durable (libsql/postgres) branch
//! composed product-auth via `from_shared_with_provider(..).into_services(..)`
//! WITHOUT `.with_provider_client(..)`, leaving `credential_account_service` as
//! the raw `FilesystemAuthProductServices` whose `refresh_account` is an
//! unconditional stub returning `BackendUnavailable`. That swallowed every
//! Google OAuth token refresh (the inline refresher treats `BackendUnavailable`
//! as transient and returns the existing, now-expired token), so every
//! capability call forced a re-auth once the 1h access token expired. The
//! sibling `build_backend_production` path routes through
//! `compose_product_auth_services`, which applies the wrap — hence the bug was
//! profile-specific.
//!
//! Discriminator that needs no live OAuth provider: the provider-backed wrapper
//! performs an account lookup first and returns `CredentialMissing` for an
//! unknown account, whereas the raw stub ignores the request and returns
//! `BackendUnavailable` unconditionally. Asserting `CredentialMissing` therefore
//! proves the durable account service was wrapped, regardless of whether the
//! durable backend is libsql or postgres.

use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountId,
    CredentialRefreshRequest,
};
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use ironclaw_reborn_composition::{RebornBuildInput, build_reborn_services};

#[tokio::test]
async fn local_dev_product_auth_refresh_is_provider_backed_not_stub() {
    let dir = tempfile::tempdir().unwrap();
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "refresh-composition-owner",
        dir.path().to_path_buf(),
    ))
    .await
    .expect("local-dev runtime should build");

    let product_auth = services
        .product_auth
        .expect("local-dev composition must expose product-auth services");
    let account_service = product_auth.credential_account_service();

    let scope = AuthProductScope::new(
        ResourceScope::local_default(
            UserId::new("refresh-composition-owner").unwrap(),
            InvocationId::new(),
        )
        .unwrap(),
        AuthSurface::Api,
    );
    let request = CredentialRefreshRequest::new(
        scope,
        AuthProviderId::new("google").unwrap(),
        CredentialAccountId::new(),
    );

    let error = account_service
        .refresh_account(request)
        .await
        .expect_err("refreshing a non-existent account must error");

    // Provider-backed wrapper looks the account up first -> CredentialMissing.
    // The raw FilesystemAuthProductServices stub (the regression) ignores the
    // request and returns BackendUnavailable unconditionally.
    assert!(
        matches!(error, AuthProductError::CredentialMissing),
        "expected CredentialMissing (the provider-backed wrapper performs an \
         account lookup first), but got {error:?}. local-dev product-auth refresh \
         fell back to the raw FilesystemAuthProductServices stub — the credential \
         account service was not wrapped in ProviderBackedCredentialAccountService. \
         Regression: issue #5378."
    );
}
