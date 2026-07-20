use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, RecordVersion, RootFilesystem};
use ironclaw_host_api::ResourceScope;

use super::domain::{
    PreparedCallbackFlow, prepare_callback_flow, update_account_from_exchange,
    update_account_from_request, validate_bound_update_authority, validate_callback_claim,
    validate_flow_update_binding, validate_manual_token_flow, validate_selection_flow,
};
use super::{FilesystemAuthProductServices, credential_status_for_completed_flow, scope_matches};
use ironclaw_auth::{
    AuthChallenge, AuthFlowId, AuthFlowManager, AuthFlowOutcome, AuthFlowRecord,
    AuthFlowRecordSource, AuthFlowState, AuthProductError, CredentialAccount, CredentialAccountId,
    CredentialAccountStatus, CredentialOwnership, CredentialSelectionInput,
    ManualTokenCompletionInput, NewAuthFlow, NewCredentialAccount, OAuthCallbackClaimRequest,
    OAuthCallbackFailureInput, OAuthCallbackInput, OAuthProviderExchange, ProviderCallbackOutcome,
    TurnGateAuthFlowQuery, binding_scope_owns_account, flow_matches_durable_owner,
    flow_matches_turn_gate_query,
};

struct CallbackAccountWrite {
    account: CredentialAccount,
    version: RecordVersion,
    rollback: CallbackAccountRollback,
}

enum CallbackAccountRollback {
    Revoke,
    Restore {
        previous_account: Box<CredentialAccount>,
        cleanup_account_id: CredentialAccountId,
        staged_cleanup: Option<Box<(CredentialAccount, RecordVersion)>>,
        rollback_cleanup_account_id: CredentialAccountId,
    },
}

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
            state: AuthFlowState::Open,
            provider: request.provider,
            challenge: Some(request.challenge),
            continuation: request.continuation,
            credential_secret_fingerprint: None,
            update_binding: request.update_binding,
            opaque_state_hash: request.opaque_state_hash,
            pkce_verifier_hash: request.pkce_verifier_hash,
            authorization_code_hash: None,
            resolution_delivered_at: None,
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
        let claimed = self
            .update_flow_with_cas(scope, request.flow_id, |record| {
                if matches!(record.state, AuthFlowState::Resolved(_)) {
                    return Ok(());
                }
                match validate_callback_claim(record, scope, &request, Utc::now()) {
                    Ok(()) => {}
                    Err(AuthProductError::UnknownOrExpiredFlow)
                        if record.state == AuthFlowState::Resolved(AuthFlowOutcome::Expired) =>
                    {
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                }
                record.state = AuthFlowState::Processing;
                record.updated_at = Utc::now();
                Ok(())
            })
            .await?;
        if claimed.state == AuthFlowState::Resolved(AuthFlowOutcome::Expired) {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        Ok(claimed)
    }
    async fn complete_oauth_callback(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        input: OAuthCallbackInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let (mut record, _) = self
            .read_flow(scope, input.flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if matches!(record.state, AuthFlowState::Resolved(_)) {
            return Ok(record);
        }
        let callback =
            match prepare_callback_flow(&mut record, scope, &input.opaque_state_hash, now) {
                Ok(callback) => callback,
                Err(error) => {
                    if record.state == AuthFlowState::Resolved(AuthFlowOutcome::Expired) {
                        self.persist_expired_flow(scope, input.flow_id, now).await?;
                    }
                    return Err(error);
                }
            };
        let exchange = match input.outcome {
            ProviderCallbackOutcome::Denied => {
                let claimed = self
                    .update_flow_with_cas(scope, input.flow_id, |current| {
                        if matches!(current.state, AuthFlowState::Open) {
                            current.state = AuthFlowState::Processing;
                            current.updated_at = now;
                        }
                        Ok(())
                    })
                    .await?;
                if matches!(claimed.state, AuthFlowState::Resolved(_)) {
                    return Ok(claimed);
                }
                return self
                    .resolve_processing_flow(
                        scope,
                        input.flow_id,
                        AuthFlowOutcome::ProviderDenied,
                        now,
                    )
                    .await;
            }
            ProviderCallbackOutcome::Authorized { exchange } => {
                let exchange = *exchange;
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
        let account_write = self
            .resolve_callback_account(input.flow_id, callback, &exchange)
            .await?;
        let account_id = account_write.account.id;
        let account_fingerprint = account_write.account.secret_fingerprint();
        let outcome = AuthFlowOutcome::Authorized { account_id };
        let resolved = self
            .update_flow_with_cas(scope, input.flow_id, |current| {
                if matches!(current.state, AuthFlowState::Processing) {
                    current.state = AuthFlowState::Resolved(outcome);
                    current.authorization_code_hash =
                        Some(exchange.authorization_code_hash.clone());
                    current.pkce_verifier_hash = Some(exchange.pkce_verifier_hash.clone());
                    current.credential_secret_fingerprint = Some(account_fingerprint.clone());
                    current.updated_at = now;
                }
                Ok(())
            })
            .await;
        match resolved {
            Ok(current) if current.state == AuthFlowState::Resolved(outcome) => {
                self.finalize_callback_account_write(&account_write).await;
                Ok(current)
            }
            Ok(current) => {
                self.rollback_failed_callback_account(account_write).await?;
                Ok(current)
            }
            Err(error) => {
                self.rollback_failed_callback_account(account_write).await?;
                Err(error)
            }
        }
    }

    async fn complete_credential_selection(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        input: CredentialSelectionInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let (mut record, _) = self
            .read_flow(scope, input.flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if let Err(error) = validate_selection_flow(&mut record, scope, &input, now) {
            if record.state == AuthFlowState::Resolved(AuthFlowOutcome::Expired) {
                self.persist_expired_flow(scope, input.flow_id, now).await?;
            }
            return Err(error);
        }
        if matches!(record.state, AuthFlowState::Resolved(_)) {
            return Ok(record);
        }
        let account = self
            .read_account(scope, input.credential_account_id)
            .await?
            .map(|(account, _)| account)
            .ok_or(AuthProductError::CredentialMissing)?;
        // Use owner-granularity for the scope check (#4935 parity with
        // complete_manual_token): the flow record may carry a different
        // invocation_id/thread_id/mission_id than the credential account was
        // originally created with. Full `scope_matches` equality would reject a
        // legitimate cross-invocation selection. The meaningful ownership boundary
        // (tenant/user/agent/project + surface + session) is enforced by
        // `binding_scope_owns_account`; see the canonical docstring at
        // crates/ironclaw_auth/src/credential.rs.
        if !binding_scope_owns_account(&record.scope, &account)
            || account.provider != record.provider
            || account.status != CredentialAccountStatus::Configured
        {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let claimed = self
            .update_flow_with_cas(scope, input.flow_id, |current| {
                match current.state {
                    AuthFlowState::Open => {
                        current.state = AuthFlowState::Processing;
                        current.updated_at = now;
                    }
                    AuthFlowState::Resolved(_) => {}
                    AuthFlowState::Processing => {
                        return Err(AuthProductError::FlowAlreadyTerminal);
                    }
                }
                Ok(())
            })
            .await?;
        if matches!(claimed.state, AuthFlowState::Resolved(_)) {
            return Ok(claimed);
        }
        self.resolve_processing_flow(
            scope,
            input.flow_id,
            AuthFlowOutcome::Authorized {
                account_id: input.credential_account_id,
            },
            Utc::now(),
        )
        .await
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
        let now = Utc::now();
        let (mut record, _) = self
            .read_flow(scope, flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        match validate_manual_token_flow(&mut record, scope, &input, now) {
            Ok(()) => {}
            Err(error) => {
                if record.state == AuthFlowState::Resolved(AuthFlowOutcome::Expired) {
                    self.persist_expired_flow(scope, flow_id, now).await?;
                }
                return Err(error);
            }
        }
        if matches!(record.state, AuthFlowState::Resolved(_)) {
            return Ok(record);
        }
        let account = self
            .read_account(scope, input.credential_account_id)
            .await?
            .map(|(account, _)| account)
            .ok_or(AuthProductError::CredentialMissing)?;
        // Use owner-granularity for the scope check (#4935 defect A, unbound/reusable path):
        // the flow record's scope carries a fresh per-request `invocation_id` (minted
        // by the submit handler for each HTTP call) while the credential account was
        // created under a different `invocation_id`, `thread_id`, or `mission_id` in
        // an earlier flow — all three are ephemeral and intentionally ignored for
        // owner-reusable accounts.  Full `scope_matches` equality would always fail
        // across requests.  The enforced ownership boundary is
        // tenant/user/agent/project + surface + session; see the canonical docstring
        // on `binding_scope_owns_account` at
        // crates/ironclaw_auth/src/credential.rs.
        if !binding_scope_owns_account(&record.scope, &account)
            || account.provider != record.provider
            || account.status != CredentialAccountStatus::Configured
        {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let claimed = self
            .update_flow_with_cas(scope, flow_id, |current| {
                match current.state {
                    AuthFlowState::Open => {
                        current.state = AuthFlowState::Processing;
                        current.updated_at = now;
                    }
                    AuthFlowState::Resolved(_) => {}
                    AuthFlowState::Processing => {
                        return Err(AuthProductError::FlowAlreadyTerminal);
                    }
                }
                Ok(())
            })
            .await?;
        if matches!(claimed.state, AuthFlowState::Resolved(_)) {
            return Ok(claimed);
        }
        self.resolve_processing_flow(
            scope,
            flow_id,
            AuthFlowOutcome::Authorized {
                account_id: input.credential_account_id,
            },
            Utc::now(),
        )
        .await
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
        self.cancel_flow(scope, flow_id).await.map(Some)
    }

    async fn fail_oauth_callback(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        input: OAuthCallbackFailureInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let (mut record, _) = self
            .read_flow(scope, input.flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if matches!(record.state, AuthFlowState::Resolved(_)) {
            return Ok(record);
        }
        match prepare_callback_flow(&mut record, scope, &input.opaque_state_hash, now) {
            Ok(_) => {}
            Err(error) => {
                if record.state == AuthFlowState::Resolved(AuthFlowOutcome::Expired) {
                    self.persist_expired_flow(scope, input.flow_id, now).await?;
                }
                return Err(error);
            }
        }
        let claimed = self
            .update_flow_with_cas(scope, input.flow_id, |current| {
                if matches!(current.state, AuthFlowState::Open) {
                    current.state = AuthFlowState::Processing;
                    current.updated_at = now;
                }
                Ok(())
            })
            .await?;
        if matches!(claimed.state, AuthFlowState::Resolved(_)) {
            return Ok(claimed);
        }
        let outcome = if input.error == ironclaw_auth::AuthErrorCode::ProviderDenied {
            AuthFlowOutcome::ProviderDenied
        } else {
            AuthFlowOutcome::Failed { error: input.error }
        };
        self.resolve_processing_flow(scope, input.flow_id, outcome, now)
            .await
    }

    async fn cancel_flow(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        self.update_flow_with_cas(scope, flow_id, |record| {
            if !matches!(record.state, AuthFlowState::Resolved(_)) {
                record.state = AuthFlowState::Resolved(AuthFlowOutcome::UserAborted);
                record.updated_at = Utc::now();
            }
            Ok(())
        })
        .await
    }

    async fn mark_resolution_delivered(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
        delivered_at: ironclaw_auth::Timestamp,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        self.update_flow_with_cas(scope, flow_id, |record| {
            if !matches!(record.state, AuthFlowState::Resolved(_)) {
                return Err(AuthProductError::FlowAlreadyTerminal);
            }
            if record.resolution_delivered_at.is_none() {
                record.resolution_delivered_at = Some(delivered_at);
                record.updated_at = delivered_at;
            }
            Ok(())
        })
        .await
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

    async fn flow_for_owner_by_id(
        &self,
        owner_scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let resource = ResourceScope {
            tenant_id: owner_scope.resource.tenant_id.clone(),
            user_id: owner_scope.resource.user_id.clone(),
            agent_id: owner_scope.resource.agent_id.clone(),
            project_id: owner_scope.resource.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        Ok(self
            .flow_records_for_resource_filtered(&resource, |flow| {
                flow.id == flow_id && flow_matches_durable_owner(flow, owner_scope)
            })
            .await?
            .into_iter()
            .next())
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
    async fn persist_expired_flow(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
        expired_at: ironclaw_auth::Timestamp,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        self.update_flow_with_cas(scope, flow_id, |record| {
            if !matches!(record.state, AuthFlowState::Resolved(_)) {
                record.state = AuthFlowState::Resolved(AuthFlowOutcome::Expired);
                record.updated_at = expired_at;
            }
            Ok(())
        })
        .await
    }

    async fn resolve_processing_flow(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
        outcome: AuthFlowOutcome,
        resolved_at: ironclaw_auth::Timestamp,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        self.update_flow_with_cas(scope, flow_id, |record| {
            match record.state {
                AuthFlowState::Processing => {
                    record.state = AuthFlowState::Resolved(outcome);
                    record.updated_at = resolved_at;
                }
                AuthFlowState::Resolved(_) => {}
                AuthFlowState::Open => return Err(AuthProductError::FlowAlreadyTerminal),
            }
            Ok(())
        })
        .await
    }

    async fn resolve_callback_account(
        &self,
        flow_id: AuthFlowId,
        callback: PreparedCallbackFlow,
        exchange: &OAuthProviderExchange,
    ) -> Result<CallbackAccountWrite, AuthProductError> {
        match exchange.account_id {
            Some(account_id) => {
                let binding = callback
                    .update_binding
                    .as_ref()
                    .ok_or(AuthProductError::CrossScopeDenied)?;
                if binding.account_id != account_id {
                    return Err(AuthProductError::CrossScopeDenied);
                }
                self.update_bound_oauth_account(flow_id, &callback.scope, binding, exchange)
                    .await
            }
            None => {
                if let Some(binding) = &callback.update_binding {
                    return self
                        .update_bound_oauth_account(flow_id, &callback.scope, binding, exchange)
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
                    .create_account_with_id_and_provider_identity_versioned(
                        account_id,
                        request.clone(),
                        exchange.provider_identity.clone(),
                        CasExpectation::Absent,
                    )
                    .await
                {
                    Ok((account, version)) => Ok(CallbackAccountWrite {
                        account,
                        version,
                        rollback: CallbackAccountRollback::Revoke,
                    }),
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
                        account.provider_identity = exchange.provider_identity.clone();
                        let version = self
                            .write_account(&account, CasExpectation::Version(version))
                            .await?;
                        if let Some(h) = &previous_access_secret
                            && previous_access_secret.as_ref() != account.access_secret.as_ref()
                        {
                            self.purge_secret_handle(&request.scope.resource, h).await;
                        }
                        if let Some(h) = &previous_refresh_secret
                            && previous_refresh_secret.as_ref() != account.refresh_secret.as_ref()
                        {
                            self.purge_secret_handle(&request.scope.resource, h).await;
                        }
                        Ok(CallbackAccountWrite {
                            account,
                            version,
                            rollback: CallbackAccountRollback::Revoke,
                        })
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }

    async fn rollback_failed_callback_account(
        &self,
        callback_write: CallbackAccountWrite,
    ) -> Result<(), AuthProductError> {
        let CallbackAccountWrite {
            account: callback_account,
            version: callback_version,
            rollback,
        } = callback_write;
        let account_id = callback_account.id;
        let lock = self.lock_for(format!("account:{account_id}"));
        let _guard = lock.lock().await;
        let Some((mut account, version)) = self
            .read_account(&callback_account.scope, account_id)
            .await?
        else {
            return Ok(());
        };

        // A later account mutation owns a different version and must not be
        // changed by this stale callback's compensation.
        if version != callback_version || account.status != CredentialAccountStatus::Configured {
            return Ok(());
        }

        if let CallbackAccountRollback::Restore {
            previous_account,
            staged_cleanup,
            rollback_cleanup_account_id,
            ..
        } = rollback
        {
            let previous_account = *previous_account;
            let cleanup_account = self
                .stage_replaced_callback_secrets(
                    rollback_cleanup_account_id,
                    &callback_account,
                    &previous_account,
                )
                .await;
            let mut restore_attempts = 0;
            let restore_result = loop {
                match self
                    .write_account(&previous_account, CasExpectation::Version(version))
                    .await
                {
                    Err(AuthProductError::BackendUnavailable) if restore_attempts < 2 => {
                        restore_attempts += 1;
                    }
                    result => break result,
                }
            };
            match restore_result {
                Ok(_) => {
                    if let Some(staged_cleanup) = staged_cleanup {
                        let (account, version) = *staged_cleanup;
                        self.clear_callback_secret_cleanup(account, version).await?;
                    }
                    return match cleanup_account? {
                        Some((account, version)) => {
                            self.purge_revoked_callback_account(account, version).await
                        }
                        None => Ok(()),
                    };
                }
                Err(AuthProductError::BackendConflict) => {
                    // A later account mutation owns the record. Preserve any
                    // staged cleanup pointers for lifecycle retry; if staging
                    // itself failed, surface that failure to the caller.
                    cleanup_account?;
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }

        account.status = CredentialAccountStatus::Revoked;
        account.updated_at = Utc::now();
        let version = match self
            .write_account(&account, CasExpectation::Version(version))
            .await
        {
            Ok(version) => version,
            // Another process changed the account after our version check. It
            // now owns cleanup or a newer connection; do not clobber it.
            Err(AuthProductError::BackendConflict) => return Ok(()),
            Err(error) => return Err(error),
        };

        self.purge_revoked_callback_account(account, version).await
    }

    async fn stage_replaced_callback_secrets(
        &self,
        cleanup_account_id: CredentialAccountId,
        replaced: &CredentialAccount,
        retained: &CredentialAccount,
    ) -> Result<Option<(CredentialAccount, RecordVersion)>, AuthProductError> {
        let access_secret = (replaced.access_secret != retained.access_secret)
            .then(|| replaced.access_secret.clone())
            .flatten();
        let refresh_secret = (replaced.refresh_secret != retained.refresh_secret)
            .then(|| replaced.refresh_secret.clone())
            .flatten();
        if access_secret.is_none() && refresh_secret.is_none() {
            return Ok(None);
        }
        self.stage_callback_secret_cleanup(
            cleanup_account_id,
            replaced.scope.clone(),
            replaced.provider.clone(),
            replaced.label.clone(),
            access_secret,
            refresh_secret,
        )
        .await
        .map(Some)
    }

    async fn clear_callback_secret_cleanup(
        &self,
        mut account: CredentialAccount,
        version: RecordVersion,
    ) -> Result<(), AuthProductError> {
        if account.status != CredentialAccountStatus::Revoked {
            return Err(AuthProductError::BackendConflict);
        }
        account.access_secret = None;
        account.refresh_secret = None;
        account.updated_at = Utc::now();
        match self
            .write_account(&account, CasExpectation::Version(version))
            .await
        {
            Ok(_) | Err(AuthProductError::BackendConflict) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(super) async fn stage_callback_secret_cleanup(
        &self,
        cleanup_account_id: CredentialAccountId,
        scope: ironclaw_auth::AuthProductScope,
        provider: ironclaw_auth::AuthProviderId,
        label: ironclaw_auth::CredentialAccountLabel,
        access_secret: Option<ironclaw_host_api::SecretHandle>,
        refresh_secret: Option<ironclaw_host_api::SecretHandle>,
    ) -> Result<(CredentialAccount, RecordVersion), AuthProductError> {
        let request = NewCredentialAccount {
            scope: scope.clone(),
            provider: provider.clone(),
            label,
            status: CredentialAccountStatus::Revoked,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: access_secret.clone(),
            refresh_secret: refresh_secret.clone(),
            scopes: Vec::new(),
        };
        match self
            .create_account_with_id_and_provider_identity_versioned(
                cleanup_account_id,
                request,
                None,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(account) => Ok(account),
            Err(AuthProductError::BackendConflict) => {
                let (mut existing, version) = self
                    .read_account(&scope, cleanup_account_id)
                    .await?
                    .ok_or(AuthProductError::BackendConflict)?;
                if existing.provider != provider
                    || existing.status != CredentialAccountStatus::Revoked
                    || !binding_scope_owns_account(&scope, &existing)
                {
                    return Err(AuthProductError::BackendConflict);
                }
                if existing.access_secret == access_secret
                    && existing.refresh_secret == refresh_secret
                {
                    return Ok((existing, version));
                }
                if existing.access_secret.is_some() || existing.refresh_secret.is_some() {
                    return Err(AuthProductError::BackendConflict);
                }
                existing.access_secret = access_secret;
                existing.refresh_secret = refresh_secret;
                existing.updated_at = Utc::now();
                let version = self
                    .write_account(&existing, CasExpectation::Version(version))
                    .await?;
                Ok((existing, version))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn purge_revoked_callback_account(
        &self,
        mut account: CredentialAccount,
        mut version: RecordVersion,
    ) -> Result<(), AuthProductError> {
        let mut delete_failed = false;
        if let Some(handle) = account.access_secret.clone() {
            match self
                .secret_store
                .delete(&account.scope.resource, &handle)
                .await
            {
                Ok(_) => {
                    account.access_secret = None;
                    account.updated_at = Utc::now();
                    version = self
                        .write_account(&account, CasExpectation::Version(version))
                        .await?;
                }
                Err(_) => delete_failed = true,
            }
        }
        if let Some(handle) = account.refresh_secret.clone() {
            match self
                .secret_store
                .delete(&account.scope.resource, &handle)
                .await
            {
                Ok(_) => {
                    account.refresh_secret = None;
                    account.updated_at = Utc::now();
                    self.write_account(&account, CasExpectation::Version(version))
                        .await?;
                }
                Err(_) => delete_failed = true,
            }
        }
        if delete_failed {
            return Err(AuthProductError::BackendUnavailable);
        }
        Ok(())
    }

    async fn finalize_callback_account_write(&self, callback_write: &CallbackAccountWrite) {
        let CallbackAccountRollback::Restore {
            previous_account,
            cleanup_account_id,
            ..
        } = &callback_write.rollback
        else {
            return;
        };
        match self
            .stage_replaced_callback_secrets(
                *cleanup_account_id,
                previous_account,
                &callback_write.account,
            )
            .await
        {
            Ok(Some((account, version))) => {
                if let Err(error) = self.purge_revoked_callback_account(account, version).await {
                    tracing::warn!(
                        cleanup_account_id = %cleanup_account_id,
                        error_code = ?error.code(),
                        "retaining replaced OAuth secrets for lifecycle cleanup retry"
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    cleanup_account_id = %cleanup_account_id,
                    error_code = ?error.code(),
                    "failed to stage replaced OAuth secrets for lifecycle cleanup"
                );
            }
        }
    }

    async fn update_bound_oauth_account(
        &self,
        flow_id: AuthFlowId,
        scope: &ironclaw_auth::AuthProductScope,
        binding: &ironclaw_auth::CredentialAccountUpdateBinding,
        exchange: &OAuthProviderExchange,
    ) -> Result<CallbackAccountWrite, AuthProductError> {
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
        // Preserve the exact prior account until the flow commit succeeds.
        // If cancellation wins, compensation can restore a newer reconnect;
        // if this flow wins, finalization purges the replaced handles.
        let previous_account = account.clone();
        update_account_from_exchange(&mut account, exchange, Utc::now());
        let cleanup_account_id = CredentialAccountId::from_uuid(flow_id.as_uuid());
        let staged_cleanup = self
            .stage_replaced_callback_secrets(cleanup_account_id, &previous_account, &account)
            .await?
            .map(Box::new);
        let version = match self
            .write_account(&account, CasExpectation::Version(version))
            .await
        {
            Ok(version) => version,
            Err(error) => {
                if let Some(staged_cleanup) = staged_cleanup {
                    let (cleanup_account, cleanup_version) = *staged_cleanup;
                    if let Err(clear_error) = self
                        .clear_callback_secret_cleanup(cleanup_account, cleanup_version)
                        .await
                    {
                        tracing::warn!(
                            cleanup_account_id = %cleanup_account_id,
                            error_code = ?clear_error.code(),
                            "failed to clear unused OAuth cleanup pointer after account update failure"
                        );
                    }
                }
                return Err(error);
            }
        };
        Ok(CallbackAccountWrite {
            account,
            version,
            rollback: CallbackAccountRollback::Restore {
                previous_account: Box::new(previous_account),
                cleanup_account_id,
                staged_cleanup,
                rollback_cleanup_account_id: CredentialAccountId::new(),
            },
        })
    }
}
