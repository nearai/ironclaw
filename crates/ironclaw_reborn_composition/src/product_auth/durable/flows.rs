use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, RootFilesystem, cas_update,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use serde::{Deserialize, Serialize};
use std::time::Duration as StdDuration;

use super::domain::{
    PreparedCallbackFlow, prepare_callback_flow, update_account_from_exchange,
    update_account_from_request, validate_bound_update_authority, validate_callback_claim,
    validate_flow_update_binding, validate_manual_token_flow, validate_selection_flow,
};
use super::paths::setup_creation_coordination_path;
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
    is_setup_class_continuation,
};

const SETUP_CREATION_LEASE_SECONDS: i64 = 30;
const SETUP_CREATION_ACQUIRE_TIMEOUT: StdDuration = StdDuration::from_secs(15);
const SETUP_CREATION_OPERATION_TIMEOUT: StdDuration = StdDuration::from_secs(20);
const SETUP_CREATION_POLL_INTERVAL: StdDuration = StdDuration::from_millis(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SetupCreationCoordination {
    holder: AuthFlowId,
    expires_at: ironclaw_auth::Timestamp,
}

fn map_setup_creation_cas_error(error: CasUpdateError<AuthProductError>) -> AuthProductError {
    match error {
        CasUpdateError::Apply(error) => error,
        error => {
            tracing::debug!(
                error = %error,
                "durable setup-flow creation coordination failed"
            );
            AuthProductError::BackendUnavailable
        }
    }
}

impl<F> FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn acquire_setup_creation(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        holder: AuthFlowId,
    ) -> Result<(), AuthProductError> {
        let acquire = async {
            loop {
                let acquired = cas_update(
                    self.filesystem.as_ref(),
                    scope,
                    path,
                    |body| {
                        serde_json::from_slice(body)
                            .map_err(|_| AuthProductError::BackendUnavailable)
                    },
                    |coordination| {
                        serde_json::to_vec(coordination)
                            .map(Entry::bytes)
                            .map(|entry| entry.with_content_type(ContentType::json()))
                            .map_err(|_| AuthProductError::BackendUnavailable)
                    },
                    |current: Option<SetupCreationCoordination>| async move {
                        let now = Utc::now();
                        if let Some(current) = current
                            && current.holder != holder
                            && current.expires_at > now
                        {
                            return Ok::<_, AuthProductError>(CasApply::no_op(current, false));
                        }
                        Ok(CasApply::new(
                            SetupCreationCoordination {
                                holder,
                                expires_at: now + Duration::seconds(SETUP_CREATION_LEASE_SECONDS),
                            },
                            true,
                        ))
                    },
                )
                .await
                .map_err(map_setup_creation_cas_error)?;
                if acquired {
                    return Ok(());
                }
                tokio::time::sleep(SETUP_CREATION_POLL_INTERVAL).await;
            }
        };
        tokio::time::timeout(SETUP_CREATION_ACQUIRE_TIMEOUT, acquire)
            .await
            .map_err(|_| AuthProductError::BackendUnavailable)?
    }

    async fn release_setup_creation(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        holder: AuthFlowId,
    ) {
        let result = cas_update(
            self.filesystem.as_ref(),
            scope,
            path,
            |body| serde_json::from_slice(body).map_err(|_| AuthProductError::BackendUnavailable),
            |coordination| {
                serde_json::to_vec(coordination)
                    .map(Entry::bytes)
                    .map(|entry| entry.with_content_type(ContentType::json()))
                    .map_err(|_| AuthProductError::BackendUnavailable)
            },
            |current: Option<SetupCreationCoordination>| async move {
                let Some(mut current) = current else {
                    return Ok::<_, AuthProductError>(CasApply::no_op(
                        SetupCreationCoordination {
                            holder,
                            expires_at: Utc::now(),
                        },
                        (),
                    ));
                };
                if current.holder != holder {
                    return Ok(CasApply::no_op(current, ()));
                }
                current.expires_at = Utc::now();
                Ok(CasApply::new(current, ()))
            },
        )
        .await;
        if let Err(error) = result {
            tracing::debug!(
                error = %error,
                "failed to release durable setup-flow creation coordination"
            );
        }
    }

    async fn create_flow_after_coordination(
        &self,
        request: NewAuthFlow,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        if is_setup_class_continuation(&request.continuation) {
            self.supersede_setup_flows(&request.scope, &request.provider)
                .await?;
        }
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

    async fn supersede_setup_flows(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        provider: &ironclaw_auth::AuthProviderId,
    ) -> Result<(), AuthProductError> {
        // Setup flows live under the owner+surface+session flow root, keyed by
        // flow id only — thread/mission/invocation are not part of the durable
        // path. Filter to non-terminal setup-class flows so a parked turn-gate
        // flow is never disturbed.
        for (flow, _version) in self.flow_records_under_scope_root(scope).await? {
            if is_terminal_status(flow.status)
                || flow.provider != *provider
                || !is_setup_class_continuation(&flow.continuation)
            {
                continue;
            }
            // Cancel through the flow's own scope: `cancel_flow` re-reads
            // under full-scope equality, while a new setup start has a fresh
            // invocation id.
            match self.cancel_flow(&flow.scope, flow.id).await {
                Ok(_) => {}
                Err(
                    AuthProductError::Canceled
                    | AuthProductError::FlowAlreadyTerminal
                    | AuthProductError::UnknownOrExpiredFlow,
                ) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

#[async_trait]
impl<F> AuthFlowManager for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn create_flow(&self, request: NewAuthFlow) -> Result<AuthFlowRecord, AuthProductError> {
        if !is_setup_class_continuation(&request.continuation) {
            return self.create_flow_after_coordination(request).await;
        }

        let coordination_path =
            setup_creation_coordination_path(&request.scope, &request.provider)?;
        let coordination_scope = request.scope.resource.clone();
        let holder = AuthFlowId::new();
        self.acquire_setup_creation(&coordination_scope, &coordination_path, holder)
            .await?;
        let result = match tokio::time::timeout(
            SETUP_CREATION_OPERATION_TIMEOUT,
            self.create_flow_after_coordination(request),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(AuthProductError::BackendUnavailable),
        };
        self.release_setup_creation(&coordination_scope, &coordination_path, holder)
            .await;
        result
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
        let account_id = self
            .resolve_callback_account(input.flow_id, callback, &exchange)
            .await?;
        record.status = AuthFlowStatus::Completed;
        record.error = None;
        record.authorization_code_hash = Some(exchange.authorization_code_hash);
        record.pkce_verifier_hash = Some(exchange.pkce_verifier_hash);
        record.credential_account_id = Some(account_id);
        record.updated_at = now;
        if let Err(error) = self
            .write_flow(scope, &record, CasExpectation::Version(version))
            .await
        {
            // The exchange already minted/updated the credential account, but
            // the flow's completion write lost a CAS race — e.g. a concurrent
            // lifecycle cancel from extension removal on another replica. A
            // live credential must not outlive its flow: revoke it
            // best-effort (clearing its secret handles) so a torn-down
            // extension cannot retain a token minted mid-removal, then
            // surface the original conflict.
            self.compensate_unanchored_callback_account(scope, account_id)
                .await;
            return Err(error);
        }
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
        if !matches!(
            record.status,
            AuthFlowStatus::Completed
                | AuthFlowStatus::Canceled
                | AuthFlowStatus::Failed
                | AuthFlowStatus::Expired
        ) {
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

    async fn fail_completed_continuation(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
        error: AuthErrorCode,
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
        // Only a completed flow that has not yet acknowledged its continuation
        // can be terminalized by a continuation-dispatch failure. Anything else
        // (already dispatched, or already terminal in another state) must not
        // regress — this races safely against a concurrent completion/dispatch.
        if record.status != AuthFlowStatus::Completed || record.continuation_emitted_at.is_some() {
            return Err(AuthProductError::FlowAlreadyTerminal);
        }
        record.status = AuthFlowStatus::Failed;
        record.error = Some(error);
        record.updated_at = Utc::now();
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
    /// Best-effort compensation for a callback whose account write committed
    /// but whose flow-completion write lost a CAS race — typically a
    /// concurrent lifecycle cancel while an extension is being removed.
    /// Revokes the account and purges its secret handles so the credential
    /// cannot outlive its canceled flow. Failures are logged, never
    /// propagated: the caller surfaces the original conflict, and the
    /// lifecycle cleanup's account scan (which now runs AFTER flow
    /// cancellation) remains the durable backstop.
    async fn compensate_unanchored_callback_account(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        account_id: CredentialAccountId,
    ) {
        let lock = self.lock_for(format!("account:{account_id}"));
        let _guard = lock.lock().await;
        let (mut account, version) = match self.read_account(scope, account_id).await {
            Ok(Some(found)) => found,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    %account_id,
                    error_code = ?error.code(),
                    "callback compensation could not read the just-minted account"
                );
                return;
            }
        };
        let purge_access = account.access_secret.take();
        let purge_refresh = account.refresh_secret.take();
        account.status = CredentialAccountStatus::Revoked;
        account.updated_at = Utc::now();
        if let Err(error) = self
            .write_account(&account, CasExpectation::Version(version))
            .await
        {
            tracing::warn!(
                %account_id,
                error_code = ?error.code(),
                "callback compensation could not revoke the just-minted account"
            );
            return;
        }
        if let Some(handle) = &purge_access {
            self.purge_secret_handle(&account.scope.resource, handle)
                .await;
        }
        if let Some(handle) = &purge_refresh {
            self.purge_secret_handle(&account.scope.resource, handle)
                .await;
        }
    }

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
                    .create_account_with_id_and_provider_identity(
                        account_id,
                        request.clone(),
                        exchange.provider_identity.clone(),
                        CasExpectation::Absent,
                    )
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
                        account.provider_identity = exchange.provider_identity.clone();
                        self.write_account(&account, CasExpectation::Version(version))
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
        // orphaned material from SecretStorePort after a successful write. New
        // tokens are written first; a write failure leaves the old handles
        // still referenced by the on-disk record.
        let previous_access_secret = account.access_secret.clone();
        let previous_refresh_secret = account.refresh_secret.clone();
        update_account_from_exchange(&mut account, exchange, Utc::now());
        self.write_account(&account, CasExpectation::Version(version))
            .await?;
        // Best-effort purge of replaced handles. Failures are non-fatal:
        // orphans in SecretStorePort are recoverable; errors must not propagate to
        // the caller.
        if let Some(h) = &previous_access_secret
            && previous_access_secret.as_ref() != account.access_secret.as_ref()
        {
            self.purge_secret_handle(&scope.resource, h).await;
        }
        if let Some(h) = &previous_refresh_secret
            && previous_refresh_secret.as_ref() != account.refresh_secret.as_ref()
        {
            self.purge_secret_handle(&scope.resource, h).await;
        }
        Ok(account_id)
    }
}
