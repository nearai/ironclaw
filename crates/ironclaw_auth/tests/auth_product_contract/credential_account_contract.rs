use crate::common::*;

#[tokio::test]
async fn credential_setup_updates_only_explicit_authorized_account() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let first = services
        .create_or_update_account(NewCredentialAccount {
            update_account_id: None,
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
        })
        .await
        .expect("create account");
    let access_secret = SecretHandle::new("github-updated-access").unwrap();
    let second = services
        .create_or_update_account(NewCredentialAccount {
            update_account_id: Some(first.id),
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
        })
        .await
        .expect("update account");

    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(second.label, label("work renamed"));
    assert_eq!(second.status, CredentialAccountStatus::Configured);
    assert_eq!(second.access_secret, Some(access_secret));
    assert_eq!(second.scopes, provider_scopes(&["repo"]));

    let same_label_without_target_creates_new_account = services
        .create_or_update_account(NewCredentialAccount {
            update_account_id: None,
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
        })
        .await
        .expect("same label without target is create");
    assert_ne!(same_label_without_target_creates_new_account.id, first.id);

    let rejected_takeover = services
        .create_or_update_account(NewCredentialAccount {
            update_account_id: Some(first.id),
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
        })
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
        .get_account(&owner, account.id)
        .await
        .expect("lookup")
        .expect("owner account");
    assert_eq!(still_owner.status, CredentialAccountStatus::RefreshFailed);
}

#[tokio::test]
async fn create_account_rejects_update_account_id() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let existing = services
        .create_account(account_request(
            owner.clone(),
            "work",
            CredentialAccountStatus::Configured,
        ))
        .await
        .expect("create account");
    let invalid = services
        .create_account(NewCredentialAccount {
            update_account_id: Some(existing.id),
            scope: owner,
            provider: provider(),
            label: label("work renamed"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect_err("create-only API rejects update ids");
    assert_eq!(invalid.code(), AuthErrorCode::InvalidRequest);
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
            update_account_id: None,
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
            update_account_id: None,
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
            update_account_id: None,
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
            update_account_id: None,
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
            update_account_id: None,
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
