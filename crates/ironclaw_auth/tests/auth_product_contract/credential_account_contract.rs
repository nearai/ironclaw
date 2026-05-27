use crate::common::*;

#[tokio::test]
async fn credential_setup_updates_only_explicit_authorized_account() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let first = services
        .create_or_update_account(CredentialAccountMutation::Create(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
        }))
        .await
        .expect("create account");
    let access_secret = SecretHandle::new("github-updated-access").unwrap();
    let second = services
        .create_or_update_account(CredentialAccountMutation::Update(CredentialAccountUpdate {
            account_id: first.id,
            account: NewCredentialAccount {
                scope: owner.clone(),
                provider: provider(),
                label: label("work renamed"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(access_secret.clone()),
                refresh_secret: None,
                scopes: provider_scopes(&["repo"]),
            },
        }))
        .await
        .expect("update account");

    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(second.label, label("work renamed"));
    assert_eq!(second.status, CredentialAccountStatus::Configured);
    assert_eq!(second.access_secret, Some(access_secret));
    assert_eq!(second.scopes, provider_scopes(&["repo"]));

    let same_label_without_target_creates_new_account = services
        .create_or_update_account(CredentialAccountMutation::Create(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work renamed"),
            status: CredentialAccountStatus::PendingSetup,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
        }))
        .await
        .expect("same label without target is create");
    assert_ne!(same_label_without_target_creates_new_account.id, first.id);

    let rejected_takeover = services
        .create_or_update_account(CredentialAccountMutation::Update(CredentialAccountUpdate {
            account_id: first.id,
            account: NewCredentialAccount {
                scope: owner.clone(),
                provider: provider(),
                label: label("takeover"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::ExtensionOwned,
                owner_extension: Some(ExtensionId::new("attacker").unwrap()),
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("github-takeover").unwrap()),
                refresh_secret: None,
                scopes: provider_scopes(&["repo"]),
            },
        }))
        .await
        .expect_err("ownership changes require a separate authority flow");
    assert_eq!(rejected_takeover, AuthProductError::CrossScopeDenied);

    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(owner, provider()).with_limit(10))
        .await
        .expect("list accounts");
    assert_eq!(accounts.accounts.len(), 2);
    assert!(
        accounts
            .accounts
            .iter()
            .any(|account| account.id == first.id)
    );
    assert!(accounts.next_cursor.is_none());
}

#[tokio::test]
async fn credential_account_update_status_updates_owner_record_and_rejects_missing_or_cross_scope()
{
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(account_request(
            owner.clone(),
            "work",
            CredentialAccountStatus::Configured,
        ))
        .await
        .expect("create account");

    let updated = services
        .update_status(&owner, account.id, CredentialAccountStatus::RefreshFailed)
        .await
        .expect("update status");
    assert_eq!(updated.status, CredentialAccountStatus::RefreshFailed);

    let missing = services
        .update_status(
            &owner,
            ironclaw_auth::CredentialAccountId::new(),
            CredentialAccountStatus::Revoked,
        )
        .await
        .expect_err("missing account");
    assert_eq!(missing, AuthProductError::CredentialMissing);

    let cross_scope = services
        .update_status(&scope("bob"), account.id, CredentialAccountStatus::Revoked)
        .await
        .expect_err("cross-scope account");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

    let still_owner = services
        .get_account(CredentialAccountLookupRequest::new(
            owner.clone(),
            account.id,
        ))
        .await
        .expect("lookup")
        .expect("owner account");
    assert_eq!(still_owner.status, CredentialAccountStatus::RefreshFailed);

    let revoked = services
        .update_status(&owner, account.id, CredentialAccountStatus::Revoked)
        .await
        .expect("terminal revoke");
    assert_eq!(revoked.status, CredentialAccountStatus::Revoked);

    let reactivated = services
        .update_status(&owner, account.id, CredentialAccountStatus::Configured)
        .await
        .expect_err("revoked accounts cannot be reactivated by status update");
    assert_eq!(reactivated.code(), AuthErrorCode::InvalidRequest);
}

#[tokio::test]
async fn credential_account_list_is_explicitly_paginated() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    for name in ["alpha", "beta", "gamma"] {
        services
            .create_account(account_request(
                owner.clone(),
                name,
                CredentialAccountStatus::Configured,
            ))
            .await
            .expect("create account");
    }

    let first_page = services
        .list_accounts(CredentialAccountListRequest::new(owner.clone(), provider()).with_limit(2))
        .await
        .expect("first page");
    assert_eq!(first_page.accounts.len(), 2);
    let cursor = first_page
        .next_cursor
        .expect("cursor for remaining account");

    let second_page = services
        .list_accounts(
            CredentialAccountListRequest::new(owner.clone(), provider())
                .with_limit(2)
                .with_cursor(cursor),
        )
        .await
        .expect("second page");
    assert_eq!(second_page.accounts.len(), 1);
    assert!(second_page.next_cursor.is_none());

    let zero_limit = services
        .list_accounts(CredentialAccountListRequest::new(owner.clone(), provider()).with_limit(0))
        .await
        .expect_err("zero limit rejected");
    assert_eq!(zero_limit.code(), AuthErrorCode::InvalidRequest);
    let too_large = services
        .list_accounts(
            CredentialAccountListRequest::new(owner, provider())
                .with_limit(CredentialAccountListRequest::MAX_LIMIT + 1),
        )
        .await
        .expect_err("oversized limit rejected");
    assert_eq!(too_large.code(), AuthErrorCode::InvalidRequest);
}

#[tokio::test]
async fn credential_account_list_filters_by_requester_authority() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice-list-authz");
    let github_extension = ExtensionId::new("github-extension").unwrap();
    let other_extension = ExtensionId::new("other-extension").unwrap();
    let reusable = services
        .create_account(account_request(
            owner.clone(),
            "reusable",
            CredentialAccountStatus::Configured,
        ))
        .await
        .expect("reusable account");
    let extension_owned = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("extension owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(github_extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-extension-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("extension-owned account");
    let shared = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("shared"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![github_extension.clone()],
            access_secret: Some(SecretHandle::new("github-shared").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("shared account");

    let unscoped = services
        .list_accounts(CredentialAccountListRequest::new(owner.clone(), provider()).with_limit(10))
        .await
        .expect("unscoped list");
    assert_eq!(account_ids(&unscoped.accounts), vec![reusable.id]);

    let wrong_extension = services
        .list_accounts(
            CredentialAccountListRequest::new(owner.clone(), provider())
                .for_extension(other_extension)
                .with_limit(10),
        )
        .await
        .expect("wrong extension list");
    assert_eq!(account_ids(&wrong_extension.accounts), vec![reusable.id]);

    let owning_extension = services
        .list_accounts(
            CredentialAccountListRequest::new(owner, provider())
                .for_extension(github_extension)
                .with_limit(10),
        )
        .await
        .expect("owning extension list");
    let ids = account_ids(&owning_extension.accounts);
    assert!(ids.contains(&reusable.id));
    assert!(ids.contains(&extension_owned.id));
    assert!(ids.contains(&shared.id));
    assert_eq!(ids.len(), 3);
}

#[tokio::test]
async fn credential_account_lookup_filters_by_requester_authority() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice-lookup-authz");
    let github_extension = ExtensionId::new("github-extension").unwrap();
    let other_extension = ExtensionId::new("other-extension").unwrap();
    let extension_owned = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("extension owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(github_extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-extension-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("extension-owned account");

    for request in [
        CredentialAccountLookupRequest::new(owner.clone(), extension_owned.id),
        CredentialAccountLookupRequest::new(owner.clone(), extension_owned.id)
            .for_extension(other_extension),
    ] {
        let denied = services
            .get_account(request)
            .await
            .expect_err("unauthorized lookup is rejected");
        assert_eq!(denied, AuthProductError::CrossScopeDenied);
    }

    let selected = services
        .get_account(
            CredentialAccountLookupRequest::new(owner, extension_owned.id)
                .for_extension(github_extension),
        )
        .await
        .expect("lookup")
        .expect("authorized account");
    assert_eq!(selected.id, extension_owned.id);
}

#[tokio::test]
async fn credential_account_selection_requires_user_choice_for_multiple_configured_accounts() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");

    let missing = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect_err("no configured account");
    assert_eq!(missing, AuthProductError::CredentialMissing);

    services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("expired"),
            status: CredentialAccountStatus::RefreshFailed,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-expired").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("refresh-failed account");
    let still_missing = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect_err("refresh-failed account is not selectable");
    assert_eq!(still_missing, AuthProductError::CredentialMissing);

    let work = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-work").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("work account");
    let selected = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect("single configured account");
    assert_eq!(selected.id, work.id);

    services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("personal"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-personal").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("personal account");
    let err = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect_err("multiple accounts require choice");
    assert_eq!(err, AuthProductError::AccountSelectionRequired);
}

#[tokio::test]
async fn credential_account_selection_filters_by_requester_authority() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let github_extension = ExtensionId::new("github-extension").unwrap();
    let other_extension = ExtensionId::new("other-extension").unwrap();

    let extension_owned = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("extension owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(github_extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-extension-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("extension-owned account");

    let unauthorized = services
        .select_unique_configured_account(CredentialAccountSelectionRequest::new(
            owner.clone(),
            provider(),
        ))
        .await
        .expect_err("no requester cannot select extension-owned account");
    assert_eq!(unauthorized, AuthProductError::CrossScopeDenied);
    let wrong_requester = services
        .select_unique_configured_account(
            CredentialAccountSelectionRequest::new(owner.clone(), provider())
                .for_extension(other_extension.clone()),
        )
        .await
        .expect_err("wrong requester cannot select extension-owned account");
    assert_eq!(wrong_requester, AuthProductError::CrossScopeDenied);

    let selected = services
        .select_unique_configured_account(
            CredentialAccountSelectionRequest::new(owner.clone(), provider())
                .for_extension(github_extension.clone()),
        )
        .await
        .expect("owning requester can select account");
    assert_eq!(selected.id, extension_owned.id);

    services
        .update_status(&owner, extension_owned.id, CredentialAccountStatus::Revoked)
        .await
        .expect("hide extension-owned account for shared test");
    let shared = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("shared"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![github_extension.clone()],
            access_secret: Some(SecretHandle::new("github-shared").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("shared account");
    let selected_shared = services
        .select_unique_configured_account(
            CredentialAccountSelectionRequest::new(owner, provider())
                .for_extension(github_extension),
        )
        .await
        .expect("granted requester can select shared account");
    assert_eq!(selected_shared.id, shared.id);
}

#[tokio::test]
async fn credential_recovery_projection_distinguishes_recoverable_states() {
    let empty = InMemoryAuthProductServices::new();
    let owner = scope("alice-empty");
    let no_account = empty
        .project_credential_recovery(CredentialRecoveryRequest::new(owner, provider()))
        .await
        .expect("empty recovery projection");
    assert_eq!(no_account.kind(), CredentialRecoveryKind::SetupRequired);
    assert_eq!(no_account.reason, CredentialRecoveryReason::NoAccount);
    assert!(no_account.selected_account().is_none());
    assert!(no_account.choices().is_empty());

    for (status, expected_kind, expected_reason, user) in [
        (
            CredentialAccountStatus::Configured,
            CredentialRecoveryKind::Configured,
            CredentialRecoveryReason::Configured,
            "alice-configured",
        ),
        (
            CredentialAccountStatus::Missing,
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::AccountMissing,
            "alice-missing",
        ),
        (
            CredentialAccountStatus::PendingSetup,
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::PendingSetup,
            "alice-pending",
        ),
        (
            CredentialAccountStatus::Inactive,
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::AccountInactive,
            "alice-inactive",
        ),
        (
            CredentialAccountStatus::Expired,
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::AccountExpired,
            "alice-expired",
        ),
        (
            CredentialAccountStatus::RefreshFailed,
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::RefreshFailed,
            "alice-refresh-failed",
        ),
        (
            CredentialAccountStatus::Revoked,
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::AccountRevoked,
            "alice-revoked",
        ),
    ] {
        let services = InMemoryAuthProductServices::new();
        let owner = scope(user);
        let account = services
            .create_account(NewCredentialAccount {
                scope: owner.clone(),
                provider: provider(),
                label: label("work"),
                status,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new(format!("github-{user}-access")).unwrap()),
                refresh_secret: Some(SecretHandle::new(format!("github-{user}-refresh")).unwrap()),
                scopes: provider_scopes(&["repo"]),
            })
            .await
            .expect("create account");

        let projection = services
            .project_credential_recovery(CredentialRecoveryRequest::new(owner, provider()))
            .await
            .expect("recovery projection");

        assert_eq!(projection.kind(), expected_kind);
        assert_eq!(projection.reason, expected_reason);
        if expected_kind == CredentialRecoveryKind::Configured {
            assert_eq!(
                projection.selected_account().map(|choice| choice.id),
                Some(account.id)
            );
            assert!(projection.choices().is_empty());
        } else {
            assert!(projection.selected_account().is_none());
            assert_eq!(projection.choices().len(), 1);
            assert_eq!(projection.choices()[0].id, account.id);
        }
    }
}

#[tokio::test]
async fn credential_recovery_projection_returns_redacted_authorized_choices() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let work = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("work"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-work-secret").unwrap()),
            refresh_secret: Some(SecretHandle::new("github-work-refresh-secret").unwrap()),
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("work account");
    services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("personal"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-personal-secret").unwrap()),
            refresh_secret: None,
            scopes: provider_scopes(&["gist"]),
        })
        .await
        .expect("personal account");

    let projection = services
        .project_credential_recovery(CredentialRecoveryRequest::new(owner.clone(), provider()))
        .await
        .expect("recovery projection");
    assert_eq!(
        projection.kind(),
        CredentialRecoveryKind::AccountSelectionRequired
    );
    assert_eq!(
        projection.reason,
        CredentialRecoveryReason::AmbiguousAccount
    );
    assert_eq!(projection.choices().len(), 2);
    assert!(projection.selected_account().is_none());

    let serialized = serde_json::to_string(&projection).unwrap();
    let round_trip: CredentialRecoveryProjection = serde_json::from_str(&serialized).unwrap();
    assert_eq!(round_trip, projection);
    let wire = serde_json::to_value(&projection).unwrap();
    assert_eq!(
        wire["kind"],
        serde_json::json!("account_selection_required")
    );
    assert_eq!(wire["reason"], serde_json::json!("ambiguous_account"));
    assert!(!serialized.contains("github-work-secret"));
    assert!(!serialized.contains("github-work-refresh-secret"));
    assert!(!serialized.contains("github-personal-secret"));
    assert!(!serialized.contains("raw provider body"));
    assert!(!serialized.contains("/host/path"));
    assert!(!serialized.contains("lease-"));

    let selected = services
        .select_configured_account(CredentialAccountChoiceRequest::new(
            owner.clone(),
            provider(),
            work.id,
        ))
        .await
        .expect("explicit policy selection");
    assert_eq!(selected.id, work.id);

    let cross_scope = services
        .select_configured_account(CredentialAccountChoiceRequest::new(
            scope("bob"),
            provider(),
            work.id,
        ))
        .await
        .expect_err("another scope cannot bind an existing account id");
    assert_eq!(cross_scope, AuthProductError::CrossScopeDenied);

    let provider_mismatch = services
        .select_configured_account(CredentialAccountChoiceRequest::new(
            owner.clone(),
            AuthProviderId::new("gitlab").unwrap(),
            work.id,
        ))
        .await
        .expect_err("provider mismatch cannot bind an existing account id");
    assert_eq!(provider_mismatch, AuthProductError::CrossScopeDenied);

    let missing = services
        .select_configured_account(CredentialAccountChoiceRequest::new(
            owner.clone(),
            provider(),
            ironclaw_auth::CredentialAccountId::new(),
        ))
        .await
        .expect_err("arbitrary missing account id is rejected");
    assert_eq!(missing, AuthProductError::CredentialMissing);

    let expired = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("expired"),
            status: CredentialAccountStatus::Expired,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-expired-secret").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("expired account");
    let unusable = services
        .select_configured_account(CredentialAccountChoiceRequest::new(
            owner,
            provider(),
            expired.id,
        ))
        .await
        .expect_err("unusable account id is rejected");
    assert_eq!(unusable, AuthProductError::CredentialMissing);
}

#[tokio::test]
async fn credential_recovery_projection_does_not_offer_unselectable_nonconfigured_choices() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice-nonconfigured");
    for name in ["missing work", "missing personal"] {
        services
            .create_account(account_request(
                owner.clone(),
                name,
                CredentialAccountStatus::Missing,
            ))
            .await
            .expect("missing account");
    }

    let setup = services
        .project_credential_recovery(CredentialRecoveryRequest::new(owner.clone(), provider()))
        .await
        .expect("missing recovery projection");
    assert_eq!(setup.kind(), CredentialRecoveryKind::SetupRequired);
    assert_eq!(setup.reason, CredentialRecoveryReason::AccountMissing);
    assert_eq!(setup.choices().len(), 2);
    assert!(setup.selected_account().is_none());

    let reauth_services = InMemoryAuthProductServices::new();
    let reauth_owner = scope("alice-reauthorize");
    for name in ["expired work", "expired personal"] {
        reauth_services
            .create_account(account_request(
                reauth_owner.clone(),
                name,
                CredentialAccountStatus::Expired,
            ))
            .await
            .expect("expired account");
    }

    let reauthorize = reauth_services
        .project_credential_recovery(CredentialRecoveryRequest::new(reauth_owner, provider()))
        .await
        .expect("expired recovery projection");
    assert_eq!(
        reauthorize.kind(),
        CredentialRecoveryKind::ReauthorizeRequired
    );
    assert_eq!(reauthorize.reason, CredentialRecoveryReason::AccountExpired);
    assert_eq!(reauthorize.choices().len(), 2);
    assert!(reauthorize.selected_account().is_none());
}

#[tokio::test]
async fn extension_owned_credential_recovery_filters_by_owner_extension() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice-extension-recovery");
    let github_extension = ExtensionId::new("github-extension").unwrap();
    let other_extension = ExtensionId::new("other-extension").unwrap();
    let extension_owned = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("extension owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(github_extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-extension-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("extension-owned account");

    for request in [
        CredentialRecoveryRequest::new(owner.clone(), provider()),
        CredentialRecoveryRequest::new(owner.clone(), provider()).for_extension(other_extension),
    ] {
        let projection = services
            .project_credential_recovery(request)
            .await
            .expect("recovery projection");
        assert_eq!(projection.kind(), CredentialRecoveryKind::SetupRequired);
        assert_eq!(projection.reason, CredentialRecoveryReason::NoAccount);
        assert!(projection.choices().is_empty());
        assert!(projection.selected_account().is_none());
    }

    let selected = services
        .project_credential_recovery(
            CredentialRecoveryRequest::new(owner, provider()).for_extension(github_extension),
        )
        .await
        .expect("owning extension recovery projection");
    assert_eq!(selected.kind(), CredentialRecoveryKind::Configured);
    assert_eq!(
        selected.selected_account().map(|account| account.id),
        Some(extension_owned.id)
    );
}

#[tokio::test]
async fn shared_admin_managed_credentials_require_explicit_grants() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let github_extension = ExtensionId::new("github-extension").unwrap();
    let other_extension = ExtensionId::new("other-extension").unwrap();
    let shared = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("shared"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![github_extension.clone()],
            access_secret: Some(SecretHandle::new("github-shared-secret").unwrap()),
            refresh_secret: None,
            scopes: provider_scopes(&["repo"]),
        })
        .await
        .expect("shared account");

    for request in [
        CredentialRecoveryRequest::new(owner.clone(), provider()),
        CredentialRecoveryRequest::new(owner.clone(), provider())
            .for_extension(other_extension.clone()),
    ] {
        let projection = services
            .project_credential_recovery(request)
            .await
            .expect("recovery projection");
        assert_eq!(projection.kind(), CredentialRecoveryKind::SetupRequired);
        assert_eq!(projection.reason, CredentialRecoveryReason::NoAccount);
        assert!(projection.choices().is_empty());
        assert!(projection.selected_account().is_none());
    }

    let granted = services
        .project_credential_recovery(
            CredentialRecoveryRequest::new(owner.clone(), provider())
                .for_extension(github_extension.clone()),
        )
        .await
        .expect("granted recovery projection");
    assert_eq!(granted.kind(), CredentialRecoveryKind::Configured);
    assert_eq!(
        granted.selected_account().map(|account| account.id),
        Some(shared.id)
    );

    let denied = services
        .select_configured_account(
            CredentialAccountChoiceRequest::new(owner.clone(), provider(), shared.id)
                .for_extension(other_extension),
        )
        .await
        .expect_err("ungranted extension cannot bind shared account id");
    assert_eq!(denied, AuthProductError::CrossScopeDenied);

    let selected = services
        .select_configured_account(
            CredentialAccountChoiceRequest::new(owner, provider(), shared.id)
                .for_extension(github_extension),
        )
        .await
        .expect("granted extension can bind shared account");
    assert_eq!(selected.id, shared.id);
}
