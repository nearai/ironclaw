use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, RootFilesystem};

use super::FilesystemAuthProductServices;
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthFlowManager, AuthProductError,
    CredentialAccountId, CredentialAccountOwnerScope, CredentialAccountStatus, CredentialOwnership,
    OAuthExchangeCleanupRequest, SecretCleanupAction, SecretCleanupReport, SecretCleanupRequest,
    SecretCleanupService,
};

#[async_trait]
impl<F> SecretCleanupService for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn retain_oauth_exchange_for_cleanup(
        &self,
        request: OAuthExchangeCleanupRequest,
    ) -> Result<CredentialAccountId, AuthProductError> {
        let account_id = CredentialAccountId::from_uuid(request.flow_id.as_uuid());
        self.stage_callback_secret_cleanup(
            account_id,
            request.scope,
            request.exchange.provider,
            request.exchange.account_label,
            Some(request.exchange.access_secret),
            request.exchange.refresh_secret,
        )
        .await?;
        Ok(account_id)
    }

    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError> {
        let mut report = SecretCleanupReport::default();
        // Cancel first, then scan accounts. Together with callback-side CAS
        // compensation this closes both interleavings: a callback that wins
        // before cancellation is found by the account scan, while a callback
        // that loses after cancellation rolls back its own late account write.
        if matches!(request.action, SecretCleanupAction::Uninstall)
            && let Some(provider) = request.provider.as_ref()
        {
            for flow in self
                .lifecycle_flows_for_owner_provider(&request.scope.resource, provider)
                .await?
            {
                let canceled = match flow.status {
                    status if ironclaw_auth::is_terminal_status(status) => flow,
                    _ => match self.cancel_flow(&flow.scope, flow.id).await {
                        Ok(canceled) => canceled,
                        Err(AuthProductError::Canceled) => flow,
                        Err(AuthProductError::FlowAlreadyTerminal) => flow,
                        Err(error) => return Err(error),
                    },
                };
                if canceled.continuation_emitted_at.is_none()
                    && matches!(
                        canceled.continuation,
                        AuthContinuationRef::TurnGateResume { .. }
                    )
                {
                    report
                        .canceled_turn_gate_continuations
                        .push(AuthContinuationEvent {
                            flow_id: canceled.id,
                            scope: canceled.scope.clone(),
                            continuation: canceled.continuation.clone(),
                            provider: canceled.provider.clone(),
                            credential_account_id: canceled.credential_account_id,
                            emitted_at: Utc::now(),
                        });
                }
            }
        }

        // Credential-owner granularity, not full scope equality: lifecycle and
        // disconnect callers mint a fresh `invocation_id` (and often arrive
        // from a different thread), so an exact-scope lookup could never find
        // the account the OAuth/manual flow stored. Per-account operations
        // below use the ACCOUNT's stored scope, which is where its record and
        // secret material actually live.
        let owner = CredentialAccountOwnerScope::from_scope(&request.scope.to_credential_owner());
        for account in self.account_records_for_owner(&owner).await? {
            let owns_extension_account = account.owner_extension.as_ref()
                == Some(&request.extension_id)
                && account.ownership == CredentialOwnership::ExtensionOwned;
            let had_grant = account
                .granted_extensions
                .iter()
                .any(|extension| extension == &request.extension_id);
            let provider_selected = request.provider.as_ref() == Some(&account.provider);
            if !(owns_extension_account || had_grant || provider_selected) {
                continue;
            }
            let lock = self.lock_for(format!("account:{}", account.id));
            let _guard = lock.lock().await;
            let (mut current, version) = self
                .read_account(&account.scope, account.id)
                .await?
                .ok_or(AuthProductError::CredentialMissing)?;
            current
                .granted_extensions
                .retain(|extension| extension != &request.extension_id);
            if had_grant {
                report.removed_grants.push(current.id);
            }
            let should_purge = if owns_extension_account || provider_selected {
                match request.action {
                    SecretCleanupAction::Deactivate => {
                        current.status = CredentialAccountStatus::Inactive;
                        report.retained_accounts.push(current.id);
                        false
                    }
                    SecretCleanupAction::Uninstall => {
                        if current.status != CredentialAccountStatus::Revoked {
                            current.status = CredentialAccountStatus::Revoked;
                            report.revoked_accounts.push(current.id);
                        }
                        true
                    }
                }
            } else {
                if had_grant {
                    report.retained_accounts.push(current.id);
                }
                false
            };
            current.updated_at = Utc::now();
            let mut version = self
                .write_account(&current, CasExpectation::Version(version))
                .await?;
            if should_purge {
                let mut delete_failed = false;
                if let Some(handle) = current.access_secret.clone() {
                    match self
                        .secret_store
                        .delete(&current.scope.resource, &handle)
                        .await
                    {
                        Ok(_) => {
                            current.access_secret = None;
                            current.updated_at = Utc::now();
                            version = self
                                .write_account(&current, CasExpectation::Version(version))
                                .await?;
                        }
                        Err(error) => {
                            tracing::debug!(
                                secret_store_reason = error.stable_reason(),
                                account_id = %current.id,
                                "lifecycle access-secret deletion failed"
                            );
                            delete_failed = true;
                        }
                    }
                }
                if let Some(handle) = current.refresh_secret.clone() {
                    match self
                        .secret_store
                        .delete(&current.scope.resource, &handle)
                        .await
                    {
                        Ok(_) => {
                            current.refresh_secret = None;
                            current.updated_at = Utc::now();
                            self.write_account(&current, CasExpectation::Version(version))
                                .await?;
                        }
                        Err(error) => {
                            tracing::debug!(
                                secret_store_reason = error.stable_reason(),
                                account_id = %current.id,
                                "lifecycle refresh-secret deletion failed"
                            );
                            delete_failed = true;
                        }
                    }
                }
                if delete_failed {
                    return Err(AuthProductError::BackendUnavailable);
                }
            }
        }

        Ok(report)
    }
}
