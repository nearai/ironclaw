use crate::common::*;

#[tokio::test]
async fn extension_owned_accounts_require_owner_and_cleanup_is_action_specific() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let extension = ExtensionId::new("github").unwrap();
    let orphan = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("orphan"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-orphan").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect_err("extension owned requires owner");
    assert_eq!(orphan.code(), AuthErrorCode::InvalidRequest);

    let owned = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("owned account");
    let reusable = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("reusable"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![extension.clone()],
            access_secret: Some(SecretHandle::new("github-reusable").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("reusable account");

    let deactivate = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner.clone(),
            extension_id: extension.clone(),
            provider: None,
            action: SecretCleanupAction::Deactivate,
        })
        .await
        .expect("deactivate");
    assert!(deactivate.retained_accounts.contains(&owned.id));
    assert!(deactivate.removed_grants.contains(&reusable.id));
    assert!(deactivate.revoked_accounts.is_empty());
    let inactive_owned = services
        .get_account(
            CredentialAccountLookupRequest::new(owner.clone(), owned.id)
                .for_extension(extension.clone()),
        )
        .await
        .expect("lookup")
        .expect("owned account remains");
    assert_eq!(inactive_owned.status, CredentialAccountStatus::Inactive);
    let isolated_services = InMemoryAuthProductServices::new();
    let isolated_owned = isolated_services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("isolated owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-isolated-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("isolated owned account");
    isolated_services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner.clone(),
            extension_id: extension.clone(),
            provider: None,
            action: SecretCleanupAction::Deactivate,
        })
        .await
        .expect("isolated deactivate");
    let deactivated_selection = isolated_services
        .select_unique_configured_account(
            CredentialAccountSelectionRequest::new(owner.clone(), provider())
                .for_extension(extension.clone()),
        )
        .await
        .expect_err("deactivated extension-owned account is not selectable");
    assert_eq!(deactivated_selection, AuthProductError::CredentialMissing);
    let isolated_after = isolated_services
        .get_account(
            CredentialAccountLookupRequest::new(owner.clone(), isolated_owned.id)
                .for_extension(extension.clone()),
        )
        .await
        .expect("lookup")
        .expect("isolated account remains");
    assert_eq!(isolated_after.status, CredentialAccountStatus::Inactive);

    let uninstall = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner,
            extension_id: extension,
            provider: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("uninstall");
    assert!(uninstall.revoked_accounts.contains(&owned.id));
}

#[tokio::test]
async fn cleanup_for_lifecycle_ignores_cross_scope_accounts() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let foreign_owner = scope("bob");
    let extension = ExtensionId::new("github").unwrap();

    let foreign_owned = services
        .create_account(NewCredentialAccount {
            scope: foreign_owner.clone(),
            provider: provider(),
            label: label("foreign owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-foreign-owned").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("foreign owned account");
    let foreign_granted = services
        .create_account(NewCredentialAccount {
            scope: foreign_owner.clone(),
            provider: provider(),
            label: label("foreign granted"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![extension.clone()],
            access_secret: Some(SecretHandle::new("github-foreign-granted").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("foreign granted account");

    let report = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner,
            extension_id: extension.clone(),
            provider: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("cleanup");
    assert!(report.revoked_accounts.is_empty());
    assert!(report.retained_accounts.is_empty());
    assert!(report.removed_grants.is_empty());

    let owned_after = services
        .get_account(
            CredentialAccountLookupRequest::new(foreign_owner.clone(), foreign_owned.id)
                .for_extension(extension.clone()),
        )
        .await
        .expect("lookup")
        .expect("foreign owned remains");
    assert_eq!(owned_after.status, CredentialAccountStatus::Configured);
    assert_eq!(owned_after.owner_extension, Some(extension.clone()));
    let granted_after = services
        .get_account(
            CredentialAccountLookupRequest::new(foreign_owner, foreign_granted.id)
                .for_extension(extension.clone()),
        )
        .await
        .expect("lookup")
        .expect("foreign granted remains");
    assert_eq!(granted_after.granted_extensions, vec![extension]);
}

#[tokio::test]
async fn cleanup_lifecycle_is_idempotent_and_quarantines_partial_failures() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let extension = ExtensionId::new("github").unwrap();
    let other_extension = ExtensionId::new("slack").unwrap();
    let owned = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-owned-secret").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("owned account");
    let shared = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("shared"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::SharedAdminManaged,
            owner_extension: None,
            granted_extensions: vec![extension.clone(), other_extension.clone()],
            access_secret: Some(SecretHandle::new("github-shared-secret").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("shared account");
    let system = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("system"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::System,
            owner_extension: None,
            granted_extensions: vec![extension.clone()],
            access_secret: Some(SecretHandle::new("github-system-secret").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("system account");
    let quarantined = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("quarantine"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-quarantine-secret").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("quarantined account");
    services
        .quarantine_cleanup_for_tests(quarantined.id, SecretCleanupQuarantineReason::RevokeFailed);

    let report = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner.clone(),
            extension_id: extension.clone(),
            provider: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("cleanup");

    assert_eq!(report.revoked_accounts, vec![owned.id]);
    assert!(report.retained_accounts.contains(&shared.id));
    assert!(report.retained_accounts.contains(&system.id));
    assert!(report.removed_grants.contains(&shared.id));
    assert!(report.removed_grants.contains(&system.id));
    assert_eq!(report.quarantined_accounts.len(), 1);
    assert_eq!(report.quarantined_accounts[0].account_id, quarantined.id);
    assert_eq!(
        report.quarantined_accounts[0].reason,
        SecretCleanupQuarantineReason::RevokeFailed
    );

    let owned_after = services
        .get_account(
            CredentialAccountLookupRequest::new(owner.clone(), owned.id)
                .for_extension(extension.clone()),
        )
        .await
        .expect("lookup")
        .expect("owned remains tombstoned");
    assert_eq!(owned_after.status, CredentialAccountStatus::Revoked);
    let shared_after = services
        .get_account(
            CredentialAccountLookupRequest::new(owner.clone(), shared.id)
                .for_extension(other_extension.clone()),
        )
        .await
        .expect("lookup")
        .expect("shared remains");
    assert_eq!(shared_after.status, CredentialAccountStatus::Configured);
    assert_eq!(shared_after.granted_extensions, vec![other_extension]);
    let quarantined_after = services
        .get_account(
            CredentialAccountLookupRequest::new(owner.clone(), quarantined.id)
                .for_extension(extension.clone()),
        )
        .await
        .expect("lookup")
        .expect("quarantined remains");
    assert_eq!(
        quarantined_after.status,
        CredentialAccountStatus::Configured
    );
    assert_eq!(quarantined_after.owner_extension, Some(extension.clone()));

    let second_report = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner,
            extension_id: extension,
            provider: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("cleanup is idempotent");
    assert!(second_report.revoked_accounts.is_empty());
    assert!(second_report.removed_grants.is_empty());
    assert_eq!(second_report.quarantined_accounts.len(), 1);
    assert_eq!(
        second_report.quarantined_accounts[0].account_id,
        quarantined.id
    );

    let serialized = serde_json::to_string(&report).expect("serialize report");
    assert!(!serialized.contains("github-owned-secret"));
    assert!(!serialized.contains("github-shared-secret"));
    assert!(!serialized.contains("github-system-secret"));
    assert!(!serialized.contains("github-quarantine-secret"));
    assert!(!serialized.contains("RAW_BACKEND_ERROR_SENTINEL"));
    assert!(!serialized.contains("/host/path"));
}

#[tokio::test]
async fn deactivate_cleanup_quarantines_partial_failures_without_mutating_account() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let extension = ExtensionId::new("github").unwrap();
    let account = services
        .create_account(NewCredentialAccount {
            scope: owner.clone(),
            provider: provider(),
            label: label("deactivate quarantine"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("github-deactivate-quarantine").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("owned account");
    services.quarantine_cleanup_for_tests(
        account.id,
        SecretCleanupQuarantineReason::BackendUnavailable,
    );

    let report = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: owner.clone(),
            extension_id: extension.clone(),
            provider: None,
            action: SecretCleanupAction::Deactivate,
        })
        .await
        .expect("deactivate cleanup");

    assert!(report.retained_accounts.is_empty());
    assert!(report.revoked_accounts.is_empty());
    assert!(report.removed_grants.is_empty());
    assert_eq!(report.quarantined_accounts.len(), 1);
    assert_eq!(report.quarantined_accounts[0].account_id, account.id);
    assert_eq!(
        report.quarantined_accounts[0].reason,
        SecretCleanupQuarantineReason::BackendUnavailable
    );

    let stored = services
        .get_account(
            CredentialAccountLookupRequest::new(owner, account.id).for_extension(extension),
        )
        .await
        .expect("lookup")
        .expect("account remains");
    assert_eq!(stored.status, CredentialAccountStatus::Configured);
}

/// OAuth-minted personal credentials are stored `UserReusable` with NO
/// extension ownership or grants (`flows.rs` account creation), and every
/// later lifecycle/disconnect caller re-derives the owner scope with a fresh
/// `invocation_id` (and often a different thread) — the #4935 defect-A shape.
/// Cleanup must therefore match at credential-owner granularity and be able to
/// select accounts by PROVIDER, or an uninstall can never revoke the token.
#[tokio::test]
async fn cleanup_matches_owner_granularity_and_provider_selected_oauth_accounts() {
    let services = InMemoryAuthProductServices::new();
    let extension = ExtensionId::new("slack").unwrap();

    // Exactly how the OAuth callback mints a personal credential.
    let oauth_account = services
        .create_account(NewCredentialAccount {
            scope: scope("alice"),
            provider: provider(),
            label: label("personal oauth"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("personal-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("oauth account");

    let report = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: reconnect_scope("alice", "thread-uninstall"),
            extension_id: extension.clone(),
            provider: Some(provider()),
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("provider-selected cleanup");
    assert_eq!(report.revoked_accounts, vec![oauth_account.id]);

    // A different provider may share the owner/grant selectors, but it must
    // not be revoked unless the cleanup explicitly selects that provider.
    let other_provider_account = services
        .create_account(NewCredentialAccount {
            scope: scope("alice"),
            provider: AuthProviderId::new("notion").unwrap(),
            label: label("other provider"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: Some(extension.clone()),
            granted_extensions: vec![extension.clone()],
            access_secret: Some(SecretHandle::new("other-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("other provider account");

    // Extension-owned accounts must also match across invocations/threads.
    let owned = services
        .create_account(NewCredentialAccount {
            scope: scope("alice"),
            provider: provider(),
            label: label("owned"),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("owned-access").unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        })
        .await
        .expect("owned account");

    let report = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: reconnect_scope("alice", "thread-uninstall-2"),
            extension_id: extension,
            provider: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("extension-keyed cleanup");
    assert_eq!(report.revoked_accounts, vec![owned.id]);
    assert_eq!(report.retained_accounts, vec![other_provider_account.id]);
    assert_eq!(report.removed_grants, vec![other_provider_account.id]);
    assert!(
        !report.revoked_accounts.contains(&other_provider_account.id),
        "an unrelated provider's account must survive extension-keyed cleanup"
    );
}

#[tokio::test]
async fn completed_unacknowledged_turn_gate_cleanup_emits_once_then_converges() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let account = services
        .create_account(account_request(
            owner.clone(),
            "cleanup completed",
            CredentialAccountStatus::Configured,
        ))
        .await
        .expect("account");
    let expires_at = Utc::now() + Duration::minutes(5);
    let flow = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::AccountSelectionRequired {
                provider: provider(),
                accounts: vec![account.projection()],
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(uuid::Uuid::new_v4().to_string())
                    .expect("turn run ref"),
                gate_ref: AuthGateRef::new("gate:cleanup-completed").expect("gate ref"),
            },
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .expect("flow");
    let completed = services
        .complete_credential_selection(
            &owner,
            CredentialSelectionInput {
                flow_id: flow.id,
                credential_account_id: account.id,
            },
        )
        .await
        .expect("selection completes");
    assert_eq!(completed.status, AuthFlowStatus::Completed);
    assert!(completed.continuation_emitted_at.is_none());

    // Completion is durable but dispatch is not acknowledged yet. Lifecycle
    // cleanup must still synthesize a denial so a gate cannot remain parked
    // after its credential is removed.
    let request = SecretCleanupRequest {
        scope: owner.clone(),
        extension_id: ExtensionId::new("github").expect("extension"),
        provider: Some(provider()),
        action: SecretCleanupAction::Uninstall,
    };
    let report = services
        .cleanup_for_lifecycle(request.clone())
        .await
        .expect("cleanup");
    assert_eq!(report.canceled_turn_gate_continuations.len(), 1);
    let event = &report.canceled_turn_gate_continuations[0];
    assert_eq!(event.flow_id, flow.id);

    services
        .mark_continuation_dispatched(&owner, flow.id, event.emitted_at)
        .await
        .expect("acknowledge cleanup continuation");
    let retry = services
        .cleanup_for_lifecycle(request)
        .await
        .expect("cleanup retry");
    assert!(retry.canceled_turn_gate_continuations.is_empty());
}
