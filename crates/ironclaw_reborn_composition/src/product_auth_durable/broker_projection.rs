//! One-way projection from product-auth `CredentialAccount` records into
//! the runtime credential broker's `CredentialAccountStore`.
//!
//! The two stores own different vocabularies and lifecycles (see
//! `docs/reborn/contracts/auth-product.md` → "Durable Production Slice"
//! and the rustdoc on [`super::FilesystemAuthProductServices`]).  This
//! module is the deliberate, non-fatal seam that keeps the broker's
//! view of accounts in sync with the product-auth UX view.
//!
//! ## Direction
//!
//! Projection is **product-auth → broker only**.  Product-auth is the
//! source of truth for "what credential accounts exist for this scope";
//! the broker maintains a runtime view used by network-credential
//! injection.  The reverse direction does not exist.
//!
//! ## Failure policy
//!
//! Projection is **best-effort**: failures are logged at `warn` and do
//! not surface to the calling product-auth flow.  Rationale:
//!
//! 1. Product-auth durable writes are the source of truth.  A broker
//!    write failure must not block flow completion or roll back a
//!    successfully-persisted account record — that would leave the user
//!    stuck mid-OAuth on a transient backend hiccup.
//! 2. The broker can be repopulated by re-running projection or by an
//!    eventual reconciler.  Drift between the two stores is therefore
//!    observable (via the warn-level logs) and recoverable, but never
//!    fatal to the UX flow.
//!
//! ## Known gaps (tracked follow-ups)
//!
//! * `allowed_targets` is projected empty.  Product-auth has no network
//!   policy source today, so projected broker accounts are visible via
//!   `accounts_for_scope` but **cannot satisfy session requests** —
//!   `CredentialTargetPolicy::matches()` rejects an empty target list.
//!   Issue: TODO follow-up to product-auth contract.
//! * `provider_or_extension_id` is one-slot in the broker; product-auth
//!   has `owner_extension` plus a list of `granted_extensions`.  We
//!   project `owner_extension` when present, otherwise fall back to
//!   `AuthProviderId` mapped through `ExtensionId::new`.  Grant changes
//!   for non-owner extensions are not currently reflected in the broker
//!   record — a multi-extension projection (or a different broker
//!   shape) is the follow-up.
//! * Cleanup with `SecretCleanupAction::Uninstall` projects the auth
//!   account's resulting `Revoked` status into the broker (rather than
//!   removing the broker row) because `CredentialAccountStore` exposes
//!   no `delete_account` method.  A revoked broker account cannot issue
//!   sessions, which matches the UX intent; the row remains visible for
//!   audit.  A trait-level delete is a separate follow-up.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ExtensionId;
use ironclaw_secrets::{CredentialAccountStore, RedactedJson};

/// Best-effort projector from a product-auth
/// [`ironclaw_auth::CredentialAccount`] into the runtime credential
/// broker store.
///
/// Implementations **must not** return errors: projection is decoupled
/// from product-auth flow control.  Wire diagnostics into the
/// implementation (tracing, metrics) instead of returning a `Result`.
#[async_trait]
pub(crate) trait BrokerAccountProjector: Send + Sync {
    /// Mirror `account` into the runtime broker store.  Status, secret
    /// handles, ownership, and provider metadata are projected; raw
    /// secret material is never touched (the broker references the same
    /// `SecretHandle`s the product-auth record already references).
    async fn project_account(&self, account: &ironclaw_auth::CredentialAccount);
}

/// Default projector: writes through to an `Arc<dyn CredentialAccountStore>`.
pub(crate) struct CredentialBrokerProjector {
    store: Arc<dyn CredentialAccountStore>,
}

impl CredentialBrokerProjector {
    pub(crate) fn new(store: Arc<dyn CredentialAccountStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl BrokerAccountProjector for CredentialBrokerProjector {
    async fn project_account(&self, account: &ironclaw_auth::CredentialAccount) {
        let Some(broker_account) = map_auth_to_broker(account) else {
            tracing::debug!(
                account_id = %account.id,
                status = ?account.status,
                "broker projection skipped: account not mappable (unusable status or invalid id)",
            );
            return;
        };
        if let Err(error) = self.store.put_account(broker_account).await {
            tracing::warn!(
                account_id = %account.id,
                error = %error,
                "broker projection failed: product-auth account persisted, broker view may drift",
            );
        }
    }
}

/// No-op projector for tests, fakes, and configurations where the
/// broker store is not wired.  Always succeeds, never writes.
#[cfg(test)]
pub(crate) struct NoopBrokerAccountProjector;

#[cfg(test)]
#[async_trait]
impl BrokerAccountProjector for NoopBrokerAccountProjector {
    async fn project_account(&self, _: &ironclaw_auth::CredentialAccount) {}
}

/// Map a product-auth account into a runtime broker account.
///
/// Returns `None` when the account is not currently usable by the
/// broker (e.g. `PendingSetup`) or when type conversion fails (e.g. the
/// provider id is not a valid `ExtensionId` and no `owner_extension` is
/// set).  Returning `None` is **not** an error: the caller logs a
/// `debug` line and drops the projection.
fn map_auth_to_broker(
    account: &ironclaw_auth::CredentialAccount,
) -> Option<ironclaw_secrets::CredentialAccount> {
    let id = ironclaw_secrets::CredentialAccountId::new(account.id.to_string()).ok()?;
    let status = map_status(account.status)?;
    let provider_or_extension_id = pick_extension_id(account)?;
    let mut secret_handles = Vec::new();
    if let Some(handle) = account.access_secret.clone() {
        secret_handles.push(handle);
    }
    if let Some(handle) = account.refresh_secret.clone() {
        secret_handles.push(handle);
    }
    let redacted_metadata = build_redacted_metadata(account);
    Some(ironclaw_secrets::CredentialAccount {
        scope: account.scope.resource.clone(),
        id,
        provider_or_extension_id,
        label: account.label.as_str().to_string(),
        status,
        secret_handles,
        // See module docs: empty until product-auth carries a network
        // policy source.  Broker `matches()` will reject any session
        // request against this account.
        allowed_targets: Vec::new(),
        redacted_metadata,
        updated_at: account.updated_at,
    })
}

fn map_status(
    status: ironclaw_auth::CredentialAccountStatus,
) -> Option<ironclaw_secrets::CredentialAccountStatus> {
    use ironclaw_auth::CredentialAccountStatus as A;
    use ironclaw_secrets::CredentialAccountStatus as B;
    match status {
        A::Configured => Some(B::Active),
        A::Expired | A::RefreshFailed => Some(B::Expired),
        A::Revoked => Some(B::Revoked),
        // States below are not yet "broker-usable": the runtime broker
        // has no equivalent state, and projecting them as Active would
        // misrepresent reality.  Drop the projection until the account
        // reaches a usable status.
        A::Inactive | A::Missing | A::PendingSetup => None,
    }
}

fn pick_extension_id(account: &ironclaw_auth::CredentialAccount) -> Option<ExtensionId> {
    if let Some(ext) = account.owner_extension.clone() {
        return Some(ext);
    }
    ExtensionId::new(account.provider.as_str()).ok()
}

fn build_redacted_metadata(account: &ironclaw_auth::CredentialAccount) -> RedactedJson {
    RedactedJson::new(serde_json::json!({
        "provider": account.provider.as_str(),
        "label": account.label.as_str(),
        "ownership": account.ownership,
        "status": account.status,
    }))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    use super::*;

    /// In-memory recording projector used by tests to assert
    /// projection contract: every `write_account` site must invoke
    /// `project_account` exactly once with the persisted record.
    #[derive(Default)]
    pub(crate) struct RecordingBrokerProjector {
        records: Mutex<Vec<ironclaw_auth::CredentialAccount>>,
    }

    impl RecordingBrokerProjector {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn records(&self) -> Vec<ironclaw_auth::CredentialAccount> {
            self.records
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl BrokerAccountProjector for RecordingBrokerProjector {
        async fn project_account(&self, account: &ironclaw_auth::CredentialAccount) {
            self.records
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(account.clone());
        }
    }
}

#[cfg(test)]
mod mapper_tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_auth::{
        AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount, CredentialAccountId,
        CredentialAccountLabel, CredentialAccountStatus, CredentialOwnership,
    };
    use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, TenantId, UserId};

    fn sample_scope() -> AuthProductScope {
        AuthProductScope {
            resource: ResourceScope {
                tenant_id: TenantId::new("tenant").unwrap(),
                user_id: UserId::new("user").unwrap(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            surface: AuthSurface::Web,
            session_id: None,
        }
    }

    fn sample_account(status: CredentialAccountStatus) -> CredentialAccount {
        let now = Utc::now();
        CredentialAccount {
            id: CredentialAccountId::new(),
            scope: sample_scope(),
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("user@example.com").unwrap(),
            status,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: vec![],
            access_secret: Some(SecretHandle::new("oauth_access").unwrap()),
            refresh_secret: Some(SecretHandle::new("oauth_refresh").unwrap()),
            scopes: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn maps_configured_account_to_active_broker_account() {
        let auth_account = sample_account(CredentialAccountStatus::Configured);
        let broker = map_auth_to_broker(&auth_account).expect("should map");
        assert_eq!(
            broker.status,
            ironclaw_secrets::CredentialAccountStatus::Active
        );
        assert_eq!(broker.id.as_str(), &auth_account.id.to_string());
        assert_eq!(broker.scope, auth_account.scope.resource);
        assert_eq!(broker.label, auth_account.label.as_str());
        assert_eq!(broker.secret_handles.len(), 2);
        assert!(
            broker.allowed_targets.is_empty(),
            "allowed_targets is intentionally empty until product-auth carries a network policy source",
        );
    }

    #[test]
    fn maps_expired_and_refresh_failed_to_expired() {
        for status in [
            CredentialAccountStatus::Expired,
            CredentialAccountStatus::RefreshFailed,
        ] {
            let auth_account = sample_account(status);
            let broker = map_auth_to_broker(&auth_account).expect("should map");
            assert_eq!(
                broker.status,
                ironclaw_secrets::CredentialAccountStatus::Expired,
                "status {status:?} should map to Expired",
            );
        }
    }

    #[test]
    fn maps_revoked_account_to_revoked_broker_account() {
        let auth_account = sample_account(CredentialAccountStatus::Revoked);
        let broker = map_auth_to_broker(&auth_account).expect("should map");
        assert_eq!(
            broker.status,
            ironclaw_secrets::CredentialAccountStatus::Revoked
        );
    }

    #[test]
    fn drops_unmappable_status_states() {
        for status in [
            CredentialAccountStatus::Inactive,
            CredentialAccountStatus::Missing,
            CredentialAccountStatus::PendingSetup,
        ] {
            let auth_account = sample_account(status);
            assert!(
                map_auth_to_broker(&auth_account).is_none(),
                "status {status:?} must not project to broker",
            );
        }
    }

    #[test]
    fn prefers_owner_extension_over_provider_for_broker_extension_id() {
        let mut auth_account = sample_account(CredentialAccountStatus::Configured);
        auth_account.owner_extension = Some(ExtensionId::new("gmail").unwrap());
        let broker = map_auth_to_broker(&auth_account).expect("should map");
        assert_eq!(broker.provider_or_extension_id.as_str(), "gmail");
    }

    #[test]
    fn falls_back_to_provider_when_no_owner_extension() {
        let auth_account = sample_account(CredentialAccountStatus::Configured);
        let broker = map_auth_to_broker(&auth_account).expect("should map");
        assert_eq!(broker.provider_or_extension_id.as_str(), "google");
    }

    #[test]
    fn collects_only_present_secret_handles() {
        let mut auth_account = sample_account(CredentialAccountStatus::Configured);
        auth_account.refresh_secret = None;
        let broker = map_auth_to_broker(&auth_account).expect("should map");
        assert_eq!(broker.secret_handles.len(), 1);
    }
}
