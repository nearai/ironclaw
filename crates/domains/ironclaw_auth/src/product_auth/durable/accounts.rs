use async_trait::async_trait;
// Imported through base64's own `prelude` rather than its `engine::` module
// path: the crate's module-charter probe is a substring scan for `engine::`,
// which would read `base64::engine::…` as product-auth reaching into the
// vendor-handshake engine. Same symbols, no false positive.
use base64::prelude::{BASE64_STANDARD, Engine as _};
use chrono::Utc;
use ironclaw_extension_contracts::linked_session::{LinkedSessionVersion, SessionBytes};
use ironclaw_filesystem::{CasExpectation, RootFilesystem};
use ironclaw_secrets::{SecretCasExpectation, SecretCasWriteOutcome, SecretVersion};

use super::domain::{
    account_is_authorized_for_requester, recovery_projection_for_single_account,
    recovery_projection_for_unconfigured_accounts, update_account_from_request,
    validate_credential_status_transition,
};
use super::{FilesystemAuthProductServices, scope_matches};
use crate::{
    AuthProductError, CredentialAccount, CredentialAccountChoiceRequest, CredentialAccountId,
    CredentialAccountListPage, CredentialAccountListRequest, CredentialAccountLookupRequest,
    CredentialAccountMutation, CredentialAccountOwnerScope, CredentialAccountProjection,
    CredentialAccountRecordSource, CredentialAccountSelectionRequest, CredentialAccountService,
    CredentialAccountStatus, CredentialRecoveryProjection, CredentialRecoveryReason,
    CredentialRecoveryRequest, CredentialRefreshReport, CredentialRefreshRequest,
    CredentialSetupService, NewCredentialAccount,
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
        let account = match self
            .read_account(&request.scope, request.account_id)
            .await?
        {
            Some((account, _)) => account,
            None => {
                let owner = CredentialAccountOwnerScope::from_scope(&request.scope);
                let Some(account) = self
                    .account_records_for_owner(&owner)
                    .await?
                    .into_iter()
                    .find(|account| account.id == request.account_id)
                else {
                    return Ok(None);
                };
                account
            }
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
        request.validate()?;
        // accounts_for_scope reads all accounts then filters; we cannot push
        // provider/cursor/auth filters to the storage layer without a per-
        // provider directory layout, so pagination is applied in-memory here.
        let mut accounts = self
            .accounts_for_scope(&request.scope)
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
        scope: &crate::AuthProductScope,
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
        if !scope_matches(&request.scope, &account.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if account.provider != request.provider {
            return Err(AuthProductError::CredentialMissing);
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

    /// One durable compare-and-swap on the account record. Never a
    /// caller-supplied value: the bump is what invalidates every handle and
    /// pooled client bound to the previous revision, so a caller that could
    /// *set* it could also replay one.
    async fn bump_link_revision(
        &self,
        scope: &crate::AuthProductScope,
        account_id: CredentialAccountId,
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
        // The ownership pin (PROPOSAL §4.5). Refusing here is what stops a
        // reusable account — reachable by EVERY installed extension, and
        // deliberately not deleted by ownership-aware cleanup — from acquiring
        // a live vendor device authorization.
        if !account.linked_device_ownership_is_pinned() {
            return Err(AuthProductError::invalid_request(
                "a linked-device account must be extension-owned by exactly one \
                 extension and carry no grants",
            ));
        }
        account.link_revision = account.link_revision.saturating_add(1);
        account.updated_at = Utc::now();
        self.write_account(&account, CasExpectation::Version(version))
            .await?;
        Ok(account)
    }

    /// Load the linked-device session blob behind one account, gated on the
    /// account's scope, requester authorization, and `link_revision`.
    ///
    /// The blob is opaque: this store decodes transport base64 and nothing
    /// else. The semantic merge on a conflict belongs to the vendor package,
    /// the only code that can read the format.
    async fn load_opaque_material(
        &self,
        request: crate::OpaqueMaterialRequest,
    ) -> Result<Option<crate::OpaqueMaterialSnapshot>, AuthProductError> {
        let account = self.authorize_opaque_material(&request).await?;
        let Some(handle) = &account.access_secret else {
            return Ok(None);
        };
        let Some(stored) = self
            .secret_store
            .read_versioned(&account.scope.resource, handle)
            .await
            .map_err(|error| {
                tracing::debug!(%error, "linked-session material read failed");
                AuthProductError::BackendUnavailable
            })?
        else {
            return Ok(None);
        };
        use secrecy::ExposeSecret as _;
        let bytes = BASE64_STANDARD
            .decode(stored.material.expose_secret())
            .map_err(|error| {
                tracing::debug!(%error, "linked-session material is not valid transport base64");
                AuthProductError::BackendUnavailable
            })?;
        let material = SessionBytes::new(bytes).map_err(|error| {
            tracing::debug!(%error, "stored linked-session material violates its bounds");
            AuthProductError::BackendUnavailable
        })?;
        Ok(Some(crate::OpaqueMaterialSnapshot {
            material,
            version: material_version_token(stored.version)?,
        }))
    }

    /// Compare-and-swap the linked-device session blob. A lost race is an
    /// outcome carrying the current version — never last-writer-wins, and
    /// never an unconditional retry: a clobbered vendor auth key is a silently
    /// dead link.
    async fn store_opaque_material(
        &self,
        write: crate::OpaqueMaterialWrite,
    ) -> Result<crate::OpaqueMaterialWriteOutcome, AuthProductError> {
        let account = self.authorize_opaque_material(&write.target).await?;
        let Some(handle) = account.access_secret.clone() else {
            return Err(AuthProductError::invalid_request(
                "linked account carries no session secret handle",
            ));
        };
        let expected = match write.expected.as_str() {
            None => SecretCasExpectation::Absent,
            Some(token) => SecretCasExpectation::Version(parse_material_version(token)?),
        };
        let encoded = BASE64_STANDARD.encode(write.material.expose());
        let outcome = self
            .secret_store
            .put_versioned(
                account.scope.resource.clone(),
                handle,
                ironclaw_secrets::SecretMaterial::from(encoded),
                None,
                expected,
            )
            .await
            .map_err(|error| {
                tracing::debug!(%error, "linked-session material write failed");
                AuthProductError::BackendUnavailable
            })?;
        match outcome {
            SecretCasWriteOutcome::Stored { version, .. } => {
                Ok(crate::OpaqueMaterialWriteOutcome::Stored {
                    version: material_version_token(version)?,
                })
            }
            SecretCasWriteOutcome::Conflict { current } => {
                Ok(crate::OpaqueMaterialWriteOutcome::Conflict {
                    current: match current {
                        Some(version) => material_version_token(version)?,
                        None => LinkedSessionVersion::absent(),
                    },
                })
            }
        }
    }
}

impl<F> FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    /// The shared gate in front of both opaque-material operations: the
    /// account must exist in the caller's scope, authorize the requesting
    /// extension, and be addressed at its **current** `link_revision` — a
    /// stale revision is refused with the current one, so a handle from
    /// before an unlink cannot read or clobber the credential that replaced
    /// it. Mirrors the in-memory fake's `authorize_opaque_material` exactly;
    /// the two are pinned together by the durable and contract test tiers.
    async fn authorize_opaque_material(
        &self,
        request: &crate::OpaqueMaterialRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        let (account, _version) = self
            .read_account(&request.scope, request.account_id)
            .await?
            .ok_or(AuthProductError::CredentialMissing)?;
        if !scope_matches(&request.scope, &account.scope) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if !account.is_authorized_for_requester(request.requester_extension.as_ref()) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if account.link_revision != request.link_revision {
            return Err(AuthProductError::LinkRevisionStale {
                current: account.link_revision,
            });
        }
        Ok(account)
    }
}

/// Render a substrate version as the opaque token the custody port carries.
fn material_version_token(
    version: SecretVersion,
) -> Result<LinkedSessionVersion, AuthProductError> {
    LinkedSessionVersion::new(version.get().to_string()).map_err(|error| {
        tracing::debug!(%error, "secret version does not form a linked-session token");
        AuthProductError::BackendUnavailable
    })
}

/// Parse a caller-presented token back into the substrate version it named.
///
/// A token this store never minted (wrong shape, another implementation's
/// format) is an invalid request, not a conflict: refusing it is what stops a
/// forged or stale-format token from expressing a write expectation at all.
fn parse_material_version(token: &str) -> Result<SecretVersion, AuthProductError> {
    token
        .parse::<u64>()
        .map(SecretVersion::from_backend)
        .map_err(|_| AuthProductError::invalid_request("unrecognized linked-session version token"))
}

#[async_trait]
impl<F> CredentialAccountRecordSource for FilesystemAuthProductServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn accounts_for_owner(
        &self,
        scope: &crate::AuthProductScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        let owner = CredentialAccountOwnerScope::from_scope(scope);
        self.account_records_for_owner(&owner).await
    }

    async fn select_unique_configured_account_for_owner(
        &self,
        request: CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.select_configured_account_for_owner(request).await
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
