use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, RootFilesystem};

use super::domain::{
    PreparedCallbackFlow, prepare_callback_flow, update_account_from_exchange,
    update_account_from_request, validate_bound_update_authority, validate_callback_claim,
    validate_flow_update_binding, validate_manual_token_flow, validate_selection_flow,
};
use super::{
    FilesystemAuthProductServices, credential_status_for_completed_flow, is_terminal_status,
    scope_matches,
};
use ironclaw_auth::{
    AuthChallenge, AuthErrorCode, AuthFlowId, AuthFlowManager, AuthFlowRecord,
    AuthFlowRecordSource, AuthFlowStatus, AuthProductError, CredentialAccountId,
    CredentialAccountStatus, CredentialOwnership, CredentialSelectionInput,
    ManualTokenCompletionInput, NewAuthFlow, NewCredentialAccount, OAuthCallbackClaimRequest,
    OAuthCallbackFailureInput, OAuthCallbackInput, OAuthProviderExchange, ProviderCallbackOutcome,
    TurnGateAuthFlowQuery, binding_scope_owns_account, flow_matches_turn_gate_query,
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
            id: request.id.unwrap_or_default(),
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
        match validate_callback_claim(&mut record, scope, &request, now) {
            Ok(()) => {}
            Err(AuthProductError::UnknownOrExpiredFlow) => {
                self.write_flow(scope, &record, CasExpectation::Version(version))
                    .await?;
                return Err(AuthProductError::UnknownOrExpiredFlow);
            }
            Err(error) => return Err(error),
        }
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
        let callback =
            match prepare_callback_flow(&mut record, scope, &input.opaque_state_hash, now) {
                Ok(cb) => cb,
                Err(AuthProductError::UnknownOrExpiredFlow) => {
                    self.write_flow(scope, &record, CasExpectation::Version(version))
                        .await?;
                    return Err(AuthProductError::UnknownOrExpiredFlow);
                }
                Err(e) => return Err(e),
            };
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

    async fn complete_manual_token(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        input: ManualTokenCompletionInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let flow_id = self
            .flows_for_scope(scope)
            .await?
            .into_iter()
            .find_map(|(flow, _)| {
                let matches_interaction = matches!(
                    &flow.challenge,
                    Some(AuthChallenge::ManualTokenRequired { interaction_id, .. })
                        if interaction_id == &input.interaction_id
                );
                matches_interaction.then_some(flow.id)
            })
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        let lock = self.lock_for(format!("flow:{flow_id}"));
        let _guard = lock.lock().await;
        let now = Utc::now();
        let (mut record, version) = self
            .read_flow(scope, flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        match validate_manual_token_flow(&mut record, scope, &input, now) {
            Ok(()) => {}
            Err(AuthProductError::UnknownOrExpiredFlow) => {
                self.write_flow(scope, &record, CasExpectation::Version(version))
                    .await?;
                return Err(AuthProductError::UnknownOrExpiredFlow);
            }
            Err(error) => return Err(error),
        }
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

    async fn cancel_manual_token(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        interaction_id: ironclaw_auth::AuthInteractionId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let Some(flow_id) = self
            .flows_for_scope(scope)
            .await?
            .into_iter()
            .find_map(|(flow, _)| {
                let matches_interaction = matches!(
                    &flow.challenge,
                    Some(AuthChallenge::ManualTokenRequired { interaction_id: id, .. })
                        if id == &interaction_id
                );
                matches_interaction.then_some(flow.id)
            })
        else {
            return Ok(None);
        };
        let lock = self.lock_for(format!("flow:{flow_id}"));
        let _guard = lock.lock().await;
        let (mut record, version) = self
            .read_flow(scope, flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if !scope_matches(scope, &record.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if !is_terminal_status(record.status) {
            record.status = AuthFlowStatus::Canceled;
            record.error = Some(AuthErrorCode::Canceled);
            record.updated_at = Utc::now();
            self.write_flow(scope, &record, CasExpectation::Version(version))
                .await?;
        }
        Ok(Some(record))
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
        match prepare_callback_flow(&mut record, scope, &input.opaque_state_hash, now) {
            Ok(_) => {}
            Err(AuthProductError::UnknownOrExpiredFlow) => {
                self.write_flow(scope, &record, CasExpectation::Version(version))
                    .await?;
                return Err(AuthProductError::UnknownOrExpiredFlow);
            }
            Err(e) => return Err(e),
        }
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

#[async_trait]
impl<F> AuthFlowRecordSource for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn flow_for_turn_gate(
        &self,
        query: TurnGateAuthFlowQuery,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        Ok(self
            .flow_records_for_owner(&query.owner)
            .await?
            .into_iter()
            .find(|flow| flow_matches_turn_gate_query(flow, &query)))
    }

    async fn flows_for_owner(
        &self,
        owner: ironclaw_auth::AuthFlowOwnerScope,
    ) -> Result<Vec<AuthFlowRecord>, AuthProductError> {
        self.flow_records_for_owner(&owner).await
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
                self.update_bound_oauth_account(&callback.scope, binding, exchange)
                    .await?;
                Ok(account_id)
            }
            None => {
                if let Some(binding) = &callback.update_binding {
                    return self
                        .update_bound_oauth_account(&callback.scope, binding, exchange)
                        .await;
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
                        let previous_access_secret = account.access_secret.clone();
                        let previous_refresh_secret = account.refresh_secret.clone();
                        update_account_from_request(&mut account, request.clone(), Utc::now())?;
                        self.write_account(&account, CasExpectation::Version(version))
                            .await?;
                        if let Some(h) = &previous_access_secret
                            && previous_access_secret.as_ref() != account.access_secret.as_ref()
                        {
                            let _ = self.secret_store.delete(&request.scope.resource, h).await;
                        }
                        if let Some(h) = &previous_refresh_secret
                            && previous_refresh_secret.as_ref() != account.refresh_secret.as_ref()
                        {
                            let _ = self.secret_store.delete(&request.scope.resource, h).await;
                        }
                        Ok(account.id)
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }

    async fn update_bound_oauth_account(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        binding: &ironclaw_auth::CredentialAccountUpdateBinding,
        exchange: &OAuthProviderExchange,
    ) -> Result<CredentialAccountId, AuthProductError> {
        let account_id = binding.account_id;
        let lock = self.lock_for(format!("account:{account_id}"));
        let _guard = lock.lock().await;
        let (mut account, version) = self
            .read_account(scope, account_id)
            .await?
            .ok_or(AuthProductError::CredentialMissing)?;
        // Owner-granularity guard (#4935 defect A): the callback `scope` is the
        // flow's stored scope, whose per-flow `invocation_id` (and any
        // thread/mission) the bound account does not share. The old
        // `scope_matches` full-equality rejected the legitimate update and left
        // the forked account in place; the owner boundary
        // (tenant/user/agent/project + session) is what must hold here.
        if !binding_scope_owns_account(scope, &account) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if account.provider != exchange.provider {
            return Err(AuthProductError::TokenExchangeFailed);
        }
        validate_bound_update_authority(&account, binding)?;
        // Capture previous secret handles before overwriting so we can delete
        // orphaned material from SecretStore after a successful write. New
        // tokens are written first; a write failure leaves the old handles
        // still referenced by the on-disk record.
        let previous_access_secret = account.access_secret.clone();
        let previous_refresh_secret = account.refresh_secret.clone();
        update_account_from_exchange(&mut account, exchange, Utc::now());
        self.write_account(&account, CasExpectation::Version(version))
            .await?;
        // Best-effort purge of replaced handles. Failures are non-fatal:
        // orphans in SecretStore are recoverable; errors must not propagate to
        // the caller.
        if let Some(h) = &previous_access_secret
            && previous_access_secret.as_ref() != account.access_secret.as_ref()
        {
            let _ = self.secret_store.delete(&scope.resource, h).await;
        }
        if let Some(h) = &previous_refresh_secret
            && previous_refresh_secret.as_ref() != account.refresh_secret.as_ref()
        {
            let _ = self.secret_store.delete(&scope.resource, h).await;
        }
        Ok(account_id)
    }
}
