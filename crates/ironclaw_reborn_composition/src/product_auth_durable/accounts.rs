use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, RootFilesystem};

use super::domain::{
    account_is_authorized_for_requester, recovery_projection_for_single_account,
    recovery_projection_for_unconfigured_accounts, update_account_from_request,
    validate_credential_status_transition,
};
use super::{FilesystemAuthProductServices, scope_matches};
use ironclaw_auth::{
    AuthProductError, CredentialAccount, CredentialAccountChoiceRequest, CredentialAccountId,
    CredentialAccountListPage, CredentialAccountListRequest, CredentialAccountLookupRequest,
    CredentialAccountMutation, CredentialAccountProjection, CredentialAccountSelectionRequest,
    CredentialAccountService, CredentialAccountStatus, CredentialRecoveryProjection,
    CredentialRecoveryReason, CredentialRecoveryRequest, CredentialRefreshReport,
    CredentialRefreshRequest, CredentialSetupService, NewCredentialAccount,
};

#[async_trait]
impl<F> CredentialAccountService for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn create_account(
        &self,
        request: NewCredentialAccount,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.create_account_with_id(CredentialAccountId::new(), request, CasExpectation::Absent)
            .await
    }

    async fn get_account(
        &self,
        request: CredentialAccountLookupRequest,
    ) -> Result<Option<CredentialAccount>, AuthProductError> {
        let Some((account, _)) = self
            .read_account(&request.scope, request.account_id)
            .await?
        else {
            return Ok(None);
        };
        if !scope_matches(&request.scope, &account.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if !account_is_authorized_for_requester(&account, request.requester_extension.as_ref()) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        Ok(Some(account))
    }

    async fn list_accounts(
        &self,
        request: CredentialAccountListRequest,
    ) -> Result<CredentialAccountListPage, AuthProductError> {
        validate_account_list_request(&request)?;
        // Fetch one extra record to detect whether a next page exists, and
        // bound the directory scan to `limit + 1` so we don’t read every
        // account in the scope when only a small page is needed.
        let fetch_limit = request.limit.saturating_add(1);
        let mut accounts = self
            .accounts_for_scope_bounded(&request.scope, fetch_limit)
            .await?
            .into_iter()
            .filter(|account| {
                account.provider == request.provider
                    && request.cursor.is_none_or(|cursor| account.id > cursor)
                    && account_is_authorized_for_requester(
                        account,
                        request.requester_extension.as_ref(),
                    )
            })
            .map(|account| account.projection())
            .collect::<Vec<_>>();
        accounts.sort_by_key(|account| account.id);
        let next_cursor = if accounts.len() > request.limit {
            accounts.truncate(request.limit);
            accounts.last().map(|account| account.id)
        } else {
            None
        };
        Ok(CredentialAccountListPage {
            accounts,
            next_cursor,
        })
    }

    async fn update_status(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        account_id: CredentialAccountId,
        status: CredentialAccountStatus,
    ) -> Result<CredentialAccount, AuthProductError> {
        let lock = self.lock_for(format!("account:{account_id}"));
        let _guard = lock.lock().await;
        let (mut account, version) = self
            .read_account(scope, account_id)
            .await?
            .ok_or(AuthProductError::CredentialMissing)?;
        if !scope_matches(scope, &account.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        validate_credential_status_transition(account.status, status)?;
        account.status = status;
        account.updated_at = Utc::now();
        self.write_account(&account, CasExpectation::Version(version))
            .await?;
        Ok(account)
    }

    async fn select_unique_configured_account(
        &self,
        request: CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        let configured = self
            .accounts_for_scope(&request.scope)
            .await?
            .into_iter()
            .filter(|account| {
                account.provider == request.provider
                    && account.status == CredentialAccountStatus::Configured
            })
            .collect::<Vec<_>>();
        if configured.is_empty() {
            return Err(AuthProductError::CredentialMissing);
        }
        let selectable = configured
            .iter()
            .filter(|account| {
                account_is_authorized_for_requester(account, request.requester_extension.as_ref())
            })
            .collect::<Vec<_>>();
        match selectable.as_slice() {
            [] => Err(AuthProductError::CrossScopeDenied),
            [account] => Ok(account.projection()),
            _ => Err(AuthProductError::AccountSelectionRequired),
        }
    }

    async fn project_credential_recovery(
        &self,
        request: CredentialRecoveryRequest,
    ) -> Result<CredentialRecoveryProjection, AuthProductError> {
        let mut accounts = self
            .accounts_for_scope(&request.scope)
            .await?
            .into_iter()
            .filter(|account| account.provider == request.provider)
            .collect::<Vec<_>>();
        accounts.sort_by_key(|account| account.id);
        if accounts.is_empty() {
            return Ok(CredentialRecoveryProjection::setup_required(
                request.provider,
                CredentialRecoveryReason::NoAccount,
                Vec::new(),
            ));
        }
        let authorized = accounts
            .iter()
            .filter(|account| {
                account_is_authorized_for_requester(account, request.requester_extension.as_ref())
            })
            .collect::<Vec<_>>();
        if authorized.is_empty() {
            return Ok(CredentialRecoveryProjection::setup_required(
                request.provider,
                CredentialRecoveryReason::NoAccount,
                Vec::new(),
            ));
        }
        let configured = authorized
            .iter()
            .copied()
            .filter(|account| account.status == CredentialAccountStatus::Configured)
            .collect::<Vec<_>>();
        match configured.as_slice() {
            [account] => {
                return Ok(CredentialRecoveryProjection::configured(
                    request.provider,
                    account.projection(),
                ));
            }
            [_, ..] => {
                return Ok(CredentialRecoveryProjection::account_selection_required(
                    request.provider,
                    configured
                        .iter()
                        .map(|account| account.projection())
                        .collect(),
                ));
            }
            [] => {}
        }
        if let [account] = authorized.as_slice() {
            return Ok(recovery_projection_for_single_account(
                request.provider,
                account,
            ));
        }
        Ok(recovery_projection_for_unconfigured_accounts(
            request.provider,
            &authorized,
        ))
    }

    async fn select_configured_account(
        &self,
        request: CredentialAccountChoiceRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        let account = self
            .read_account(&request.scope, request.account_id)
            .await?
            .map(|(account, _)| account)
            .ok_or(AuthProductError::CredentialMissing)?;
        if !scope_matches(&request.scope, &account.scope) || account.provider != request.provider {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if account.status != CredentialAccountStatus::Configured {
            return Err(AuthProductError::CredentialMissing);
        }
        if !account_is_authorized_for_requester(&account, request.requester_extension.as_ref()) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        Ok(account.projection())
    }

    async fn refresh_account(
        &self,
        _request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }
}

#[async_trait]
impl<F> CredentialSetupService for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn create_or_update_account(
        &self,
        request: CredentialAccountMutation,
    ) -> Result<CredentialAccount, AuthProductError> {
        match request {
            CredentialAccountMutation::Create(account) => self.create_account(account).await,
            CredentialAccountMutation::Update(update) => {
                let lock = self.lock_for(format!("account:{}", update.account_id));
                let _guard = lock.lock().await;
                let (mut account, version) = self
                    .read_account(&update.account.scope, update.account_id)
                    .await?
                    .ok_or(AuthProductError::CredentialMissing)?;
                update_account_from_request(&mut account, update.account, Utc::now())?;
                self.write_account(&account, CasExpectation::Version(version))
                    .await?;
                Ok(account)
            }
        }
    }
}

fn validate_account_list_request(
    request: &CredentialAccountListRequest,
) -> Result<(), AuthProductError> {
    if request.limit == 0 {
        return Err(AuthProductError::InvalidRequest {
            reason: "credential account list limit must be non-zero".to_string(),
        });
    }
    if request.limit > CredentialAccountListRequest::MAX_LIMIT {
        return Err(AuthProductError::InvalidRequest {
            reason: format!(
                "credential account list limit must be at most {}",
                CredentialAccountListRequest::MAX_LIMIT
            ),
        });
    }
    Ok(())
}
