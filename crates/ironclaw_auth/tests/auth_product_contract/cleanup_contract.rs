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
            lifecycle_package: None,
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
            lifecycle_package: None,
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
            lifecycle_package: None,
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
            lifecycle_package: None,
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
            lifecycle_package: None,
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
            lifecycle_package: None,
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
            lifecycle_package: None,
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
            lifecycle_package: None,
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
            lifecycle_package: None,
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

/// A credential-owner scope carrying the `Callback` surface, no session, and a
/// fresh invocation — the shape a removal/disconnect cleanup actually arrives
/// on (`revoke_exclusive_credentials` / `personal_credential_cleanup_request`
/// build it via `AuthProductScope::credential_owner(.., AuthSurface::Callback)`).
/// It deliberately differs in surface/session/invocation from the `Web`-surface
/// scope [`scope`] mints a pending flow under, so a test that cleans up with it
/// proves cleanup matches flows at credential-owner granularity, not by the
/// surface/session the connect popup happened to use.
fn callback_cleanup_scope(user: &str) -> AuthProductScope {
    let mut cleanup = scope(user);
    cleanup.surface = AuthSurface::Callback;
    cleanup.session_id = None;
    cleanup.resource.invocation_id = InvocationId::new();
    cleanup
}

/// A3 · Removal cancels pending OAuth flows (RFC 9700 §4.7.1 + RFC 7009 §1).
///
/// When an extension is uninstalled mid-handshake, its pending setup flow is
/// canceled even though the removal cleanup arrives on a *different* surface
/// (`Callback`) than the `Web` connect popup that minted the flow; a late
/// provider callback is then rejected, and no credential is minted from the
/// abandoned flow. Re-expressed from `nea25/auth-oauth-parity`'s
/// `uninstall_cancels_pending_flow_and_rejects_late_callback` onto this branch's
/// reconciled auth structure.
#[tokio::test]
async fn uninstall_cancels_pending_flow_and_rejects_late_callback() {
    let services = InMemoryAuthProductServices::new();
    let flow_scope = scope("alice");
    let expires_at = Utc::now() + Duration::minutes(5);

    // A pending setup flow, mid-handshake (AwaitingUser), awaiting the callback,
    // minted under the connect popup's `Web` surface + session.
    let flow = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: flow_scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash("state")),
            pkce_verifier_hash: Some(pkce_hash("pkce")),
            expires_at,
        })
        .await
        .expect("pending flow");
    assert_eq!(flow.status, AuthFlowStatus::AwaitingUser);

    // The extension is uninstalled while the flow is still in flight. Cleanup
    // arrives on the `Callback`-surface credential-owner scope, exactly as
    // `revoke_exclusive_credentials` builds it.
    services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: callback_cleanup_scope("alice"),
            extension_id: ExtensionId::new("github").expect("extension"),
            provider: Some(provider()),
            lifecycle_package: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("cleanup");

    // The pending flow is now canceled and never bound a credential.
    let after = services
        .get_flow(&flow_scope, flow.id)
        .await
        .expect("lookup")
        .expect("record");
    assert_eq!(after.status, AuthFlowStatus::Canceled);
    assert!(after.credential_account_id.is_none());

    // A late provider callback for the removed extension is rejected outright,
    // so no token exchange (and no credential mint) can proceed.
    let late = services
        .claim_oauth_callback(
            &flow_scope,
            ironclaw_auth::OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("state"),
                provider: provider(),
                pkce_verifier_hash: pkce_hash("pkce"),
            },
        )
        .await
        .expect_err("late callback after uninstall must be rejected");
    assert_eq!(late, AuthProductError::Canceled);

    // No credential account exists for the owner+provider.
    let accounts = services
        .list_accounts(CredentialAccountListRequest::new(flow_scope, provider()))
        .await
        .expect("list accounts");
    assert!(accounts.accounts.is_empty());
}

/// A3 breadth (owner decision 2026-07-15): cleanup cancels EVERY non-terminal
/// flow kind for the owner+provider — connect (`SetupOnly`) *and* a blocked-tool
/// auth gate (`TurnGateResume`) — on BOTH `Deactivate` and `Uninstall`, while a
/// different provider's flow and an already-terminal flow are untouched. Any
/// non-terminal flow can otherwise mint a credential on a late callback.
#[tokio::test]
async fn cleanup_cancels_all_pending_flow_kinds_for_provider_on_deactivate() {
    let services = InMemoryAuthProductServices::new();
    let flow_scope = scope("alice");
    let expires_at = Utc::now() + Duration::minutes(5);
    let other_provider = AuthProviderId::new("notion").expect("provider");

    let make_flow = |provider: AuthProviderId, continuation: AuthContinuationRef| {
        let services = &services;
        let flow_scope = flow_scope.clone();
        async move {
            services
                .create_flow(NewAuthFlow {
                    id: None,
                    scope: flow_scope,
                    kind: AuthFlowKind::IntegrationCredential,
                    provider,
                    challenge: AuthChallenge::OAuthUrl {
                        authorization_url: authorization_url("https://provider.example/oauth"),
                        expires_at,
                    },
                    continuation,
                    update_binding: None,
                    opaque_state_hash: Some(state_hash("state")),
                    pkce_verifier_hash: Some(pkce_hash("pkce")),
                    expires_at,
                })
                .await
                .expect("flow")
        }
    };

    let setup = make_flow(provider(), AuthContinuationRef::SetupOnly).await;
    let gate = make_flow(
        provider(),
        AuthContinuationRef::TurnGateResume {
            turn_run_ref: TurnRunRef::new("run-a3").expect("run ref"),
            gate_ref: AuthGateRef::new("gate:a3").expect("gate ref"),
        },
    )
    .await;
    // A different provider's pending flow must survive a provider-scoped cleanup.
    let bystander = make_flow(other_provider, AuthContinuationRef::SetupOnly).await;

    // Deactivate (owner call: same as uninstall for flow cancellation), provider
    // selected, arriving on the callback-surface owner scope.
    let report = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: callback_cleanup_scope("alice"),
            extension_id: ExtensionId::new("github").expect("extension"),
            provider: Some(provider()),
            lifecycle_package: None,
            action: SecretCleanupAction::Deactivate,
        })
        .await
        .expect("cleanup");

    // F2 · Exactly the canceled TURN-GATE flow is handed to the composition
    // layer for gate denial — once, and never the `SetupOnly` connect flow.
    assert_eq!(report.canceled_turn_gate_continuations.len(), 1);
    let event = &report.canceled_turn_gate_continuations[0];
    assert_eq!(event.flow_id, gate.id);
    assert_eq!(event.provider, provider());
    assert_eq!(event.credential_account_id, None);
    assert!(matches!(
        &event.continuation,
        AuthContinuationRef::TurnGateResume { gate_ref, .. } if gate_ref.as_str() == "gate:a3"
    ));

    let status = |id| {
        let services = &services;
        let flow_scope = flow_scope.clone();
        async move {
            services
                .get_flow(&flow_scope, id)
                .await
                .expect("lookup")
                .expect("record")
                .status
        }
    };
    assert_eq!(status(setup.id).await, AuthFlowStatus::Canceled);
    assert_eq!(status(gate.id).await, AuthFlowStatus::Canceled);
    assert_eq!(
        status(bystander.id).await,
        AuthFlowStatus::AwaitingUser,
        "a different provider's pending flow must survive provider-scoped cleanup"
    );

    // Acknowledge the gate-denial handoff exactly as the composition dispatch
    // loop does, so a cleanup retry cannot re-emit the same denial.
    services
        .mark_continuation_dispatched(&event.scope, event.flow_id, event.emitted_at)
        .await
        .expect("cleanup denial acknowledgement supports canceled flows");

    // Idempotent: a second cleanup finds nothing live to cancel, reports no
    // further turn-gate continuations, and still succeeds.
    let retry = services
        .cleanup_for_lifecycle(SecretCleanupRequest {
            scope: callback_cleanup_scope("alice"),
            extension_id: ExtensionId::new("github").expect("extension"),
            provider: Some(provider()),
            lifecycle_package: None,
            action: SecretCleanupAction::Uninstall,
        })
        .await
        .expect("idempotent cleanup");
    assert!(retry.canceled_turn_gate_continuations.is_empty());
    assert_eq!(status(setup.id).await, AuthFlowStatus::Canceled);
}

/// F2 · A flow can reach a terminal state (here: `Completed`) with its
/// `TurnGateResume` continuation still unacknowledged — completion is durable
/// but dispatch is not. Lifecycle cleanup must still synthesize exactly one
/// denial handoff for it (a gate cannot remain parked after its credential is
/// removed), and acknowledging via `mark_continuation_dispatched` converges a
/// retry to an empty report.
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
                turn_run_ref: TurnRunRef::new("run-cleanup-completed").expect("turn run ref"),
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
        lifecycle_package: None,
        action: SecretCleanupAction::Uninstall,
    };
    let report = services
        .cleanup_for_lifecycle(request.clone())
        .await
        .expect("cleanup");
    assert_eq!(report.canceled_turn_gate_continuations.len(), 1);
    let event = &report.canceled_turn_gate_continuations[0];
    assert_eq!(event.flow_id, flow.id);
    // Callback-wins invariant (removal/callback race): a credential minted by
    // a flow that completed before — or raced ahead of — the removal must not
    // survive the cleanup. The account scan runs AFTER flow cancellation, so
    // a mint that beat the cancellation is still swept here.
    assert_eq!(
        report.revoked_accounts,
        vec![account.id],
        "the completed flow's credential is revoked by the same cleanup"
    );

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

#[tokio::test]
async fn expired_unacknowledged_turn_gate_cleanup_emits_once_then_converges() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let flow = services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://example.com/oauth/authorize",
                )
                .unwrap(),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::TurnGateResume {
                turn_run_ref: TurnRunRef::new(uuid::Uuid::new_v4().to_string())
                    .expect("turn run ref"),
                gate_ref: AuthGateRef::new("gate:cleanup-expired").expect("gate ref"),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash("expired-state")),
            pkce_verifier_hash: Some(pkce_hash("expired-pkce")),
            expires_at: Utc::now() - Duration::seconds(1),
        })
        .await
        .expect("flow");
    let expiry_error = services
        .claim_oauth_callback(
            &owner,
            ironclaw_auth::OAuthCallbackClaimRequest {
                flow_id: flow.id,
                opaque_state_hash: state_hash("expired-state"),
                provider: provider(),
                pkce_verifier_hash: pkce_hash("expired-pkce"),
            },
        )
        .await
        .expect_err("expired callback must terminalize the flow");
    assert_eq!(expiry_error, AuthProductError::UnknownOrExpiredFlow);

    let request = SecretCleanupRequest {
        scope: owner.clone(),
        extension_id: ExtensionId::new("github").expect("extension"),
        provider: Some(provider()),
        lifecycle_package: None,
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
        .expect("acknowledge expired cleanup continuation");
    let acknowledged = services
        .get_flow(&owner, flow.id)
        .await
        .expect("expired flow lookup")
        .expect("expired flow remains durable");
    assert_eq!(acknowledged.status, AuthFlowStatus::Expired);
    assert!(acknowledged.continuation_emitted_at.is_some());

    let retry = services
        .cleanup_for_lifecycle(request)
        .await
        .expect("cleanup retry");
    assert!(retry.canceled_turn_gate_continuations.is_empty());
}

/// Uninstall's package selector cancels the removed extension's own
/// `LifecycleActivation` flows even with NO provider selector — the shape the
/// production removal path uses when the provider is shared with another
/// installed extension — while other packages' flows on the same provider
/// survive. Mirrors the durable-store behavior
/// (`product_auth::durable::tests::cleanup_for_lifecycle_cancels_the_removed_packages_flows_despite_shared_provider`).
#[tokio::test]
async fn uninstall_cancels_lifecycle_package_flows_regardless_of_provider() {
    let services = InMemoryAuthProductServices::new();
    let owner = scope("alice");
    let removed_package = LifecyclePackageRef::new("gmail").unwrap();
    let surviving_package = LifecyclePackageRef::new("gdrive").unwrap();
    let expires_at = Utc::now() + Duration::minutes(5);
    let lifecycle_flow = |package: &LifecyclePackageRef, state: &str| NewAuthFlow {
        id: None,
        scope: owner.clone(),
        kind: AuthFlowKind::IntegrationCredential,
        provider: provider(),
        challenge: AuthChallenge::OAuthUrl {
            authorization_url: OAuthAuthorizationUrl::new("https://example.com/oauth/authorize")
                .unwrap(),
            expires_at,
        },
        continuation: AuthContinuationRef::LifecycleActivation {
            package_ref: package.clone(),
        },
        update_binding: None,
        opaque_state_hash: Some(state_hash(state)),
        pkce_verifier_hash: Some(pkce_hash(state)),
        expires_at,
    };
    // `create_flow` allows at most one live setup-class flow per
    // owner+provider (a later creation supersedes the earlier one), so the
    // two halves of the selector invariant are staged sequentially: first the
    // removed package's own live flow dies with the uninstall…
    let removed_flow = services
        .create_flow(lifecycle_flow(&removed_package, "removed-state"))
        .await
        .expect("removed package flow");

    let request = SecretCleanupRequest {
        scope: owner.clone(),
        extension_id: ExtensionId::new("gmail").unwrap(),
        provider: None,
        lifecycle_package: Some(removed_package),
        action: SecretCleanupAction::Uninstall,
    };
    let report = services
        .cleanup_for_lifecycle(request.clone())
        .await
        .expect("package-keyed cleanup");

    assert_eq!(
        services
            .get_flow(&owner, removed_flow.id)
            .await
            .expect("removed flow lookup")
            .expect("removed flow retained")
            .status,
        AuthFlowStatus::Canceled,
        "the removed package's connect flow dies with the extension"
    );

    // …then, with ANOTHER package's flow live on the very same provider, a
    // repeat of the removed package's uninstall must not blanket-cancel the
    // shared provider's flow: the package selector discriminates by package.
    let surviving_flow = services
        .create_flow(lifecycle_flow(&surviving_package, "surviving-state"))
        .await
        .expect("surviving package flow");
    let repeat = services
        .cleanup_for_lifecycle(request.clone())
        .await
        .expect("package-keyed cleanup repeat");
    assert_eq!(
        services
            .get_flow(&owner, surviving_flow.id)
            .await
            .expect("surviving flow lookup")
            .expect("surviving flow retained")
            .status,
        AuthFlowStatus::AwaitingUser,
        "another package's flow on the same provider survives"
    );
    assert!(
        repeat.canceled_flows.is_empty(),
        "the repeat uninstall has nothing of its own package left to cancel"
    );
    assert_eq!(
        report
            .canceled_flows
            .iter()
            .map(|flow| flow.flow_id)
            .collect::<Vec<_>>(),
        vec![removed_flow.id],
        "the report names the canceled flow for eager verifier cleanup"
    );

    let retry = services
        .cleanup_for_lifecycle(request)
        .await
        .expect("cleanup retry");
    assert!(retry.canceled_flows.is_empty(), "cleanup is idempotent");
}
