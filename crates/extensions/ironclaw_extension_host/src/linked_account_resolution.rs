//! The host-implemented [`LinkedAccountResolver`]: which linked account does
//! this tool call act as?
//!
//! The tool ABI deliberately carries no host-issued grant (`ToolPorts` and
//! `ToolCall` are frozen), so the resolver is supplied at bind, beside the
//! custody factory, and the package calls it per dispatch. Resolution runs
//! through the **same** credential-account selection service every runtime
//! credential injection uses — one selection policy, not a parallel one — so
//! the host-managed-fallback exclusion, status checks, and requester
//! authorization all apply before a grant is ever minted.
//!
//! Vendor-blind by construction: the vendor id and extension id are read from
//! the resolved manifest at bind time; nothing here names a concrete
//! extension.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, RuntimeCredentialAccountSelectionService,
    runtime_credential_account_selection_request,
};
use ironclaw_extension_contracts::linked_session::{
    LinkedAccountGrant, LinkedAccountRef, LinkedAccountResolutionError, LinkedAccountResolver,
};
use ironclaw_extension_registry::ResolvedExtensionManifest;
use ironclaw_host_api::capability::RuntimeCredentialAccountSetup;
use ironclaw_host_api::ids::{ExtensionId, VendorId};
use ironclaw_host_api::resource::ResourceScope;
use tracing::debug;

use crate::linked_session_custody::{LinkedSessionStore, UnavailableLinkedAccountResolver};

/// Builds the per-extension resolver a bind hands the package.
///
/// A factory rather than one resolver because the vendor axis is per
/// extension: it comes from the manifest's device-link auth surface, resolved
/// once at bind. Generic code stays vendor-blind — the value is data from the
/// resolved manifest, never a name in host code.
pub trait LinkedAccountResolution: Send + Sync {
    fn resolver_for(&self, resolved: &ResolvedExtensionManifest) -> Arc<dyn LinkedAccountResolver>;
}

/// The fail-closed factory for deployments that wire no linked-account
/// custody: every resolver it builds answers `Unavailable`.
pub struct UnavailableLinkedAccountResolution;

impl LinkedAccountResolution for UnavailableLinkedAccountResolution {
    fn resolver_for(
        &self,
        _resolved: &ResolvedExtensionManifest,
    ) -> Arc<dyn LinkedAccountResolver> {
        Arc::new(UnavailableLinkedAccountResolver)
    }
}

/// The production factory: resolvers select through the credential-account
/// service and register the resulting ref in the custody directory.
pub struct CredentialLinkedAccountResolution {
    selection: Arc<dyn RuntimeCredentialAccountSelectionService>,
    sessions: Arc<LinkedSessionStore>,
}

impl CredentialLinkedAccountResolution {
    pub fn new(
        selection: Arc<dyn RuntimeCredentialAccountSelectionService>,
        sessions: Arc<LinkedSessionStore>,
    ) -> Self {
        Self {
            selection,
            sessions,
        }
    }
}

impl LinkedAccountResolution for CredentialLinkedAccountResolution {
    fn resolver_for(&self, resolved: &ResolvedExtensionManifest) -> Arc<dyn LinkedAccountResolver> {
        // The vendor axis comes from the manifest's device-link auth surface.
        // An extension without one gets the fail-closed resolver: its tools
        // (if any) have no linked account to act as, and answering
        // `Unavailable` is honest where guessing a vendor would not be.
        let vendor = resolved.auth.iter().find_map(|surface| {
            matches!(
                surface.recipe,
                Some(ironclaw_extension_contracts::recipe::VendorAuthRecipe::DeviceLink(_))
            )
            .then(|| surface.vendor.clone())
        });
        match vendor {
            Some(vendor) => Arc::new(CredentialLinkedAccountResolver {
                selection: Arc::clone(&self.selection),
                sessions: Arc::clone(&self.sessions),
                extension_id: resolved.id.clone(),
                vendor,
            }),
            None => Arc::new(UnavailableLinkedAccountResolver),
        }
    }
}

/// Resolves one extension's linked account for one caller scope.
struct CredentialLinkedAccountResolver {
    selection: Arc<dyn RuntimeCredentialAccountSelectionService>,
    sessions: Arc<LinkedSessionStore>,
    extension_id: ExtensionId,
    vendor: VendorId,
}

#[async_trait]
impl LinkedAccountResolver for CredentialLinkedAccountResolver {
    async fn resolve(
        &self,
        scope: &ResourceScope,
    ) -> Result<LinkedAccountGrant, LinkedAccountResolutionError> {
        let request = runtime_credential_account_selection_request(
            scope,
            &self.vendor,
            RuntimeCredentialAccountSetup::DeviceLink,
            &[],
            &self.extension_id,
        )
        .map_err(|error| {
            debug!(error = ?error, "linked-account selection request could not be built");
            LinkedAccountResolutionError::Unavailable
        })?;
        let account = self
            .selection
            .select_unique_configured_runtime_account(request)
            .await
            .map_err(|error| match error {
                AuthProductError::CredentialMissing
                | AuthProductError::AccountSelectionRequired => {
                    LinkedAccountResolutionError::NotLinked
                }
                other => {
                    debug!(error = %other, "linked-account selection failed");
                    LinkedAccountResolutionError::Unavailable
                }
            })?;
        // Defense in depth: the selection service already gates on the
        // device-link setup, but a grant must never be minted for an account
        // that is not a pinned linked device — that is the §4.5 ownership pin.
        if !account.is_linked_device() || !account.linked_device_ownership_is_pinned() {
            return Err(LinkedAccountResolutionError::NotLinked);
        }
        let account_ref = LinkedAccountRef::new(account.id.to_string()).map_err(|error| {
            debug!(%error, "credential account id does not form a linked-account ref");
            LinkedAccountResolutionError::Unavailable
        })?;
        // Teach custody where this ref's material lives before any handle
        // under the grant is opened. Registration is wiring, not authority:
        // the auth domain re-checks scope, requester, and revision on every
        // material operation.
        self.sessions.register_account(
            self.extension_id.clone(),
            account_ref.clone(),
            account.scope.clone(),
            account.id,
        );
        Ok(LinkedAccountGrant::new(account_ref, account.link_revision))
    }
}
