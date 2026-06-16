use ironclaw_host_api::ExtensionId;

use crate::{
    AuthChallenge, AuthErrorCode, AuthFlowRecord, AuthFlowStatus, AuthProductError,
    CredentialAccount, CredentialAccountUpdateBinding, CredentialOwnership, CredentialRecoveryKind,
    CredentialRecoveryProjection, CredentialRecoveryReason, CredentialRefreshRequest,
    CredentialSelectionInput, ManualTokenCompletionInput, ManualTokenSetupRequest, NewAuthFlow,
    NewCredentialAccount, OAuthCallbackClaimRequest, OAuthProviderExchange, Timestamp,
    flow::credential_status_for_completed_flow, scope_matches,
};

pub struct PreparedCallbackFlow {
    pub scope: crate::AuthProductScope,
    pub update_binding: Option<CredentialAccountUpdateBinding>,
    pub expected_pkce_verifier_hash: Option<crate::PkceVerifierHash>,
}

pub fn prepare_callback_flow(
    record: &mut AuthFlowRecord,
    scope: &crate::AuthProductScope,
    opaque_state_hash: &crate::OpaqueStateHash,
    now: Timestamp,
) -> Result<PreparedCallbackFlow, AuthProductError> {
    if !scope_matches(scope, &record.scope) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if crate::is_terminal_status(record.status) {
        return Err(match record.status {
            AuthFlowStatus::Canceled => AuthProductError::Canceled,
            _ => AuthProductError::FlowAlreadyTerminal,
        });
    }
    expire_if_needed(record, now)?;
    if !record
        .opaque_state_hash
        .as_ref()
        .is_some_and(|expected| expected.constant_time_eq(opaque_state_hash))
    {
        return Err(AuthProductError::CrossScopeDenied);
    }
    Ok(PreparedCallbackFlow {
        scope: record.scope.clone(),
        update_binding: record.update_binding.clone(),
        expected_pkce_verifier_hash: record.pkce_verifier_hash.clone(),
    })
}

pub fn validate_callback_claim(
    record: &mut AuthFlowRecord,
    scope: &crate::AuthProductScope,
    request: &OAuthCallbackClaimRequest,
    now: Timestamp,
) -> Result<(), AuthProductError> {
    if !scope_matches(scope, &record.scope) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if !record
        .opaque_state_hash
        .as_ref()
        .is_some_and(|expected| expected.constant_time_eq(&request.opaque_state_hash))
    {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if record.provider != request.provider {
        return Err(AuthProductError::TokenExchangeFailed);
    }
    if !record
        .pkce_verifier_hash
        .as_ref()
        .is_some_and(|expected| expected.constant_time_eq(&request.pkce_verifier_hash))
    {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if crate::is_terminal_status(record.status) {
        return match record.status {
            AuthFlowStatus::Completed => Ok(()),
            AuthFlowStatus::Canceled => Err(AuthProductError::Canceled),
            _ => Err(AuthProductError::FlowAlreadyTerminal),
        };
    }
    expire_if_needed(record, now)?;
    if record.status != AuthFlowStatus::AwaitingUser {
        return Err(AuthProductError::FlowAlreadyTerminal);
    }
    Ok(())
}

pub fn validate_selection_flow(
    record: &mut AuthFlowRecord,
    scope: &crate::AuthProductScope,
    input: &CredentialSelectionInput,
    now: Timestamp,
) -> Result<(), AuthProductError> {
    if !scope_matches(scope, &record.scope) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if crate::is_terminal_status(record.status) {
        return match (record.status, record.credential_account_id) {
            (AuthFlowStatus::Completed, Some(completed))
                if completed == input.credential_account_id =>
            {
                Ok(())
            }
            (AuthFlowStatus::Canceled, _) => Err(AuthProductError::Canceled),
            _ => Err(AuthProductError::FlowAlreadyTerminal),
        };
    }
    expire_if_needed(record, now)?;
    if record.status != AuthFlowStatus::AwaitingUser {
        return Err(AuthProductError::FlowAlreadyTerminal);
    }
    let Some(AuthChallenge::AccountSelectionRequired { provider, accounts }) = &record.challenge
    else {
        return Err(AuthProductError::invalid_request(
            "auth flow is not awaiting credential selection",
        ));
    };
    if provider != &record.provider {
        return Err(AuthProductError::invalid_request(
            "auth flow credential selection provider mismatch",
        ));
    }
    if !accounts.iter().any(|account| {
        account.id == input.credential_account_id
            && account.provider == record.provider
            && account.status == crate::CredentialAccountStatus::Configured
    }) {
        return Err(AuthProductError::CredentialMissing);
    }
    Ok(())
}

pub fn validate_manual_token_flow(
    record: &mut AuthFlowRecord,
    scope: &crate::AuthProductScope,
    input: &ManualTokenCompletionInput,
    now: Timestamp,
) -> Result<(), AuthProductError> {
    if !scope_matches(scope, &record.scope) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    let Some(AuthChallenge::ManualTokenRequired {
        interaction_id,
        provider,
        ..
    }) = &record.challenge
    else {
        return Err(AuthProductError::invalid_request(
            "auth flow is not awaiting manual token",
        ));
    };
    if interaction_id != &input.interaction_id {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if provider != &record.provider {
        return Err(AuthProductError::invalid_request(
            "auth flow manual-token provider mismatch",
        ));
    }
    if crate::is_terminal_status(record.status) {
        return match (record.status, record.credential_account_id) {
            (AuthFlowStatus::Completed, Some(completed))
                if completed == input.credential_account_id =>
            {
                Ok(())
            }
            (AuthFlowStatus::Canceled, _) => Err(AuthProductError::Canceled),
            _ => Err(AuthProductError::FlowAlreadyTerminal),
        };
    }
    expire_if_needed(record, now)?;
    if record.status != AuthFlowStatus::AwaitingUser {
        return Err(AuthProductError::FlowAlreadyTerminal);
    }
    Ok(())
}

fn expire_if_needed(record: &mut AuthFlowRecord, now: Timestamp) -> Result<(), AuthProductError> {
    if now <= record.expires_at {
        return Ok(());
    }
    record.status = AuthFlowStatus::Expired;
    record.error = Some(AuthErrorCode::UnknownOrExpiredFlow);
    record.updated_at = now;
    Err(AuthProductError::UnknownOrExpiredFlow)
}

pub fn update_account_from_exchange(
    account: &mut CredentialAccount,
    exchange: &OAuthProviderExchange,
    now: Timestamp,
) {
    account.label = exchange.account_label.clone();
    account.status = credential_status_for_completed_flow();
    account.access_secret = Some(exchange.access_secret.clone());
    account.refresh_secret = exchange.refresh_secret.clone();
    account.scopes = exchange.scopes.clone();
    account.updated_at = now;
}

pub fn update_account_from_request(
    account: &mut CredentialAccount,
    request: NewCredentialAccount,
    now: Timestamp,
) -> Result<CredentialAccount, AuthProductError> {
    validate_account_update_target(account, &request)?;
    validate_new_credential_account(&request)?;
    account.label = request.label;
    account.status = request.status;
    account.access_secret = request.access_secret;
    account.refresh_secret = request.refresh_secret;
    account.scopes = request.scopes;
    account.updated_at = now;
    Ok(account.clone())
}

pub fn validate_account_update_target(
    account: &CredentialAccount,
    request: &NewCredentialAccount,
) -> Result<(), AuthProductError> {
    if !scope_matches(&request.scope, &account.scope) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if account.provider != request.provider {
        return Err(AuthProductError::invalid_request(
            "credential account update target provider mismatch",
        ));
    }
    validate_update_authority_fields(
        account,
        request.ownership,
        request.owner_extension.as_ref(),
        &request.granted_extensions,
    )
}

/// Validate that a stored `account` may be updated by a reconnect carrying
/// `binding`, at durable OWNER granularity (#4935 defect A).
///
/// Unlike [`validate_account_update_target`] — which compares the request scope
/// to the account scope with full `scope_matches` equality and is correct for a
/// fresh, same-scope write — this is the *bound reconnect apply* check. The flow
/// / manual-token `scope` carries a fresh per-flow `invocation_id` (and possibly
/// a thread/mission) the account does not share, so the apply step must compare
/// at owner granularity exactly as the setup-time binding validation
/// ([`validate_manual_token_update_binding`] / [`validate_flow_update_binding`])
/// does. Applying `validate_account_update_target` on a bound apply path accepts
/// the binding at setup but then rejects it at apply for every cross-thread
/// reconnect — re-introducing the #4935 fork on the manual-token path.
pub fn validate_bound_account_update_target(
    account: &CredentialAccount,
    scope: &crate::AuthProductScope,
    provider: &crate::AuthProviderId,
    binding: &CredentialAccountUpdateBinding,
) -> Result<(), AuthProductError> {
    validate_scoped_update_binding(
        account,
        scope,
        provider,
        binding,
        "credential account update target provider mismatch",
    )
}

pub fn validate_flow_update_binding(
    account: &CredentialAccount,
    request: &NewAuthFlow,
) -> Result<(), AuthProductError> {
    let Some(binding) = request.update_binding.as_ref() else {
        return Ok(());
    };
    validate_scoped_update_binding(
        account,
        &request.scope,
        &request.provider,
        binding,
        "auth flow update target provider mismatch",
    )
}

pub fn validate_manual_token_update_binding(
    account: &CredentialAccount,
    request: &ManualTokenSetupRequest,
    binding: &CredentialAccountUpdateBinding,
) -> Result<(), AuthProductError> {
    validate_scoped_update_binding(
        account,
        &request.scope,
        &request.provider,
        binding,
        "manual token update target provider mismatch",
    )
}

fn validate_scoped_update_binding(
    account: &CredentialAccount,
    scope: &crate::AuthProductScope,
    provider: &crate::AuthProviderId,
    binding: &CredentialAccountUpdateBinding,
    provider_mismatch: &'static str,
) -> Result<(), AuthProductError> {
    // Validate the bind at durable OWNER granularity (#4935 defect A). The
    // flow / manual-token `scope` carries a fresh per-flow `invocation_id` (and
    // possibly a thread/mission) that the account — created in an earlier flow
    // — does not share. The old `scope_matches` full-equality compared those
    // transient fields and so rejected every legitimate reconnect, forking a
    // duplicate account each time. tenant/user/agent/project stay hard-required
    // via `CredentialAccountOwnerScope`; session_id stays matched; the
    // requester-authority check below is unchanged, so an unauthorized
    // requester still cannot bind another owner's account.
    if !crate::binding_scope_owns_account(scope, account) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if &account.provider != provider {
        return Err(AuthProductError::invalid_request(provider_mismatch));
    }
    validate_bound_update_authority(account, binding)
}

pub fn validate_bound_update_authority(
    account: &CredentialAccount,
    binding: &CredentialAccountUpdateBinding,
) -> Result<(), AuthProductError> {
    validate_update_authority_fields(
        account,
        binding.ownership,
        binding.owner_extension.as_ref(),
        &binding.granted_extensions,
    )
}

fn validate_update_authority_fields(
    account: &CredentialAccount,
    ownership: CredentialOwnership,
    owner_extension: Option<&ExtensionId>,
    granted_extensions: &[ExtensionId],
) -> Result<(), AuthProductError> {
    if account.ownership != ownership
        || account.owner_extension.as_ref() != owner_extension
        || account.granted_extensions.as_slice() != granted_extensions
    {
        return Err(AuthProductError::CrossScopeDenied);
    }
    Ok(())
}

pub fn account_is_authorized_for_requester(
    account: &CredentialAccount,
    requester_extension: Option<&ExtensionId>,
) -> bool {
    account.is_authorized_for_requester(requester_extension)
}

/// Selects the runtime default from duplicate reusable accounts for one provider.
///
/// Runtime credential gates cannot show an account picker, and historical OAuth
/// setup can store the same login under capability-derived labels. When every
/// candidate is reusable, unbound, and configured with an access secret, recency
/// is the setup-time choice signal; mixed ownership still requires explicit
/// account selection.
pub fn select_latest_duplicate_user_reusable_account(
    accounts: &[CredentialAccount],
) -> Option<CredentialAccount> {
    let first = accounts.first()?;
    if !accounts.iter().all(|account| {
        account.provider == first.provider
            && account.status == crate::CredentialAccountStatus::Configured
            && account.ownership == CredentialOwnership::UserReusable
            && account.owner_extension.is_none()
            && account.granted_extensions.is_empty()
            && account.access_secret.is_some()
    }) {
        return None;
    }
    accounts
        .iter()
        .max_by_key(|account| (account.updated_at, account.created_at, account.id))
        .cloned()
}

pub fn validate_new_credential_account(
    request: &NewCredentialAccount,
) -> Result<(), AuthProductError> {
    if request.ownership == CredentialOwnership::ExtensionOwned && request.owner_extension.is_none()
    {
        return Err(AuthProductError::invalid_request(
            "extension-owned credential accounts require owner_extension",
        ));
    }
    Ok(())
}

pub fn validate_credential_status_transition(
    current: crate::CredentialAccountStatus,
    next: crate::CredentialAccountStatus,
) -> Result<(), AuthProductError> {
    if current == next || credential_status_transition_allowed(current, next) {
        return Ok(());
    }
    Err(AuthProductError::invalid_request(
        "credential account status transition is not allowed",
    ))
}

fn credential_status_transition_allowed(
    current: crate::CredentialAccountStatus,
    next: crate::CredentialAccountStatus,
) -> bool {
    use crate::CredentialAccountStatus::{
        Configured, Expired, Inactive, Missing, PendingSetup, RefreshFailed, Revoked,
    };

    match current {
        PendingSetup => matches!(next, Configured | Missing | Expired | Inactive | Revoked),
        Configured => matches!(next, RefreshFailed | Missing | Expired | Inactive | Revoked),
        RefreshFailed => matches!(next, Configured | Missing | Expired | Inactive | Revoked),
        Missing => matches!(next, PendingSetup | Configured | Inactive | Revoked),
        Expired => matches!(next, PendingSetup | Configured | Inactive | Revoked),
        Inactive => matches!(next, PendingSetup | Configured | Revoked),
        Revoked => false,
    }
}

pub fn validate_refresh_target(
    account: &CredentialAccount,
    request: &CredentialRefreshRequest,
) -> Result<(), AuthProductError> {
    if !scope_matches(&request.scope, &account.scope) || account.provider != request.provider {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if !account_is_authorized_for_requester(account, request.requester_extension.as_ref()) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if matches!(
        account.status,
        crate::CredentialAccountStatus::Missing
            | crate::CredentialAccountStatus::PendingSetup
            | crate::CredentialAccountStatus::Inactive
            | crate::CredentialAccountStatus::Revoked
    ) {
        return Err(AuthProductError::CredentialMissing);
    }
    Ok(())
}

pub fn recovery_projection_for_single_account(
    provider: crate::AuthProviderId,
    account: &CredentialAccount,
) -> CredentialRecoveryProjection {
    let (kind, reason) = recovery_kind_and_reason_for_status(account.status);
    match kind {
        CredentialRecoveryKind::Configured => {
            CredentialRecoveryProjection::configured(provider, account.projection())
        }
        CredentialRecoveryKind::SetupRequired => CredentialRecoveryProjection::setup_required(
            provider,
            reason,
            vec![account.projection()],
        ),
        CredentialRecoveryKind::ReauthorizeRequired => {
            CredentialRecoveryProjection::reauthorize_required(
                provider,
                reason,
                vec![account.projection()],
            )
        }
        CredentialRecoveryKind::AccountSelectionRequired => {
            unreachable!("single account recovery cannot produce account selection required")
        }
    }
}

pub fn recovery_projection_for_unconfigured_accounts(
    provider: crate::AuthProviderId,
    accounts: &[&CredentialAccount],
) -> CredentialRecoveryProjection {
    let setup_reason = accounts
        .iter()
        .map(|account| recovery_kind_and_reason_for_status(account.status))
        .find_map(|(kind, reason)| {
            (kind == CredentialRecoveryKind::SetupRequired).then_some(reason)
        });
    let reason = setup_reason.unwrap_or_else(|| {
        accounts
            .iter()
            .map(|account| recovery_kind_and_reason_for_status(account.status).1)
            .next()
            .unwrap_or(CredentialRecoveryReason::NoAccount)
    });
    let choices = accounts
        .iter()
        .map(|account| account.projection())
        .collect::<Vec<_>>();
    if setup_reason.is_some() {
        CredentialRecoveryProjection::setup_required(provider, reason, choices)
    } else {
        CredentialRecoveryProjection::reauthorize_required(provider, reason, choices)
    }
}

fn recovery_kind_and_reason_for_status(
    status: crate::CredentialAccountStatus,
) -> (CredentialRecoveryKind, CredentialRecoveryReason) {
    match status {
        crate::CredentialAccountStatus::Configured => (
            CredentialRecoveryKind::Configured,
            CredentialRecoveryReason::Configured,
        ),
        crate::CredentialAccountStatus::PendingSetup => (
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::PendingSetup,
        ),
        crate::CredentialAccountStatus::Missing => (
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::AccountMissing,
        ),
        crate::CredentialAccountStatus::Inactive => (
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::AccountInactive,
        ),
        crate::CredentialAccountStatus::Expired => (
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::AccountExpired,
        ),
        crate::CredentialAccountStatus::RefreshFailed => (
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::RefreshFailed,
        ),
        crate::CredentialAccountStatus::Revoked => (
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::AccountRevoked,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthProviderId, CredentialAccountId, CredentialAccountLabel, CredentialAccountStatus,
        OAuthAuthorizationCode, OAuthProviderExchange, PkceVerifierSecret, ProviderScope,
    };
    use chrono::Utc;
    use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, UserId};
    use secrecy::SecretString;

    #[test]
    fn update_account_from_exchange_replaces_provider_reported_scopes() {
        let mut account = CredentialAccount {
            id: CredentialAccountId::new(),
            scope: crate::AuthProductScope::new(
                ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                    .unwrap(),
                crate::AuthSurface::Api,
            ),
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("github").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("old-access").unwrap()),
            refresh_secret: Some(SecretHandle::new("old-refresh").unwrap()),
            scopes: vec![
                ProviderScope::new("repo").unwrap(),
                ProviderScope::new("admin:org").unwrap(),
            ],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let exchange = OAuthProviderExchange {
            provider: AuthProviderId::new("github").unwrap(),
            account_label: CredentialAccountLabel::new("github").unwrap(),
            authorization_code_hash: crate::authorization_code_hash(
                &OAuthAuthorizationCode::new(SecretString::from("code")).unwrap(),
            )
            .unwrap(),
            pkce_verifier_hash: crate::pkce_verifier_hash(
                &PkceVerifierSecret::new(SecretString::from("pkce")).unwrap(),
            )
            .unwrap(),
            access_secret: SecretHandle::new("new-access").unwrap(),
            refresh_secret: Some(SecretHandle::new("new-refresh").unwrap()),
            scopes: vec![ProviderScope::new("repo").unwrap()],
            account_id: None,
        };

        update_account_from_exchange(&mut account, &exchange, Utc::now());

        assert_eq!(
            account
                .scopes
                .iter()
                .map(|scope| scope.as_str())
                .collect::<Vec<_>>(),
            vec!["repo"]
        );
    }
}
