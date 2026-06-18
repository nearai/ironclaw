use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, FilesystemError, RecordVersion, RootFilesystem};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use super::{
    FilesystemAuthProductServices, credential_status_for_completed_flow,
    domain::{
        update_account_from_request, validate_account_update_target,
        validate_bound_account_update_target, validate_manual_token_update_binding,
        validate_new_credential_account,
    },
    paths::{fs_error, interaction_path, manual_token_secret_handle},
    scope_matches,
};
use ironclaw_auth::{
    AuthChallenge, AuthInteractionId, AuthInteractionService, AuthProductError, CredentialAccount,
    CredentialAccountId, CredentialAccountUpdateBinding, CredentialOwnership,
    ManualTokenSetupRequest, NewCredentialAccount, SecretSubmitRequest, SecretSubmitResult,
    Timestamp,
};

#[async_trait]
impl<F> AuthInteractionService for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn request_secret_input(
        &self,
        request: ManualTokenSetupRequest,
    ) -> Result<AuthChallenge, AuthProductError> {
        if let Some(binding) = &request.update_binding {
            let account = self
                .read_account(&request.scope, binding.account_id)
                .await?
                .map(|(account, _)| account)
                .ok_or(AuthProductError::CredentialMissing)?;
            validate_manual_token_update_binding(&account, &request, binding)?;
        }
        let interaction = StoredManualTokenInteraction {
            id: AuthInteractionId::new(),
            scope: request.scope,
            provider: request.provider.clone(),
            label: request.label.clone(),
            continuation: request.continuation,
            update_binding: request.update_binding,
            expires_at: request.expires_at,
            consumed_at: None,
        };
        self.write_record(
            &interaction.scope.resource,
            &interaction_path(&interaction.scope, interaction.id)?,
            &interaction,
            CasExpectation::Absent,
        )
        .await?;
        Ok(AuthChallenge::ManualTokenRequired {
            interaction_id: interaction.id,
            provider: interaction.provider,
            label: interaction.label,
            expires_at: interaction.expires_at,
        })
    }

    async fn submit_manual_token(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        request: SecretSubmitRequest,
    ) -> Result<SecretSubmitResult, AuthProductError> {
        validate_secret(&request)?;
        let lock = self.lock_for(format!("interaction:{}", request.interaction_id));
        let _guard = lock.lock().await;
        let path = interaction_path(scope, request.interaction_id)?;
        let (mut pending, version): (StoredManualTokenInteraction, RecordVersion) = self
            .read_record(&scope.resource, &path)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)?;
        if !scope_matches(scope, &pending.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        let now = Utc::now();
        if pending.consumed_at.is_some() || now > pending.expires_at {
            return Err(AuthProductError::UnknownOrExpiredFlow);
        }
        let continuation = pending.continuation.clone();
        let account = self
            .create_or_update_manual_token_account(&pending, request.secret)
            .await?;
        pending.consumed_at = Some(now);
        self.write_record(
            &scope.resource,
            &path,
            &pending,
            CasExpectation::Version(version),
        )
        .await?;
        Ok(SecretSubmitResult {
            account_id: account.id,
            status: account.status,
            continuation,
        })
    }

    async fn abandon_manual_token(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        interaction_id: AuthInteractionId,
    ) -> Result<bool, AuthProductError> {
        let path = interaction_path(scope, interaction_id)?;
        match self.filesystem.delete(&scope.resource, &path).await {
            Ok(()) => Ok(true),
            Err(FilesystemError::NotFound { .. }) => Ok(false),
            Err(error) => Err(fs_error(error)),
        }
    }
}

impl<F> FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn create_or_update_manual_token_account(
        &self,
        pending: &StoredManualTokenInteraction,
        secret: SecretString,
    ) -> Result<CredentialAccount, AuthProductError> {
        let reusable_account = match pending.update_binding.as_ref() {
            Some(_) => None,
            None => self.find_reusable_manual_token_account(pending).await?,
        };
        let account_id = pending
            .update_binding
            .as_ref()
            .map(|binding| binding.account_id)
            .or_else(|| reusable_account.as_ref().map(|account| account.id))
            .unwrap_or_else(|| CredentialAccountId::from_uuid(pending.id.as_uuid()));
        let access_secret = manual_token_secret_handle(account_id, pending.id)?;

        let (ownership, owner_extension, granted_extensions) = pending
            .update_binding
            .as_ref()
            .map(|binding| {
                (
                    binding.ownership,
                    binding.owner_extension.clone(),
                    binding.granted_extensions.clone(),
                )
            })
            .unwrap_or((CredentialOwnership::UserReusable, None, Vec::new()));
        let request_scope = reusable_account
            .as_ref()
            .map(|account| account.scope.clone())
            .unwrap_or_else(|| pending.scope.clone());
        let request = NewCredentialAccount {
            scope: request_scope,
            provider: pending.provider.clone(),
            label: pending.label.clone(),
            status: credential_status_for_completed_flow(),
            ownership,
            owner_extension,
            granted_extensions,
            access_secret: Some(access_secret.clone()),
            refresh_secret: None,
            scopes: Vec::new(),
        };
        match (pending.update_binding.as_ref(), reusable_account.as_ref()) {
            (Some(binding), _) => {
                let lock = self.lock_for(format!("account:{}", binding.account_id));
                let _guard = lock.lock().await;
                let (mut account, version) = self
                    .read_account(&pending.scope, binding.account_id)
                    .await?
                    .ok_or(AuthProductError::CredentialMissing)?;
                // Bound reconnect: authorize at owner granularity (#4935 defect A),
                // exactly as the OAuth callback's `update_bound_oauth_account`
                // does. `validate_account_update_target` (full `scope_matches`)
                // would accept this binding at manual-token setup but then reject
                // it here for any cross-thread reconnect, re-forking the account.
                validate_bound_account_update_target(
                    &account,
                    &pending.scope,
                    &pending.provider,
                    binding,
                )?;
                // Mutate in place at the account's own durable scope (the
                // reconnect arrives from a different thread/invocation; the
                // account does not move), so the subsequent same-scope update
                // check is trivially satisfied — mirroring the reusable path.
                let request = NewCredentialAccount {
                    scope: account.scope.clone(),
                    ..request
                };
                // Capture the old handle so we can delete it from SecretStore after a
                // successful rotation write.  The new handle is stored first so that
                // a write failure still leaves the old material reachable.
                let previous_access_secret = account.access_secret.clone();
                self.store_manual_secret(&account.scope.resource, access_secret, secret)
                    .await?;
                update_account_from_request(&mut account, request, Utc::now())?;
                if let Err(error) = self
                    .write_account(&account, CasExpectation::Version(version))
                    .await
                {
                    // Write failed — clean up the newly stored secret; the old one is
                    // still referenced by the on-disk account record.
                    self.cleanup_manual_secret(&account.scope.resource, &account.access_secret)
                        .await;
                    return Err(error);
                }
                // Write succeeded — the new handle is now canonical.  Delete the
                // previous handle if it differs so we don’t orphan it in SecretStore.
                if previous_access_secret.as_ref() != account.access_secret.as_ref() {
                    self.cleanup_manual_secret(&account.scope.resource, &previous_access_secret)
                        .await;
                }
                Ok(account)
            }
            (None, Some(reusable)) => {
                let lock = self.lock_for(format!("account:{}", reusable.id));
                let _guard = lock.lock().await;
                let (mut account, version) = self
                    .read_account(&reusable.scope, reusable.id)
                    .await?
                    .ok_or(AuthProductError::CredentialMissing)?;
                validate_reusable_manual_token_account(&account, pending)?;
                validate_account_update_target(&account, &request)?;
                let previous_access_secret = account.access_secret.clone();
                self.store_manual_secret(&pending.scope.resource, access_secret, secret)
                    .await?;
                update_account_from_request(&mut account, request, Utc::now())?;
                if let Err(error) = self
                    .write_account(&account, CasExpectation::Version(version))
                    .await
                {
                    self.cleanup_manual_secret(&pending.scope.resource, &account.access_secret)
                        .await;
                    return Err(error);
                }
                if previous_access_secret.as_ref() != account.access_secret.as_ref() {
                    self.cleanup_manual_secret(&account.scope.resource, &previous_access_secret)
                        .await;
                }
                Ok(account)
            }
            (None, None) => {
                validate_new_credential_account(&request)?;
                self.store_manual_secret(&pending.scope.resource, access_secret, secret)
                    .await?;
                match self
                    .create_account_with_id(account_id, request.clone(), CasExpectation::Absent)
                    .await
                {
                    Ok(account) => Ok(account),
                    Err(error) => {
                        self.cleanup_manual_secret(&pending.scope.resource, &request.access_secret)
                            .await;
                        Err(error)
                    }
                }
            }
        }
    }

    async fn find_reusable_manual_token_account(
        &self,
        pending: &StoredManualTokenInteraction,
    ) -> Result<Option<CredentialAccount>, AuthProductError> {
        let owner = ironclaw_auth::CredentialAccountOwnerScope::from_scope(&pending.scope);
        let mut matches = self
            .account_records_for_owner(&owner)
            .await?
            .into_iter()
            .filter(|account| validate_reusable_manual_token_account(account, pending).is_ok())
            .collect::<Vec<_>>();
        matches.sort_by_key(|account| (account.updated_at, account.created_at, account.id));
        Ok(matches.pop())
    }

    async fn store_manual_secret(
        &self,
        resource: &ironclaw_host_api::ResourceScope,
        access_secret: ironclaw_host_api::SecretHandle,
        secret: SecretString,
    ) -> Result<(), AuthProductError> {
        self.secret_store
            .put(resource.clone(), access_secret, secret)
            .await
            .map(|_| ())
            .map_err(|_| AuthProductError::BackendUnavailable)
    }

    async fn cleanup_manual_secret(
        &self,
        scope: &ironclaw_host_api::ResourceScope,
        access_secret: &Option<ironclaw_host_api::SecretHandle>,
    ) {
        // Best-effort: called on error paths where the account write failed, or
        // after successful secret rotation.  The secret is already unreachable
        // via the account record; a delete failure leaves orphaned material in
        // SecretStore but does not affect auth-flow correctness.
        if let Some(access_secret) = access_secret {
            let _ = self.secret_store.delete(scope, access_secret).await;
        }
    }
}

fn validate_reusable_manual_token_account(
    account: &CredentialAccount,
    pending: &StoredManualTokenInteraction,
) -> Result<(), AuthProductError> {
    if account.provider != pending.provider
        || account.label != pending.label
        || account.ownership != CredentialOwnership::UserReusable
        || account.owner_extension.is_some()
        || !account.granted_extensions.is_empty()
        || account.access_secret.is_none()
        || account.status == ironclaw_auth::CredentialAccountStatus::Revoked
    {
        return Err(AuthProductError::CredentialMissing);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredManualTokenInteraction {
    id: AuthInteractionId,
    scope: ironclaw_auth::AuthProductScope,
    provider: ironclaw_auth::AuthProviderId,
    label: ironclaw_auth::CredentialAccountLabel,
    continuation: ironclaw_auth::AuthContinuationRef,
    update_binding: Option<CredentialAccountUpdateBinding>,
    expires_at: Timestamp,
    consumed_at: Option<Timestamp>,
}

fn validate_secret(request: &SecretSubmitRequest) -> Result<(), AuthProductError> {
    let exposed = request.secret.expose_secret();
    if exposed.trim().is_empty() {
        return Err(AuthProductError::InvalidRequest {
            reason: "secret value must not be empty".to_string(),
        });
    }
    if exposed.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(AuthProductError::InvalidRequest {
            reason: "secret value must not contain NUL/control characters".to_string(),
        });
    }
    Ok(())
}
