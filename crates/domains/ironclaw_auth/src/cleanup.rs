use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::ExtensionId;
use serde::{Deserialize, Serialize};

use crate::{
    AuthContinuationEvent, AuthFlowId, AuthProductError, AuthProviderId, CredentialAccountId,
    CredentialAccountRecordSource, LifecyclePackageRef, scope::AuthProductScope,
};

/// Lifecycle event that drives credential/session cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretCleanupAction {
    Deactivate,
    Uninstall,
}

/// Accounts are matched at credential-owner granularity (the scope's
/// tenant/user/agent/project owner — see
/// [`AuthProductScope::to_credential_owner`]), never by full scope equality:
/// every lifecycle/disconnect caller re-derives its scope with a fresh
/// `invocation_id`, so exact-scope matching could never find the account the
/// OAuth flow stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretCleanupRequest {
    pub scope: AuthProductScope,
    pub extension_id: ExtensionId,
    /// Explicit opt-in that ALSO selects the owner's accounts issued by this
    /// provider. OAuth-minted personal credentials are stored `UserReusable`
    /// with no extension ownership or grants, so an extension-keyed cleanup
    /// can never reach them; per the crate guardrail, reusable credentials are
    /// untouched *by default* — this selector is the deliberate exception a
    /// channel disconnect uses to revoke (not delete) the caller's own
    /// personal token.
    pub provider: Option<AuthProviderId>,
    /// Cancel every non-terminal flow whose `LifecycleActivation`
    /// continuation names this package, regardless of provider. Uninstall
    /// callers pass the removed extension's package ref so its own connect
    /// flows die with it even when the provider is shared with another
    /// installed extension (a provider-keyed cancel is deliberately skipped
    /// for shared providers, but the removed extension's flows must not
    /// survive to complete a late callback and then compensate away the
    /// shared credential).
    pub lifecycle_package: Option<LifecyclePackageRef>,
    pub action: SecretCleanupAction,
}

/// A flow that lifecycle cleanup drove to (or found already in) a terminal
/// state, so callers can drop per-flow material stored outside the flow
/// record — today the setup-path PKCE verifier secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanceledCleanupFlow {
    pub scope: AuthProductScope,
    pub flow_id: AuthFlowId,
}

/// Redacted cleanup report. It carries account ids only, never secret handles or
/// backend diagnostic details.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretCleanupReport {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked_accounts: Vec<CredentialAccountId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retained_accounts: Vec<CredentialAccountId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_grants: Vec<CredentialAccountId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quarantined_accounts: Vec<SecretCleanupQuarantine>,
    /// Canceled turn-gate continuations that the composition layer must deny
    /// through the turn coordinator before lifecycle cleanup is complete.
    /// This internal handoff is deliberately omitted from product responses;
    /// it carries no secret material (flow/scope/continuation refs only).
    #[serde(skip)]
    pub canceled_turn_gate_continuations: Vec<AuthContinuationEvent>,
    /// Flows this cleanup walked to a terminal state, so the composition
    /// layer can eagerly drop their durable setup PKCE verifier secrets
    /// instead of leaving them to TTL expiry. Internal handoff only — never
    /// serialized into product responses.
    #[serde(skip)]
    pub canceled_flows: Vec<CanceledCleanupFlow>,
}

/// Stable redacted cleanup quarantine category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretCleanupQuarantineReason {
    RevokeFailed,
    GrantRevokeFailed,
    TombstoneFailed,
    BackendUnavailable,
}

/// Redacted cleanup diagnostic. It names only the affected account and stable
/// failure category, never backend strings, secret handles, or host paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretCleanupQuarantine {
    pub account_id: CredentialAccountId,
    pub reason: SecretCleanupQuarantineReason,
}

#[async_trait]
pub trait SecretCleanupService: Send + Sync {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError>;
}

/// One established linked device to tear down, addressed exactly the way a live
/// custody handle is: account **and** link revision.
///
/// The revision travels because it is part of the identity of a session, not a
/// hint — a request carrying a stale one must fail rather than log a device out
/// of the credential that replaced the one it named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedDeviceRevokeRequest {
    /// The account's own stored scope, which is where its record and secret
    /// material live — never the lifecycle caller's freshly minted scope.
    pub scope: AuthProductScope,
    /// The extension that owns the account, and whose adapter holds the vendor
    /// conversation.
    pub extension_id: ExtensionId,
    pub account_id: CredentialAccountId,
    pub link_revision: u64,
}

/// Why a linked-device teardown did not complete.
///
/// Closed and text-free on purpose: the implementation is above this crate and
/// speaks to a vendor, so a variant that could carry a message would be a place
/// for vendor error bodies to land. Every variant means the same thing to
/// cleanup — the vendor may still hold a live authorization — and all three
/// quarantine as [`SecretCleanupQuarantineReason::RevokeFailed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkedDeviceRevokeError {
    /// No installed extension binds a device-link adapter for this account.
    #[error("no device-link adapter is bound for this extension")]
    NoBinding,
    /// The vendor refused or failed the logout.
    #[error("linked-device vendor logout failed")]
    Vendor,
    /// Host mediation for linked devices is not wired or is unavailable.
    #[error("linked-device revocation is unavailable")]
    Unavailable,
}

/// Ends the vendor-side authorization behind one linked device.
///
/// Declared here and implemented above this crate, by the tier that can resolve
/// an extension to its bound device-link adapter. `ironclaw_auth` owns *when*
/// a device must be logged out and *in what order*; it deliberately cannot see
/// the vendor call itself.
#[async_trait]
pub trait LinkedDeviceRevoker: Send + Sync {
    async fn revoke_linked_device(
        &self,
        request: LinkedDeviceRevokeRequest,
    ) -> Result<(), LinkedDeviceRevokeError>;
}

/// A revoker slot filled after construction.
///
/// The vendor half of a device link is the extension host's snapshot driver,
/// and the extension host is composed *after* the auth service bundle — so the
/// cleanup chain is built over this slot and composition fills it the moment
/// the driver exists. Unfilled it fails closed: the decorator quarantines
/// `RevokeFailed` rather than silently skipping the vendor logout.
#[derive(Default)]
pub struct DeferredLinkedDeviceRevoker {
    inner: std::sync::OnceLock<Arc<dyn LinkedDeviceRevoker>>,
}

impl DeferredLinkedDeviceRevoker {
    /// Bind the real revoker. First fill wins; a second is ignored.
    pub fn fill(&self, revoker: Arc<dyn LinkedDeviceRevoker>) {
        let _ = self.inner.set(revoker);
    }
}

#[async_trait]
impl LinkedDeviceRevoker for DeferredLinkedDeviceRevoker {
    async fn revoke_linked_device(
        &self,
        request: LinkedDeviceRevokeRequest,
    ) -> Result<(), LinkedDeviceRevokeError> {
        match self.inner.get() {
            Some(revoker) => revoker.revoke_linked_device(request).await,
            None => Err(LinkedDeviceRevokeError::Unavailable),
        }
    }
}

/// Cleanup decorator that logs a linked device out **before** the credential it
/// hangs off is unbound.
///
/// **The ordering is the whole point, and it only runs one way.** Deactivate
/// and uninstall both end with the account's secret material purged and its
/// record unbound; after that, nothing can name the session blob a logout needs,
/// and the code that could have made the call is gone with the extension. So
/// the revoke happens here, strictly before the inner service is called, and
/// the structure of this decorator — not a comment inside a longer routine — is
/// what guarantees it (PROPOSAL §4.5, "ordered before unbind").
///
/// **A failed logout never blocks the unbind.** Local deletion proceeds and the
/// account is reported quarantined with
/// [`SecretCleanupQuarantineReason::RevokeFailed`], which is the honest answer:
/// the credential is gone on this side and the vendor may still hold a device.
/// Swallowing the failure would report a teardown that did not happen.
pub struct LinkedDeviceCleanupService {
    accounts: Arc<dyn CredentialAccountRecordSource>,
    revoker: Arc<dyn LinkedDeviceRevoker>,
    inner: Arc<dyn SecretCleanupService>,
}

impl LinkedDeviceCleanupService {
    pub fn new(
        accounts: Arc<dyn CredentialAccountRecordSource>,
        revoker: Arc<dyn LinkedDeviceRevoker>,
        inner: Arc<dyn SecretCleanupService>,
    ) -> Self {
        Self {
            accounts,
            revoker,
            inner,
        }
    }

    /// The linked devices this lifecycle event tears down: the ones this
    /// extension owns outright.
    ///
    /// A grant is not enough and cannot be: a linked device is pinned
    /// `ExtensionOwned` with no grants, so an extension that merely *held* a
    /// grant on an account with a live device authorization is a record that
    /// should not exist — and logging a device out on its behalf would let an
    /// unrelated uninstall kill somebody else's session.
    async fn revocable_devices(
        &self,
        request: &SecretCleanupRequest,
    ) -> Result<Vec<LinkedDeviceRevokeRequest>, AuthProductError> {
        let accounts = match self
            .accounts
            .accounts_for_owner(&request.scope.to_credential_owner())
            .await
        {
            Ok(accounts) => accounts,
            // A bundle whose account read model is not wired cannot hold a
            // linked device — the mint runs through the same credential
            // service — so there is nothing here to revoke and cleanup
            // proceeds. Any OTHER read failure is fatal on purpose: skipping
            // a vendor logout because a backend blipped would leave a live
            // device authorization nobody can see, which is the exact failure
            // this decorator exists to prevent.
            Err(AuthProductError::UnsupportedOperation { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        Ok(accounts
            .into_iter()
            .filter(|account| {
                account.is_linked_device()
                    && account.linked_device_ownership_is_pinned()
                    && account.owner_extension.as_ref() == Some(&request.extension_id)
            })
            .map(|account| LinkedDeviceRevokeRequest {
                scope: account.scope.clone(),
                extension_id: request.extension_id.clone(),
                account_id: account.id,
                link_revision: account.link_revision,
            })
            .collect())
    }
}

#[async_trait]
impl SecretCleanupService for LinkedDeviceCleanupService {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError> {
        let devices = self.revocable_devices(&request).await?;
        let mut quarantined = Vec::new();
        for device in devices {
            let account_id = device.account_id;
            if let Err(error) = self.revoker.revoke_linked_device(device).await {
                tracing::debug!(
                    %account_id,
                    revoke_error = %error,
                    "linked-device logout failed; unbinding anyway and reporting unverified"
                );
                quarantined.push(SecretCleanupQuarantine {
                    account_id,
                    reason: SecretCleanupQuarantineReason::RevokeFailed,
                });
            }
        }
        let mut report = self.inner.cleanup_for_lifecycle(request).await?;
        report.quarantined_accounts.extend(quarantined);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_host_api::{
        ids::{InvocationId, SecretHandle, UserId},
        resource::ResourceScope,
    };

    use super::*;
    use crate::{
        AuthSurface, CredentialAccount, CredentialAccountLabel, CredentialAccountService,
        CredentialAccountStatus, CredentialOwnership, InMemoryAuthProductServices,
        NewCredentialAccount,
    };

    /// Every step either collaborator took, in the order it happened. The
    /// ordering assertion is the reason this is one shared log and not two.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum CleanupStep {
        Revoked(CredentialAccountId),
        Unbound,
    }

    #[derive(Default)]
    struct CleanupJournal(Mutex<Vec<CleanupStep>>);

    impl CleanupJournal {
        fn record(&self, step: CleanupStep) {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(step);
        }

        fn steps(&self) -> Vec<CleanupStep> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    struct RecordingRevoker {
        journal: Arc<CleanupJournal>,
        failure: Option<LinkedDeviceRevokeError>,
        seen: Mutex<Vec<LinkedDeviceRevokeRequest>>,
    }

    impl RecordingRevoker {
        fn new(journal: Arc<CleanupJournal>) -> Self {
            Self {
                journal,
                failure: None,
                seen: Mutex::new(Vec::new()),
            }
        }

        fn failing(journal: Arc<CleanupJournal>, failure: LinkedDeviceRevokeError) -> Self {
            Self {
                journal,
                failure: Some(failure),
                seen: Mutex::new(Vec::new()),
            }
        }

        fn seen(&self) -> Vec<LinkedDeviceRevokeRequest> {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl LinkedDeviceRevoker for RecordingRevoker {
        async fn revoke_linked_device(
            &self,
            request: LinkedDeviceRevokeRequest,
        ) -> Result<(), LinkedDeviceRevokeError> {
            self.journal
                .record(CleanupStep::Revoked(request.account_id));
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            match self.failure {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        }
    }

    /// Stands in for the durable cleanup that unbinds the credential. It only
    /// has to be observable in the journal; what it would delete is the durable
    /// store's own tested behavior. It reports one quarantine of its own so a
    /// decorator that *replaced* the inner report instead of extending it would
    /// be caught.
    struct RecordingUnbind {
        journal: Arc<CleanupJournal>,
        own_quarantine: CredentialAccountId,
    }

    impl RecordingUnbind {
        fn new(journal: Arc<CleanupJournal>) -> Self {
            Self {
                journal,
                own_quarantine: CredentialAccountId::new(),
            }
        }
    }

    #[async_trait]
    impl SecretCleanupService for RecordingUnbind {
        async fn cleanup_for_lifecycle(
            &self,
            _request: SecretCleanupRequest,
        ) -> Result<SecretCleanupReport, AuthProductError> {
            self.journal.record(CleanupStep::Unbound);
            Ok(SecretCleanupReport {
                quarantined_accounts: vec![SecretCleanupQuarantine {
                    account_id: self.own_quarantine,
                    reason: SecretCleanupQuarantineReason::BackendUnavailable,
                }],
                ..SecretCleanupReport::default()
            })
        }
    }

    fn owner_scope(user: &str) -> AuthProductScope {
        AuthProductScope::new(
            ResourceScope::local_default(UserId::new(user).unwrap(), InvocationId::new()).unwrap(),
            AuthSurface::Api,
        )
    }

    fn new_account(
        scope: AuthProductScope,
        provider: &str,
        ownership: CredentialOwnership,
        owner_extension: Option<&str>,
    ) -> NewCredentialAccount {
        NewCredentialAccount {
            scope,
            provider: AuthProviderId::new(provider).unwrap(),
            label: CredentialAccountLabel::new(format!("{provider}-account")).unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership,
            owner_extension: owner_extension.map(|id| ExtensionId::new(id).unwrap()),
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new(format!("{provider}_access")).unwrap()),
            refresh_secret: None,
            scopes: Vec::new(),
        }
    }

    /// Create an account and drive it to `link_revision = 1`, i.e. an
    /// established linked device.
    async fn linked_device(
        accounts: &InMemoryAuthProductServices,
        scope: AuthProductScope,
        provider: &str,
        owner_extension: &str,
    ) -> CredentialAccount {
        let created = accounts
            .create_account(NewCredentialAccount::for_linked_device(
                scope.clone(),
                AuthProviderId::new(provider).unwrap(),
                CredentialAccountLabel::new(format!("{provider}-linked")).unwrap(),
                ExtensionId::new(owner_extension).unwrap(),
                SecretHandle::new(format!("{provider}_session")).unwrap(),
            ))
            .await
            .expect("linked-device account is created");
        accounts
            .bump_link_revision(&scope, created.id)
            .await
            .expect("ownership is pinned, so the link revision bumps")
    }

    fn cleanup_request(
        scope: AuthProductScope,
        extension: &str,
        action: SecretCleanupAction,
    ) -> SecretCleanupRequest {
        SecretCleanupRequest {
            scope,
            extension_id: ExtensionId::new(extension).unwrap(),
            provider: None,
            lifecycle_package: None,
            action,
        }
    }

    /// PROPOSAL §4.5: the vendor logout is ordered BEFORE the unbind, on both
    /// lifecycle events — after the unbind the session blob is gone and the
    /// extension that could call the vendor is gone with it.
    #[tokio::test]
    async fn deactivate_and_uninstall_revoke_the_linked_device_before_unbinding_it() {
        for action in [
            SecretCleanupAction::Deactivate,
            SecretCleanupAction::Uninstall,
        ] {
            let accounts = Arc::new(InMemoryAuthProductServices::new());
            let scope = owner_scope("alice");
            let account = linked_device(&accounts, scope.clone(), "vendor-a", "ext-a").await;
            let journal = Arc::new(CleanupJournal::default());
            let revoker = Arc::new(RecordingRevoker::new(Arc::clone(&journal)));
            let service = LinkedDeviceCleanupService::new(
                accounts,
                Arc::clone(&revoker) as Arc<dyn LinkedDeviceRevoker>,
                Arc::new(RecordingUnbind::new(Arc::clone(&journal))),
            );

            service
                .cleanup_for_lifecycle(cleanup_request(scope, "ext-a", action))
                .await
                .expect("cleanup succeeds");

            assert_eq!(
                journal.steps(),
                vec![CleanupStep::Revoked(account.id), CleanupStep::Unbound],
                "the vendor logout must run before the credential is unbound ({action:?})"
            );
            let seen = revoker.seen();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].account_id, account.id);
            assert_eq!(
                seen[0].link_revision, 1,
                "the revoke is addressed at the account's current link revision"
            );
            assert_eq!(seen[0].extension_id, ExtensionId::new("ext-a").unwrap());
        }
    }

    /// A logout that fails is reported, not swallowed: the local credential is
    /// still unbound, and the account is quarantined `RevokeFailed` so the
    /// outcome reads as explicitly unverified.
    #[tokio::test]
    async fn a_failed_linked_device_logout_quarantines_and_still_unbinds() {
        for failure in [
            LinkedDeviceRevokeError::NoBinding,
            LinkedDeviceRevokeError::Vendor,
            LinkedDeviceRevokeError::Unavailable,
        ] {
            let accounts = Arc::new(InMemoryAuthProductServices::new());
            let scope = owner_scope("alice");
            let account = linked_device(&accounts, scope.clone(), "vendor-a", "ext-a").await;
            let journal = Arc::new(CleanupJournal::default());
            let service = LinkedDeviceCleanupService::new(
                accounts,
                Arc::new(RecordingRevoker::failing(Arc::clone(&journal), failure)),
                Arc::new(RecordingUnbind::new(Arc::clone(&journal))),
            );

            let report = service
                .cleanup_for_lifecycle(cleanup_request(
                    scope,
                    "ext-a",
                    SecretCleanupAction::Uninstall,
                ))
                .await
                .expect("a failed logout must not fail the cleanup");

            assert_eq!(
                journal.steps(),
                vec![CleanupStep::Revoked(account.id), CleanupStep::Unbound],
                "local deletion proceeds even when the vendor logout fails ({failure:?})"
            );
            assert!(
                report
                    .quarantined_accounts
                    .iter()
                    .any(|quarantine| quarantine.account_id == account.id
                        && quarantine.reason == SecretCleanupQuarantineReason::RevokeFailed),
                "a failed logout must quarantine RevokeFailed, not report success ({failure:?})"
            );
        }
    }

    /// The decorator is not a blanket "log everything out": an account with no
    /// link revision has no device behind it, and calling a vendor for it would
    /// be a fabricated teardown.
    #[tokio::test]
    async fn an_account_that_is_not_a_linked_device_is_never_revoked() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope = owner_scope("alice");
        accounts
            .create_account(new_account(
                scope.clone(),
                "vendor-a",
                CredentialOwnership::ExtensionOwned,
                Some("ext-a"),
            ))
            .await
            .expect("account is created");
        let journal = Arc::new(CleanupJournal::default());
        let revoker = Arc::new(RecordingRevoker::new(Arc::clone(&journal)));
        let service = LinkedDeviceCleanupService::new(
            accounts,
            Arc::clone(&revoker) as Arc<dyn LinkedDeviceRevoker>,
            Arc::new(RecordingUnbind::new(Arc::clone(&journal))),
        );

        service
            .cleanup_for_lifecycle(cleanup_request(
                scope,
                "ext-a",
                SecretCleanupAction::Uninstall,
            ))
            .await
            .expect("cleanup succeeds");

        assert_eq!(journal.steps(), vec![CleanupStep::Unbound]);
        assert!(revoker.seen().is_empty());
    }

    /// Uninstalling one extension must not log out another extension's device.
    #[tokio::test]
    async fn another_extensions_linked_device_is_left_alone() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope = owner_scope("alice");
        let mine = linked_device(&accounts, scope.clone(), "vendor-a", "ext-a").await;
        linked_device(&accounts, scope.clone(), "vendor-b", "ext-b").await;
        let journal = Arc::new(CleanupJournal::default());
        let revoker = Arc::new(RecordingRevoker::new(Arc::clone(&journal)));
        let service = LinkedDeviceCleanupService::new(
            accounts,
            Arc::clone(&revoker) as Arc<dyn LinkedDeviceRevoker>,
            Arc::new(RecordingUnbind::new(Arc::clone(&journal))),
        );

        service
            .cleanup_for_lifecycle(cleanup_request(
                scope,
                "ext-a",
                SecretCleanupAction::Uninstall,
            ))
            .await
            .expect("cleanup succeeds");

        let seen = revoker.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].account_id, mine.id);
    }

    /// Read-model doubles for the two shapes of "we could not enumerate
    /// accounts". The distinction is security-relevant, so it is pinned rather
    /// than left to whichever error a future read model happens to return.
    struct UnwiredAccounts;

    #[async_trait]
    impl CredentialAccountRecordSource for UnwiredAccounts {
        async fn accounts_for_owner(
            &self,
            _scope: &AuthProductScope,
        ) -> Result<Vec<crate::CredentialAccount>, AuthProductError> {
            Err(AuthProductError::UnsupportedOperation {
                operation: "accounts_for_owner",
            })
        }
    }

    struct FailingAccounts;

    #[async_trait]
    impl CredentialAccountRecordSource for FailingAccounts {
        async fn accounts_for_owner(
            &self,
            _scope: &AuthProductScope,
        ) -> Result<Vec<crate::CredentialAccount>, AuthProductError> {
            Err(AuthProductError::BackendUnavailable)
        }
    }

    /// A bundle with **no** account read model has no linked device to log
    /// out — the mint runs through the same credential service — so cleanup
    /// proceeds. A read model that genuinely **failed** is the opposite: we
    /// cannot tell whether a live device authorization exists, and unbinding
    /// anyway would strand one where nobody can see it. Failing closed there
    /// is the whole point of the decorator.
    #[tokio::test]
    async fn an_unwired_account_read_model_is_not_a_failed_one() {
        let scope = owner_scope("alice");
        let journal = Arc::new(CleanupJournal::default());
        let revoker = Arc::new(RecordingRevoker::new(Arc::clone(&journal)));

        let unwired = LinkedDeviceCleanupService::new(
            Arc::new(UnwiredAccounts),
            Arc::clone(&revoker) as Arc<dyn LinkedDeviceRevoker>,
            Arc::new(RecordingUnbind::new(Arc::clone(&journal))),
        );
        unwired
            .cleanup_for_lifecycle(cleanup_request(
                scope.clone(),
                "ext-a",
                SecretCleanupAction::Uninstall,
            ))
            .await
            .expect("an unwired read model must not break cleanup");
        assert_eq!(
            journal.steps(),
            vec![CleanupStep::Unbound],
            "the inner cleanup still runs, and no vendor logout is fabricated"
        );
        assert!(revoker.seen().is_empty());

        let failing = LinkedDeviceCleanupService::new(
            Arc::new(FailingAccounts),
            Arc::clone(&revoker) as Arc<dyn LinkedDeviceRevoker>,
            Arc::new(RecordingUnbind::new(Arc::clone(&journal))),
        );
        let error = failing
            .cleanup_for_lifecycle(cleanup_request(
                scope,
                "ext-a",
                SecretCleanupAction::Uninstall,
            ))
            .await
            .expect_err("a failed read model must not silently skip the logout");
        assert!(
            matches!(error, AuthProductError::BackendUnavailable),
            "unexpected error: {error:?}"
        );
    }
}
