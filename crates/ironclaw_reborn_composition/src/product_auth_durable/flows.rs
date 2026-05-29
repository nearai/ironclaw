use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, RootFilesystem};

use super::domain::{
    PreparedCallbackFlow, prepare_callback_flow, update_account_from_exchange,
    update_account_from_request, validate_bound_update_authority, validate_callback_claim,
    validate_flow_update_binding, validate_selection_flow,
};
use super::{
    FilesystemAuthProductServices, credential_status_for_completed_flow, is_terminal_status,
    scope_matches,
};
use ironclaw_auth::{
    AuthErrorCode, AuthFlowId, AuthFlowManager, AuthFlowRecord, AuthFlowStatus, AuthProductError,
    CredentialAccountId, CredentialAccountStatus, CredentialOwnership, CredentialSelectionInput,
    NewAuthFlow, NewCredentialAccount, OAuthCallbackClaimRequest, OAuthCallbackFailureInput,
    OAuthCallbackInput, OAuthProviderExchange, ProviderCallbackOutcome,
};

#[async_trait]
impl<F> AuthFlowManager for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn create_flow(&self, request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError> {
        if let Some(binding) = &request.update_binding {
            let account = self
                .read_account(&request.scope, binding.account_id)
                .await?
                .map(|(account, _)| account)
                .ok_or(AuthProductError::CredentialMissing)?;
            validate_flow_update_binding(&account, &request)?;
        }
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
            update_binding: request.update_binding,
            opaque_state_hash: request.opaque_state_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            authorization_code_hash: None,
            error: None,
            continuation_emitted_at: None,
            created_at: now,
            updated_at: now,
            expires_at: request.expires_at,
        };
        self.write_flow(&record.scope, &record, CasExpectation::Absent)
            .await?;
        Ok(record)
    }

    async fn get_flow(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let Some((record, _)) = self.read_flow(scope, flow_id).await? else {
            return Ok(None);
        };
        if !scope_matches(scope, &record.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        Ok(Some(record))
    }

    async fn claim_oauth_callback(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        request: OAuthCallbackClaimRequest,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let lock = self.lock_for(format!("flow:{}", request.flow_id));
        let _guard = lock.lock().await;
        let now = Utc::now();
        let (mut record, version) = self
            .read_flow(scope, request.flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        validate_callback_claim(&mut record, scope, &request, now)?;
        if record.status == AuthFlowStatus::Completed {
            return Ok(record);
        }
        record.status = AuthFlowStatus::CallbackReceived;
        record.updated_at = now;
        self.write_flow(scope, &record, CasExpectation::Version(version))
            .await?;
        Ok(record)
    }

    async fn complete_oauth_callback(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        input: OAuthCallbackInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let lock = self.lock_for(format!("flow:{}", input.flow_id));
        let _guard = lock.lock().await;
        let now = Utc::now();
        let (mut record, version) = self
            .read_flow(scope, input.flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        let callback = prepare_callback_flow(&mut record, scope, &input.opaque_state_hash, now)?;
        let exchange = match input.outcome {
            ProviderCallbackOutcome::Denied => {
                record.status = AuthFlowStatus::Failed;
                record.error = Some(AuthErrorCode::ProviderDenied);
                record.updated_at = now;
                self.write_flow(scope, &record, CasExpectation::Version(version))
                    .await?;
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
        let account_id = self
            .resolve_callback_account(input.flow_id, callback, &exchange)
            .await?;
        record.status = AuthFlowStatus::Completed;
        record.error = None;
        record.authorization_code_hash = Some(exchange.authorization_code_hash);
        record.pkce_verifier_hash = Some(exchange.pkce_verifier_hash);
        record.credential_account_id = Some(account_id);
        record.updated_at = now;
        self.write_flow(scope, &record, CasExpectation::Version(version))
            .await?;
        Ok(record)
    }

    async fn complete_credential_selection(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        input: CredentialSelectionInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let lock = self.lock_for(format!("flow:{}", input.flow_id));
        let _guard = lock.lock().await;
        let now = Utc::now();
        let (mut record, version) = self
            .read_flow(scope, input.flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        validate_selection_flow(&mut record, scope, &input, now)?;
        if record.status == AuthFlowStatus::Completed {
            return Ok(record);
        }
        let account = self
            .read_account(scope, input.credential_account_id)
            .await?
            .map(|(account, _)| account)
            .ok_or(AuthProductError::CredentialMissing)?;
        if !scope_matches(&record.scope, &account.scope)
            || account.provider != record.provider
            || account.status != CredentialAccountStatus::Configured
        {
            return Err(AuthProductError::CrossScopeDenied);
        }
        record.status = AuthFlowStatus::Completed;
        record.error = None;
        record.credential_account_id = Some(input.credential_account_id);
        record.updated_at = now;
        self.write_flow(scope, &record, CasExpectation::Version(version))
            .await?;
        Ok(record)
    }

    async fn fail_oauth_callback(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        input: OAuthCallbackFailureInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let lock = self.lock_for(format!("flow:{}", input.flow_id));
        let _guard = lock.lock().await;
        let now = Utc::now();
        let (mut record, version) = self
            .read_flow(scope, input.flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        prepare_callback_flow(&mut record, scope, &input.opaque_state_hash, now)?;
        record.status = AuthFlowStatus::Failed;
        record.error = Some(input.error);
        record.updated_at = now;
        self.write_flow(scope, &record, CasExpectation::Version(version))
            .await?;
        Ok(record)
    }

    async fn cancel_flow(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let lock = self.lock_for(format!("flow:{flow_id}"));
        let _guard = lock.lock().await;
        let (mut record, version) = self
            .read_flow(scope, flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if !scope_matches(scope, &record.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if is_terminal_status(record.status) {
            return Err(match record.status {
                AuthFlowStatus::Canceled => AuthProductError::Canceled,
                _ => AuthProductError::FlowAlreadyTerminal,
            });
        }
        record.status = AuthFlowStatus::Canceled;
        record.error = Some(AuthErrorCode::Canceled);
        record.updated_at = Utc::now();
        self.write_flow(scope, &record, CasExpectation::Version(version))
            .await?;
        Ok(record)
    }

    async fn mark_continuation_dispatched(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
        emitted_at: ironclaw_auth::Timestamp,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let lock = self.lock_for(format!("flow:{flow_id}"));
        let _guard = lock.lock().await;
        let (mut record, version) = self
            .read_flow(scope, flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if !scope_matches(scope, &record.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if record.status != AuthFlowStatus::Completed {
            return Err(AuthProductError::FlowAlreadyTerminal);
        }
        // Idempotent: if the continuation was already marked by a concurrent
        // caller, return the existing record without writing.
        if record.continuation_emitted_at.is_some() {
            return Ok(record);
        }
        record.continuation_emitted_at = Some(emitted_at);
        record.updated_at = emitted_at;
        self.write_flow(scope, &record, CasExpectation::Version(version))
            .await?;
        Ok(record)
    }
}

impl<F> FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn resolve_callback_account(
        &self,
        flow_id: AuthFlowId,
        callback: PreparedCallbackFlow,
        exchange: &OAuthProviderExchange,
    ) -> Result<CredentialAccountId, AuthProductError> {
        match exchange.account_id {
            Some(account_id) => {
                let binding = callback
                    .update_binding
                    .as_ref()
                    .ok_or(AuthProductError::CrossScopeDenied)?;
                if binding.account_id != account_id {
                    return Err(AuthProductError::CrossScopeDenied);
                }
                let lock = self.lock_for(format!("account:{account_id}"));
                let _guard = lock.lock().await;
                let (mut account, version) = self
                    .read_account(&callback.scope, account_id)
                    .await?
                    .ok_or(AuthProductError::CredentialMissing)?;
                if !scope_matches(&callback.scope, &account.scope) {
                    return Err(AuthProductError::CrossScopeDenied);
                }
                if account.provider != exchange.provider {
                    return Err(AuthProductError::TokenExchangeFailed);
                }
                validate_bound_update_authority(&account, binding)?;
                // Capture previous secret handles before overwriting so we can
                // delete orphaned material from SecretStore after a successful
                // write.  New tokens are written first; a write failure leaves
                // the old handles still referenced by the on-disk record.
                let previous_access_secret = account.access_secret.clone();
                let previous_refresh_secret = account.refresh_secret.clone();
                update_account_from_exchange(&mut account, exchange, Utc::now());
                self.write_account(&account, CasExpectation::Version(version))
                    .await?;
                // Best-effort purge of replaced handles.  Failures are
                // non-fatal: orphans in SecretStore are recoverable; errors
                // must not propagate to the caller.
                if let Some(h) = &previous_access_secret
                    && previous_access_secret.as_ref() != account.access_secret.as_ref()
                {
                    let _ = self.secret_store.delete(&callback.scope.resource, h).await;
                }
                if let Some(h) = &previous_refresh_secret
                    && previous_refresh_secret.as_ref() != account.refresh_secret.as_ref()
                {
                    let _ = self.secret_store.delete(&callback.scope.resource, h).await;
                }
                Ok(account_id)
            }
            None => {
                if callback.update_binding.is_some() {
                    return Err(AuthProductError::CrossScopeDenied);
                }
                let account_id = CredentialAccountId::from_uuid(flow_id.as_uuid());
                let request = NewCredentialAccount {
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
                };
                match self
                    .create_account_with_id(account_id, request.clone(), CasExpectation::Absent)
                    .await
                {
                    Ok(account) => Ok(account.id),
                    // CAS conflict: another concurrent callback already created the account.
                    // Re-read, validate it belongs to this flow/scope/provider, then
                    // overwrite only if identity matches.
                    Err(AuthProductError::BackendConflict) => {
                        let (mut account, version) = self
                            .read_account(&request.scope, account_id)
                            .await?
                            .ok_or(AuthProductError::BackendConflict)?;
                        if !scope_matches(&request.scope, &account.scope) {
                            return Err(AuthProductError::CrossScopeDenied);
                        }
                        if account.provider != request.provider {
                            return Err(AuthProductError::TokenExchangeFailed);
                        }
                        update_account_from_request(&mut account, request, Utc::now())?;
                        self.write_account(&account, CasExpectation::Version(version))
                            .await?;
                        Ok(account.id)
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }
}
