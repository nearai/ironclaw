use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, RootFilesystem};

use super::FilesystemAuthProductServices;
use ironclaw_auth::{
    AuthProductError, CredentialAccountStatus, CredentialOwnership, SecretCleanupAction,
    SecretCleanupReport, SecretCleanupRequest, SecretCleanupService,
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
        for account in self.accounts_for_scope(&request.scope, usize::MAX).await? {
            let owns_extension_account = account.owner_extension.as_ref()
                == Some(&request.extension_id)
                && account.ownership == CredentialOwnership::ExtensionOwned;
            let had_grant = account
                .granted_extensions
                .iter()
                .any(|extension| extension == &request.extension_id);
            if !(owns_extension_account || had_grant) {
                continue;
            }
            let lock = self.lock_for(format!("account:{}", account.id));
            let _guard = lock.lock().await;
            let (mut current, version) = self
                .read_account(&request.scope, account.id)
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
            let (purge_access, purge_refresh) = if owns_extension_account {
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
            // Purge secret material after the account record is safely
            // persisted without the handles.
            if let Some(h) = &purge_access {
                let _ = self.secret_store.delete(&request.scope.resource, h).await;
            }
            if let Some(h) = &purge_refresh {
                let _ = self.secret_store.delete(&request.scope.resource, h).await;
            }
        }
        Ok(report)
    }
}
