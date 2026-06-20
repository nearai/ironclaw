use super::*;

#[tokio::test]
async fn resolver_uses_latest_duplicate_user_reusable_account() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let first_secret = SecretHandle::new("old-token").unwrap();
    let latest_secret = SecretHandle::new("new-token").unwrap();
    ConfiguredAccount::new(auth_scope.clone(), "github")
        .access_secret(Some(first_secret))
        .create(&accounts)
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    ConfiguredAccount::new(auth_scope, "github")
        .access_secret(Some(latest_secret.clone()))
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: &RuntimeCredentialAccountSetup::ManualToken,
            provider_scopes: &[],
            requester_extension: &ExtensionId::new("github").unwrap(),
        })
        .await
        .unwrap();

    assert_eq!(resolved.handle, latest_secret);
}

/// Direct reproduction of the reported bug (#auth-gate-reuse): a single
/// Google login authenticated through different gate/setup surfaces ends up
/// stored as multiple reusable accounts under capability-derived labels
/// ("gmail google", "google-calendar google"). The runtime resolver must
/// pick the most-recent usable credential instead of returning
/// `AuthRequired`, which re-prompted the user on every gmail/calendar call.
#[tokio::test]
async fn resolver_resolves_google_capability_labeled_duplicates() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let gmail_scope = ProviderScope::new("https://www.googleapis.com/auth/gmail.modify").unwrap();
    let latest_secret = SecretHandle::new("calendar-surface-token").unwrap();
    ConfiguredAccount::new(auth_scope.clone(), "google")
        .label("gmail google")
        .access_secret(Some(SecretHandle::new("gmail-surface-token").unwrap()))
        .scopes(&["https://www.googleapis.com/auth/gmail.modify"])
        .create(&accounts)
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    ConfiguredAccount::new(auth_scope, "google")
        .label("google-calendar google")
        .access_secret(Some(latest_secret.clone()))
        .scopes(&["https://www.googleapis.com/auth/gmail.modify"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let resolved = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[gmail_scope.as_str().to_string()],
            requester_extension: &ExtensionId::new("gmail").unwrap(),
        })
        .await
        .expect("capability-labeled google duplicates must resolve, not re-prompt");

    assert_eq!(resolved.handle, latest_secret);
}

#[tokio::test]
async fn resolver_does_not_auto_select_mixed_reusable_and_extension_owned_accounts() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let requester = ExtensionId::new("gmail").unwrap();
    let google_scope =
        ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
    ConfiguredAccount::new(auth_scope.clone(), "google")
        .access_secret(Some(SecretHandle::new("reusable-token").unwrap()))
        .scopes(&["https://www.googleapis.com/auth/gmail.readonly"])
        .create(&accounts)
        .await;
    ConfiguredAccount::new(auth_scope, "google")
        .ownership(CredentialOwnership::ExtensionOwned)
        .owner_extension("gmail")
        .access_secret(Some(SecretHandle::new("extension-token").unwrap()))
        .scopes(&["https://www.googleapis.com/auth/gmail.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[google_scope.as_str().to_string()],
            requester_extension: &requester,
        })
        .await
        .expect_err("mixed ownership must require explicit account selection");

    assert_eq!(error, CredentialStageError::AuthRequired);
}

#[tokio::test]
async fn resolver_does_not_auto_select_mixed_reusable_and_shared_admin_accounts() {
    let accounts = Arc::new(InMemoryAuthProductServices::new());
    let scope =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
    let requester = ExtensionId::new("gmail").unwrap();
    let google_scope =
        ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
    ConfiguredAccount::new(auth_scope.clone(), "google")
        .access_secret(Some(SecretHandle::new("reusable-token").unwrap()))
        .scopes(&["https://www.googleapis.com/auth/gmail.readonly"])
        .create(&accounts)
        .await;
    ConfiguredAccount::new(auth_scope, "google")
        .ownership(CredentialOwnership::SharedAdminManaged)
        .granted_extensions(vec![requester.clone()])
        .access_secret(Some(SecretHandle::new("shared-token").unwrap()))
        .scopes(&["https://www.googleapis.com/auth/gmail.readonly"])
        .create(&accounts)
        .await;
    let resolver = resolver_with_accounts(accounts);

    let error = resolver
        .resolve_access_secret(RuntimeCredentialAccountRequest {
            scope: &scope,
            provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
            setup: &RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            provider_scopes: &[google_scope.as_str().to_string()],
            requester_extension: &requester,
        })
        .await
        .expect_err("mixed sharing semantics must require explicit account selection");

    assert_eq!(error, CredentialStageError::AuthRequired);
}
