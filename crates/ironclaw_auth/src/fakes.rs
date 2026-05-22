use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::SecretHandle;

use crate::{
    AuthChallenge, AuthContinuationEvent, AuthFlowId, AuthFlowManager, AuthFlowRecord,
    AuthFlowStatus, AuthInteractionId, AuthInteractionService, AuthProductError,
    AuthProviderClient, AuthProviderId, CredentialAccount, CredentialAccountId,
    CredentialAccountProjection, CredentialAccountService, CredentialAccountStatus,
    CredentialOwnership, CredentialSetupService, ManualTokenSetupRequest, NewAuthFlow,
    NewCredentialAccount, OAuthCallbackInput, OAuthProviderCallbackRequest, OAuthProviderExchange,
    ProviderCallbackOutcome, SecretCleanupAction, SecretCleanupReport, SecretCleanupRequest,
    SecretCleanupService, SecretSubmitRequest, SecretSubmitResult,
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
        let record = AuthFlowRecord {
            id: AuthFlowId::new(),
            scope: request.scope,
            kind: request.kind,
            status: AuthFlowStatus::AwaitingUser,
            provider: request.provider,
            challenge: Some(request.challenge),
            continuation: request.continuation,
            credential_account_id: None,
            opaque_state_hash: request.opaque_state_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            authorization_code_hash: None,
            error: None,
            created_at: now,
            updated_at: now,
            expires_at: request.expires_at,
        };
        self.lock_state().flows.insert(record.id, record.clone());
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
        if !scope_matches(scope, &record.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if now > record.expires_at {
            record.status = AuthFlowStatus::Expired;
            record.error = Some(crate::AuthErrorCode::UnknownOrExpiredFlow);
            record.updated_at = now;
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        if crate::is_terminal_status(record.status) {
            return Err(match record.status {
                AuthFlowStatus::Canceled => AuthProductError::Canceled,
                _ => AuthProductError::FlowAlreadyTerminal,
            });
        }
        if record.opaque_state_hash.as_ref() != Some(&input.opaque_state_hash) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let flow_scope = record.scope.clone();

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
                exchange
            }
        };

        let account_id = match exchange.account_id {
            Some(account_id) => {
                let account = state
                    .accounts
                    .get_mut(&account_id)
                    .ok_or(AuthProductError::CredentialMissing)?;
                if !scope_matches(&flow_scope, &account.scope) {
                    return Err(AuthProductError::CrossScopeDenied);
                }
                if account.provider != exchange.provider {
                    return Err(AuthProductError::TokenExchangeFailed);
                }
                account.label = exchange.account_label.clone();
                account.status = credential_status_for_completed_flow();
                account.access_secret = Some(exchange.access_secret.clone());
                account.refresh_secret = exchange.refresh_secret.clone();
                account.scopes = exchange.scopes.clone();
                account.updated_at = now;
                account_id
            }
            None => {
                create_account_in_state(
                    &mut state,
                    NewCredentialAccount {
                        scope: flow_scope,
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
                .id
            }
        };

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
        scope: &crate::AuthProductScope,
        provider: &AuthProviderId,
    ) -> Result<Vec<CredentialAccountProjection>, AuthProductError> {
        Ok(self
            .lock_state()
            .accounts
            .values()
            .filter(|account| scope_matches(scope, &account.scope) && &account.provider == provider)
            .map(CredentialAccount::projection)
            .collect())
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
        account.status = status;
        account.updated_at = now;
        Ok(account.clone())
    }

    async fn select_unique_configured_account(
        &self,
        scope: &crate::AuthProductScope,
        provider: &AuthProviderId,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        let configured = self
            .lock_state()
            .accounts
            .values()
            .filter(|account| {
                scope_matches(scope, &account.scope)
                    && &account.provider == provider
                    && account.status == CredentialAccountStatus::Configured
            })
            .map(CredentialAccount::projection)
            .collect::<Vec<_>>();
        match configured.as_slice() {
            [] => Err(AuthProductError::CredentialMissing),
            [account] => Ok(account.clone()),
            _ => Err(AuthProductError::AccountSelectionRequired),
        }
    }
}

#[async_trait]
impl CredentialSetupService for InMemoryAuthProductServices {
    async fn create_or_update_account(
        &self,
        request: NewCredentialAccount,
    ) -> Result<CredentialAccount, AuthProductError> {
        let mut state = self.lock_state();
        let existing_id = state
            .accounts
            .values()
            .find(|account| {
                account.scope == request.scope
                    && account.provider == request.provider
                    && account.label == request.label
            })
            .map(|account| account.id);

        if let Some(account_id) = existing_id {
            let now = Utc::now();
            let account = state
                .accounts
                .get_mut(&account_id)
                .ok_or(AuthProductError::CredentialMissing)?;
            return update_account_from_request(account, request, now);
        }

        create_account_in_state(&mut state, request)
    }
}

#[async_trait]
impl AuthInteractionService for InMemoryAuthProductServices {
    async fn request_secret_input(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        let interaction_id = AuthInteractionId::new();
        self.lock_state().interactions.insert(
            interaction_id,
            PendingSecretInteraction {
                scope: request.scope,
                provider: request.provider.clone(),
                label: request.label.clone(),
                continuation: request.continuation,
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
        let account = create_account_in_state(
            &mut state,
            NewCredentialAccount {
                scope: pending.scope,
                provider: pending.provider,
                label: pending.label,
                status: credential_status_for_completed_flow(),
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(generated_secret_handle("manual-access")?),
                refresh_secret: None,
                scopes: Vec::new(),
            },
        )?;
        Ok(SecretSubmitResult {
            account_id: account.id,
            status: account.status,
            continuation: pending.continuation,
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

fn update_account_from_request(
    account: &mut CredentialAccount,
    request: NewCredentialAccount,
    now: crate::Timestamp,
) -> Result<CredentialAccount, AuthProductError> {
    validate_new_credential_account(&request)?;
    account.scope = request.scope;
    account.provider = request.provider;
    account.label = request.label;
    account.status = request.status;
    account.ownership = request.ownership;
    account.owner_extension = request.owner_extension;
    account.granted_extensions = request.granted_extensions;
    account.access_secret = request.access_secret;
    account.refresh_secret = request.refresh_secret;
    account.scopes = request.scopes;
    account.updated_at = now;
    Ok(account.clone())
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

fn generated_secret_handle(prefix: &str) -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!("{prefix}-{}", CredentialAccountId::new()))
        .map_err(|_| AuthProductError::BackendUnavailable)
}
