use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, RootFilesystem};

use super::FilesystemAuthProductServices;
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthFlowManager, AuthProductError,
    CanceledCleanupFlow, CredentialAccountOwnerScope, CredentialAccountStatus, CredentialOwnership,
    SecretCleanupAction, SecretCleanupReport, SecretCleanupRequest, SecretCleanupService,
};

#[async_trait]
impl<F> SecretCleanupService for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError> {
        let mut report = SecretCleanupReport::default();
        // A3 · Cancel the provider's pending flows BEFORE enumerating
        // accounts (RFC 9700 §4.7.1 + RFC 7009 §1). Ordering closes the
        // callback/removal race: a callback racing this cleanup either loses
        // — its flow is canceled first, so `complete_oauth_callback` rejects
        // before writing an account — or wins and completes first, in which
        // case the account it minted already exists when the scan below runs
        // and is revoked like any other. Scanning accounts first left a
        // window where a callback completing between the scan and the flow
        // cancellation minted a credential that survived removal.
        //
        // Owner decision 2026-07-15: cancel on both Deactivate and Uninstall.
        // Shared-vendor safe by construction — the removal caller only
        // selects a provider exclusive to the removed extension. Idempotent:
        // a concurrently terminal flow is skipped, never an error.
        //
        // F2 · Any enumerated flow whose `TurnGateResume` continuation was
        // never acknowledged — freshly canceled here or already terminal — is
        // reported so the composition layer denies its blocked turn gate
        // instead of leaving the turn parked. `mark_continuation_dispatched`
        // makes the handoff emit-once across cleanup retries.
        if request.provider.is_some() || request.lifecycle_package.is_some() {
            let mut flows = Vec::new();
            if let Some(provider) = request.provider.as_ref() {
                flows.extend(
                    self.lifecycle_flows_for_owner_provider(&request.scope.resource, provider)
                        .await?,
                );
            }
            // Package-keyed selection (#6169) is independent of the provider
            // selector: uninstall passes it even when the provider is shared
            // with (and therefore retained for) another installed extension,
            // so the removed extension's own LifecycleActivation flows still
            // die with it.
            if let Some(package) = request.lifecycle_package.as_ref() {
                for flow in self
                    .lifecycle_flows_for_owner_package(&request.scope.resource, package)
                    .await?
                {
                    if !flows.iter().any(|existing| existing.id == flow.id) {
                        flows.push(flow);
                    }
                }
            }
            for flow in flows {
                let canceled = match flow.status {
                    status if ironclaw_auth::is_terminal_status(status) => flow,
                    _ => match self.cancel_flow(&flow.scope, flow.id).await {
                        Ok(canceled) => canceled,
                        Err(AuthProductError::Canceled)
                        | Err(AuthProductError::FlowAlreadyTerminal)
                        | Err(AuthProductError::UnknownOrExpiredFlow) => flow,
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
                // Name every walked terminal flow so the composition wrapper
                // can eagerly drop its durable setup PKCE verifier.
                report.canceled_flows.push(CanceledCleanupFlow {
                    scope: canceled.scope.clone(),
                    flow_id: canceled.id,
                });
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
            // Capture handles to purge before mutating the record so we can
            // delete from SecretStore after the account write.
            let (purge_access, purge_refresh) = if owns_extension_account || provider_selected {
                match request.action {
                    SecretCleanupAction::Deactivate => {
                        current.status = CredentialAccountStatus::Inactive;
                        report.retained_accounts.push(current.id);
                        (None, None)
                    }
                    SecretCleanupAction::Uninstall => {
                        let access = current.access_secret.take();
                        let refresh = current.refresh_secret.take();
                        if current.status != CredentialAccountStatus::Revoked {
                            current.status = CredentialAccountStatus::Revoked;
                            report.revoked_accounts.push(current.id);
                        }
                        (access, refresh)
                    }
                }
            } else {
                if had_grant {
                    report.retained_accounts.push(current.id);
                }
                (None, None)
            };
            current.updated_at = Utc::now();
            self.write_account(&current, CasExpectation::Version(version))
                .await?;
            // Purge secret material after the account record is safely persisted
            // without the handles.  Best-effort: the account no longer references
            // these handles so any leftover material becomes unreachable even if
            // the delete call fails (e.g. transient backend outage).
            if let Some(h) = &purge_access {
                self.purge_secret_handle(&current.scope.resource, h).await;
            }
            if let Some(h) = &purge_refresh {
                self.purge_secret_handle(&current.scope.resource, h).await;
            }
        }
        Ok(report)
    }
}
