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
        for account in self.accounts_for_scope(&request.scope).await? {
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
            // Capture secret handles before any mutations so we can purge
            // them from SecretStore after the account record is persisted.
            let mut handles_to_purge: [Option<ironclaw_host_api::SecretHandle>; 2] = [None, None];
            if owns_extension_account {
                match request.action {
                    SecretCleanupAction::Deactivate => {
                        current.status = CredentialAccountStatus::Inactive;
                        report.retained_accounts.push(current.id);
                    }
                    SecretCleanupAction::Uninstall => {
                        handles_to_purge =
                            [current.access_secret.take(), current.refresh_secret.take()];
                        if current.status != CredentialAccountStatus::Revoked {
                            current.status = CredentialAccountStatus::Revoked;
                            report.revoked_accounts.push(current.id);
                        }
                    }
                }
            } else if had_grant {
                report.retained_accounts.push(current.id);
            }
            current.updated_at = Utc::now();
            self.write_account(&current, CasExpectation::Version(version))
                .await?;
            // Purge raw secret material after the account record is safely
            // persisted without the handles.
            for handle in handles_to_purge.iter().flatten() {
                let _ = self
                    .secret_store
                    .delete(&request.scope.resource, handle)
                    .await;
            }
        }
        Ok(report)
    }
}
