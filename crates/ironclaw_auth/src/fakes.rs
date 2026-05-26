use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{ExtensionId, SecretHandle};

use crate::{
    AuthChallenge, AuthContinuationEvent, AuthFlowId, AuthFlowManager, AuthFlowRecord,
    AuthFlowStatus, AuthInteractionId, AuthInteractionService, AuthProductError,
    AuthProviderClient, CredentialAccount, CredentialAccountId, CredentialAccountListPage,
    CredentialAccountListRequest, CredentialAccountMutation, CredentialAccountProjection,
    CredentialAccountSelectionRequest, CredentialAccountService, CredentialAccountStatus,
    CredentialAccountUpdateBinding, CredentialOwnership, CredentialSetupService,
    ManualTokenSetupRequest, NewAuthFlow, NewCredentialAccount, OAuthCallbackClaimRequest,
    OAuthCallbackFailureInput, OAuthCallbackInput, OAuthProviderCallbackRequest,
    OAuthProviderExchange, ProviderCallbackOutcome, SecretCleanupAction, SecretCleanupReport,
    SecretCleanupRequest, SecretCleanupService, SecretSubmitRequest, SecretSubmitResult,
    cleanup::SecretCleanupAction::Deactivate, flow::credential_status_for_completed_flow,
    interaction::PendingSecretInteraction, provider::validate_provider_callback_request,
    scope_matches,
};

#[derive(Default)]
struct AuthState {
    flows: HashMap<AuthFlowId, AuthFlowRecord>,
    interactions: HashMap<AuthInteractionId, PendingSecretInteraction>,
    accounts: HashMap<CredentialAccountId, CredentialAccount>,
    continuations: Vec<AuthContinuationEvent>,
}

/// In-memory fake implementation of all product-auth service ports.
///
/// This is test support, not production persistence. It intentionally models
/// important fail-closed transitions so downstream code cannot depend on unsafe
/// shortcuts while production stores are still being composed.
#[derive(Default)]
pub struct InMemoryAuthProductServices {
    state: Mutex<AuthState>,
}

impl InMemoryAuthProductServices {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn continuations(&self) -> Vec<AuthContinuationEvent> {
        self.lock_state().continuations.clone()
    }

    fn lock_state(&self) -> MutexGuard<'_, AuthState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl AuthFlowManager for InMemoryAuthProductServices {
    async fn create_flow(&self, request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let mut state = self.lock_state();
        if let Some(binding) = &request.update_binding {
            let account = state
                .accounts
                .get(&binding.account_id)
                .ok_or(AuthProductError::CredentialMissing)?;
            validate_flow_update_binding(account, &request)?;
        }
        let record = AuthFlowRecord {
            id: AuthFlowId::new(),
            scope: request.scope,
            kind: request.kind,
            status: AuthFlowStatus::AwaitingUser,
            provider: request.provider,
            challenge: Some(request.challenge),
            continuation: request.continuation,
            credential_account_id: None,
            update_binding: request.update_binding,
            opaque_state_hash: request.opaque_state_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            authorization_code_hash: None,
            error: None,
            created_at: now,
            updated_at: now,
            expires_at: request.expires_at,
        };
        state.flows.insert(record.id, record.clone());
        Ok(record)
    }

    async fn get_flow(
        &self,
        scope: &crate::AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let state = self.lock_state();
        let Some(record) = state.flows.get(&flow_id) else {
            return Ok(None);
        };
        if !scope_matches(scope, &record.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        Ok(Some(record.clone()))
    }

    async fn claim_oauth_callback(
        &self,
        scope: &crate::AuthProductScope,
        request: OAuthCallbackClaimRequest,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let mut state = self.lock_state();
        let record = state
            .flows
            .get_mut(&request.flow_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
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
                AuthFlowStatus::Completed => Ok(record.clone()),
                AuthFlowStatus::Canceled => Err(AuthProductError::Canceled),
                _ => Err(AuthProductError::FlowAlreadyTerminal),
            };
        }
        if now > record.expires_at {
            record.status = AuthFlowStatus::Expired;
            record.error = Some(crate::AuthErrorCode::UnknownOrExpiredFlow);
            record.updated_at = now;
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        if record.status != AuthFlowStatus::AwaitingUser {
            return Err(AuthProductError::FlowAlreadyTerminal);
        }
        record.status = AuthFlowStatus::CallbackReceived;
        record.updated_at = now;
        Ok(record.clone())
    }

    async fn complete_oauth_callback(
        &self,
        scope: &crate::AuthProductScope,
        input: OAuthCallbackInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let mut state = self.lock_state();
        let record = state
            .flows
            .get_mut(&input.flow_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        let callback = prepare_callback_flow(record, scope, &input.opaque_state_hash, now)?;

        let exchange = match input.outcome {
            ProviderCallbackOutcome::Denied => {
                record.status = AuthFlowStatus::Failed;
                record.error = Some(crate::AuthErrorCode::ProviderDenied);
                record.updated_at = now;
                return Err(AuthProductError::ProviderDenied);
            }
            ProviderCallbackOutcome::Authorized { exchange } => {
                if exchange.provider != record.provider {
                    return Err(AuthProductError::TokenExchangeFailed);
                }
                if !callback
                    .expected_pkce_verifier_hash
                    .as_ref()
                    .is_some_and(|expected| expected.constant_time_eq(&exchange.pkce_verifier_hash))
                {
                    return Err(AuthProductError::CrossScopeDenied);
                }
                exchange
            }
        };

        let account_id = resolve_callback_account(&mut state, callback, &exchange, now)?;

        let record = state
            .flows
            .get_mut(&input.flow_id)
            .ok_or(AuthProductError::BackendUnavailable)?;
        record.status = AuthFlowStatus::Completed;
        record.error = None;
        record.authorization_code_hash = Some(exchange.authorization_code_hash);
        record.pkce_verifier_hash = Some(exchange.pkce_verifier_hash);
        record.credential_account_id = Some(account_id);
        record.updated_at = now;
        let completed = record.clone();
        state.continuations.push(AuthContinuationEvent {
            flow_id: completed.id,
            scope: completed.scope.clone(),
            continuation: completed.continuation.clone(),
            credential_account_id: completed.credential_account_id,
            emitted_at: now,
        });
        Ok(completed)
    }

    async fn fail_oauth_callback(
        &self,
        scope: &crate::AuthProductScope,
        input: OAuthCallbackFailureInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let mut state = self.lock_state();
        let record = state
            .flows
            .get_mut(&input.flow_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        let _callback = prepare_callback_flow(record, scope, &input.opaque_state_hash, now)?;
        record.status = AuthFlowStatus::Failed;
        record.error = Some(input.error);
        record.updated_at = now;
        Ok(record.clone())
    }

    async fn cancel_flow(
        &self,
        scope: &crate::AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let mut state = self.lock_state();
        let record = state
            .flows
            .get_mut(&flow_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if !scope_matches(scope, &record.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if crate::is_terminal_status(record.status) {
            return Err(match record.status {
                AuthFlowStatus::Canceled => AuthProductError::Canceled,
                _ => AuthProductError::FlowAlreadyTerminal,
            });
        }
        record.status = AuthFlowStatus::Canceled;
        record.error = Some(crate::AuthErrorCode::Canceled);
        record.updated_at = now;
        Ok(record.clone())
    }
}

#[async_trait]
impl CredentialAccountService for InMemoryAuthProductServices {
    async fn create_account(
        &self,
        request: NewCredentialAccount,
    ) -> Result<CredentialAccount, AuthProductError> {
        create_account_in_state(&mut self.lock_state(), request)
    }

    async fn get_account(
        &self,
        scope: &crate::AuthProductScope,
        account_id: CredentialAccountId,
    ) -> Result<Option<CredentialAccount>, AuthProductError> {
        let state = self.lock_state();
        let Some(account) = state.accounts.get(&account_id) else {
            return Ok(None);
        };
        if !scope_matches(scope, &account.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        Ok(Some(account.clone()))
    }

    async fn list_accounts(
        &self,
        request: CredentialAccountListRequest,
    ) -> Result<CredentialAccountListPage, AuthProductError> {
        request.validate()?;
        let mut accounts = self
            .lock_state()
            .accounts
            .values()
            .filter(|account| {
                scope_matches(&request.scope, &account.scope)
                    && account.provider == request.provider
                    && request.cursor.is_none_or(|cursor| account.id > cursor)
            })
            .map(CredentialAccount::projection)
            .collect::<Vec<_>>();
        accounts.sort_by_key(|account| account.id);
        let next_cursor = if accounts.len() > request.limit {
            accounts.truncate(request.limit);
            accounts.last().map(|account| account.id)
        } else {
            None
        };
        Ok(CredentialAccountListPage {
            accounts,
            next_cursor,
        })
    }

    async fn update_status(
        &self,
        scope: &crate::AuthProductScope,
        account_id: CredentialAccountId,
        status: CredentialAccountStatus,
    ) -> Result<CredentialAccount, AuthProductError> {
        let now = Utc::now();
        let mut state = self.lock_state();
        let account = state
            .accounts
            .get_mut(&account_id)
            .ok_or(AuthProductError::CredentialMissing)?;
        if !scope_matches(scope, &account.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        validate_credential_status_transition(account.status, status)?;
        account.status = status;
        account.updated_at = now;
        Ok(account.clone())
    }

    async fn select_unique_configured_account(
        &self,
        request: CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        let state = self.lock_state();
        let configured = state
            .accounts
            .values()
            .filter(|account| {
                scope_matches(&request.scope, &account.scope)
                    && account.provider == request.provider
                    && account.status == CredentialAccountStatus::Configured
            })
            .collect::<Vec<_>>();
        if configured.is_empty() {
            return Err(AuthProductError::CredentialMissing);
        }
        let selectable = configured
            .iter()
            .copied()
            .filter(|account| account_is_selectable_for_requester(account, &request))
            .map(CredentialAccount::projection)
            .collect::<Vec<_>>();
        match selectable.as_slice() {
            [] => Err(AuthProductError::CrossScopeDenied),
            [account] => Ok(account.clone()),
            _ => Err(AuthProductError::AccountSelectionRequired),
        }
    }
}

#[async_trait]
impl CredentialSetupService for InMemoryAuthProductServices {
    async fn create_or_update_account(
        &self,
        request: CredentialAccountMutation,
    ) -> Result<CredentialAccount, AuthProductError> {
        let mut state = self.lock_state();
        match request {
            CredentialAccountMutation::Create(account) => {
                create_account_in_state(&mut state, account)
            }
            CredentialAccountMutation::Update(update) => {
                let now = Utc::now();
                let account = state
                    .accounts
                    .get_mut(&update.account_id)
                    .ok_or(AuthProductError::CredentialMissing)?;
                validate_account_update_target(account, &update.account)?;
                update_account_from_request(account, update.account, now)
            }
        }
    }
}

#[async_trait]
impl AuthInteractionService for InMemoryAuthProductServices {
    async fn request_secret_input(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        let interaction_id = AuthInteractionId::new();
        let mut state = self.lock_state();
        if let Some(binding) = &request.update_binding {
            let account = state
                .accounts
                .get(&binding.account_id)
                .ok_or(AuthProductError::CredentialMissing)?;
            validate_manual_token_update_binding(account, &request, binding)?;
        }
        state.interactions.insert(
            interaction_id,
            PendingSecretInteraction {
                scope: request.scope,
                provider: request.provider.clone(),
                label: request.label.clone(),
                continuation: request.continuation,
                update_binding: request.update_binding,
                expires_at: request.expires_at,
            },
        );
        Ok(AuthChallenge::ManualTokenRequired {
            interaction_id,
            provider: request.provider,
            label: request.label,
            expires_at: request.expires_at,
        })
    }

    async fn submit_manual_token(
        &self,
        scope: &crate::AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        request.validate_secret()?;
        let now = Utc::now();
        let mut state = self.lock_state();
        let pending = state
            .interactions
            .get(&request.interaction_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if !scope_matches(scope, &pending.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if now > pending.expires_at {
            state.interactions.remove(&request.interaction_id);
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        let pending = state
            .interactions
            .remove(&request.interaction_id)
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        let continuation = pending.continuation.clone();
        let account = create_or_update_manual_token_account(&mut state, pending)?;
        Ok(SecretSubmitResult {
            account_id: account.id,
            status: account.status,
            continuation,
        })
    }
}

#[async_trait]
impl AuthProviderClient for InMemoryAuthProductServices {
    async fn exchange_callback(
        &self,
        request: OAuthProviderCallbackRequest,
    ) -> Result<OAuthProviderExchange, AuthProductError> {
        validate_provider_callback_request(&request)?;
        Ok(OAuthProviderExchange {
            provider: request.provider,
            account_label: request.account_label,
            authorization_code_hash: request.authorization_code_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            access_secret: generated_secret_handle("oauth-access")?,
            refresh_secret: Some(generated_secret_handle("oauth-refresh")?),
            scopes: request.scopes,
            account_id: None,
        })
    }
}

#[async_trait]
impl SecretCleanupService for InMemoryAuthProductServices {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError> {
        let mut state = self.lock_state();
        let mut report = SecretCleanupReport::default();
        for account in state.accounts.values_mut() {
            if !scope_matches(&request.scope, &account.scope) {
                continue;
            }
            let had_grant = account
                .granted_extensions
                .iter()
                .any(|extension| extension == &request.extension_id);
            account
                .granted_extensions
                .retain(|extension| extension != &request.extension_id);
            if had_grant {
                report.removed_grants.push(account.id);
            }

            if account.owner_extension.as_ref() == Some(&request.extension_id)
                && account.ownership == CredentialOwnership::ExtensionOwned
            {
                match request.action {
                    Deactivate => {
                        account.status = CredentialAccountStatus::Inactive;
                        account.updated_at = Utc::now();
                        report.retained_accounts.push(account.id);
                    }
                    SecretCleanupAction::Uninstall => {
                        if account.status != CredentialAccountStatus::Revoked {
                            account.status = CredentialAccountStatus::Revoked;
                            account.updated_at = Utc::now();
                            report.revoked_accounts.push(account.id);
                        }
                    }
                }
            } else if had_grant {
                report.retained_accounts.push(account.id);
            }
        }
        Ok(report)
    }
}

fn create_account_in_state(
    state: &mut AuthState,
    request: NewCredentialAccount,
) -> Result<CredentialAccount, AuthProductError> {
    validate_new_credential_account(&request)?;
    let now = Utc::now();
    let account = CredentialAccount {
        id: CredentialAccountId::new(),
        scope: request.scope,
        provider: request.provider,
        label: request.label,
        status: request.status,
        ownership: request.ownership,
        owner_extension: request.owner_extension,
        granted_extensions: request.granted_extensions,
        access_secret: request.access_secret,
        refresh_secret: request.refresh_secret,
        scopes: request.scopes,
        created_at: now,
        updated_at: now,
    };
    state.accounts.insert(account.id, account.clone());
    Ok(account)
}

struct PreparedCallbackFlow {
    scope: crate::AuthProductScope,
    update_binding: Option<crate::CredentialAccountUpdateBinding>,
    expected_pkce_verifier_hash: Option<crate::PkceVerifierHash>,
}

fn prepare_callback_flow(
    record: &mut AuthFlowRecord,
    scope: &crate::AuthProductScope,
    opaque_state_hash: &crate::OpaqueStateHash,
    now: crate::Timestamp,
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
    if now > record.expires_at {
        record.status = AuthFlowStatus::Expired;
        record.error = Some(crate::AuthErrorCode::UnknownOrExpiredFlow);
        record.updated_at = now;
        return Err(AuthProductError::UnknownOrExpiredFlow);
    }
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

fn resolve_callback_account(
    state: &mut AuthState,
    callback: PreparedCallbackFlow,
    exchange: &OAuthProviderExchange,
    now: crate::Timestamp,
) -> Result<CredentialAccountId, AuthProductError> {
    match exchange.account_id {
        Some(account_id) => {
            update_bound_callback_account(state, callback, exchange, account_id, now)
        }
        None => create_callback_account(state, callback, exchange),
    }
}

fn update_bound_callback_account(
    state: &mut AuthState,
    callback: PreparedCallbackFlow,
    exchange: &OAuthProviderExchange,
    account_id: CredentialAccountId,
    now: crate::Timestamp,
) -> Result<CredentialAccountId, AuthProductError> {
    let Some(binding) = callback.update_binding.as_ref() else {
        return Err(AuthProductError::CrossScopeDenied);
    };
    if binding.account_id != account_id {
        return Err(AuthProductError::CrossScopeDenied);
    }
    let account = state
        .accounts
        .get_mut(&account_id)
        .ok_or(AuthProductError::CredentialMissing)?;
    if !scope_matches(&callback.scope, &account.scope) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if account.provider != exchange.provider {
        return Err(AuthProductError::TokenExchangeFailed);
    }
    validate_bound_update_authority(account, binding)?;
    update_account_from_exchange(account, exchange, now);
    Ok(account_id)
}

fn create_callback_account(
    state: &mut AuthState,
    callback: PreparedCallbackFlow,
    exchange: &OAuthProviderExchange,
) -> Result<CredentialAccountId, AuthProductError> {
    if callback.update_binding.is_some() {
        return Err(AuthProductError::CrossScopeDenied);
    }
    Ok(create_account_in_state(
        state,
        NewCredentialAccount {
            scope: callback.scope,
            provider: exchange.provider.clone(),
            label: exchange.account_label.clone(),
            status: credential_status_for_completed_flow(),
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(exchange.access_secret.clone()),
            refresh_secret: exchange.refresh_secret.clone(),
            scopes: exchange.scopes.clone(),
        },
    )?
    .id)
}

fn update_account_from_request(
    account: &mut CredentialAccount,
    request: NewCredentialAccount,
    now: crate::Timestamp,
) -> Result<CredentialAccount, AuthProductError> {
    validate_new_credential_account(&request)?;
    account.label = request.label;
    account.status = request.status;
    account.access_secret = request.access_secret;
    account.refresh_secret = request.refresh_secret;
    account.scopes = request.scopes;
    account.updated_at = now;
    Ok(account.clone())
}

fn create_or_update_manual_token_account(
    state: &mut AuthState,
    pending: PendingSecretInteraction,
) -> Result<CredentialAccount, AuthProductError> {
    match pending.update_binding.as_ref() {
        Some(binding) => {
            let account_request = manual_token_account_request(
                &pending,
                binding.ownership,
                binding.owner_extension.clone(),
                binding.granted_extensions.clone(),
            )?;
            let now = Utc::now();
            let account = state
                .accounts
                .get_mut(&binding.account_id)
                .ok_or(AuthProductError::CredentialMissing)?;
            validate_account_update_target(account, &account_request)?;
            update_account_from_request(account, account_request, now)
        }
        None => create_account_in_state(
            state,
            manual_token_account_request(
                &pending,
                CredentialOwnership::UserReusable,
                None,
                Vec::new(),
            )?,
        ),
    }
}

fn manual_token_account_request(
    pending: &PendingSecretInteraction,
    ownership: CredentialOwnership,
    owner_extension: Option<ExtensionId>,
    granted_extensions: Vec<ExtensionId>,
) -> Result<NewCredentialAccount, AuthProductError> {
    Ok(NewCredentialAccount {
        scope: pending.scope.clone(),
        provider: pending.provider.clone(),
        label: pending.label.clone(),
        status: credential_status_for_completed_flow(),
        ownership,
        owner_extension,
        granted_extensions,
        access_secret: Some(generated_secret_handle("manual-access")?),
        refresh_secret: None,
        scopes: Vec::new(),
    })
}

fn update_account_from_exchange(
    account: &mut CredentialAccount,
    exchange: &OAuthProviderExchange,
    now: crate::Timestamp,
) {
    account.label = exchange.account_label.clone();
    account.status = credential_status_for_completed_flow();
    account.access_secret = Some(exchange.access_secret.clone());
    account.refresh_secret = exchange.refresh_secret.clone();
    account.scopes = exchange.scopes.clone();
    account.updated_at = now;
}

fn validate_account_update_target(
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

fn validate_flow_update_binding(
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
        UpdateBindingValidationContext::AuthFlow,
    )
}

fn validate_manual_token_update_binding(
    account: &CredentialAccount,
    request: &ManualTokenSetupRequest,
    binding: &CredentialAccountUpdateBinding,
) -> Result<(), AuthProductError> {
    validate_scoped_update_binding(
        account,
        &request.scope,
        &request.provider,
        binding,
        UpdateBindingValidationContext::ManualToken,
    )
}

#[derive(Debug, Clone, Copy)]
enum UpdateBindingValidationContext {
    AuthFlow,
    ManualToken,
}

fn validate_scoped_update_binding(
    account: &CredentialAccount,
    scope: &crate::AuthProductScope,
    provider: &crate::AuthProviderId,
    binding: &CredentialAccountUpdateBinding,
    context: UpdateBindingValidationContext,
) -> Result<(), AuthProductError> {
    if !scope_matches(scope, &account.scope) {
        return Err(AuthProductError::CrossScopeDenied);
    }
    if &account.provider != provider {
        return Err(AuthProductError::invalid_request(match context {
            UpdateBindingValidationContext::AuthFlow => "auth flow update target provider mismatch",
            UpdateBindingValidationContext::ManualToken => {
                "manual token update target provider mismatch"
            }
        }));
    }
    validate_bound_update_authority(account, binding)
}

fn validate_bound_update_authority(
    account: &CredentialAccount,
    binding: &crate::CredentialAccountUpdateBinding,
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

fn account_is_selectable_for_requester(
    account: &CredentialAccount,
    request: &CredentialAccountSelectionRequest,
) -> bool {
    match account.ownership {
        CredentialOwnership::UserReusable => true,
        CredentialOwnership::ExtensionOwned => {
            account
                .owner_extension
                .as_ref()
                .is_some_and(|owner_extension| {
                    request.requester_extension.as_ref() == Some(owner_extension)
                })
        }
        CredentialOwnership::SharedAdminManaged => request
            .requester_extension
            .as_ref()
            .is_some_and(|requester| account.granted_extensions.contains(requester)),
        CredentialOwnership::System => false,
    }
}

fn validate_new_credential_account(request: &NewCredentialAccount) -> Result<(), AuthProductError> {
    if request.ownership == CredentialOwnership::ExtensionOwned && request.owner_extension.is_none()
    {
        return Err(AuthProductError::invalid_request(
            "extension-owned credential accounts require owner_extension",
        ));
    }
    Ok(())
}

fn validate_credential_status_transition(
    current: CredentialAccountStatus,
    next: CredentialAccountStatus,
) -> Result<(), AuthProductError> {
    if current == next || credential_status_transition_allowed(current, next) {
        return Ok(());
    }
    Err(AuthProductError::invalid_request(
        "credential account status transition is not allowed",
    ))
}

fn credential_status_transition_allowed(
    current: CredentialAccountStatus,
    next: CredentialAccountStatus,
) -> bool {
    use CredentialAccountStatus::{
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

fn generated_secret_handle(prefix: &str) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("{prefix}-{}", CredentialAccountId::new()))
        .map_err(|_| AuthProductError::BackendUnavailable)
}
