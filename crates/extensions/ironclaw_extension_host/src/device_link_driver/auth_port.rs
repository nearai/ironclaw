//! `ironclaw_auth::DeviceLinkDriver` over [`SnapshotDeviceLinkDriver`].
//!
//! The engine is in the parent module; this file is the projection between two
//! vocabularies and nothing else. Precedent for the direction of the seam:
//! [`crate::recipes::InstalledManifestAuthRecipeResolver`], which implements
//! the auth crate's recipe-resolver port the same way — auth declares, the
//! extension host implements, and auth never learns what an adapter is.
//!
//! It calls the parent's `*_at` entry points rather than the public inherent
//! methods on purpose: those flatten "nothing is bound" into the contracts
//! error vocabulary, and this port has a dedicated, non-restartable variant for
//! exactly that case.

use std::time::Instant;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthFlowId, DeviceLinkBeginRequest, DeviceLinkBinding, DeviceLinkCancelRequest,
    DeviceLinkDriver, DeviceLinkDriverError, DeviceLinkPollRequest, DeviceLinkStepOutcome,
    DeviceLinkSubmitRequest, LinkedDeviceRevokeError, LinkedDeviceRevokeRequest,
    LinkedDeviceRevoker,
};
use ironclaw_extension_contracts::device_link::{
    DeviceLinkError, DeviceLinkFlowId, DeviceLinkStep,
};
use ironclaw_extension_contracts::linked_session::{LinkedAccountGrant, LinkedAccountRef};

use super::{DeviceLinkRequest, DriverFailure, SnapshotDeviceLinkDriver};

#[async_trait]
impl DeviceLinkDriver for SnapshotDeviceLinkDriver {
    async fn begin(
        &self,
        request: DeviceLinkBeginRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError> {
        let driver_request = host_request(&request.flow_id, &request.binding)?;
        let step = self
            .begin_at(&driver_request, request.mode, Instant::now())
            .await
            .map_err(|failure| driver_error(&request.binding, failure))?;
        Ok(outcome(step))
    }

    async fn poll(
        &self,
        request: DeviceLinkPollRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError> {
        let driver_request = host_request(&request.flow_id, &request.binding)?;
        let step = self
            .poll_at(&driver_request, Instant::now())
            .await
            .map_err(|failure| driver_error(&request.binding, failure))?;
        Ok(outcome(step))
    }

    async fn submit(
        &self,
        request: DeviceLinkSubmitRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError> {
        let driver_request = host_request(&request.flow_id, &request.binding)?;
        let step = self
            .submit_input_at(&driver_request, request.input, Instant::now())
            .await
            .map_err(|failure| driver_error(&request.binding, failure))?;
        Ok(outcome(step))
    }

    async fn cancel(&self, request: DeviceLinkCancelRequest) -> Result<(), DeviceLinkDriverError> {
        let driver_request = host_request(&request.flow_id, &request.binding)?;
        // Every cancel reason converges here: the driver's job on an abort is
        // the same whoever pressed it — forget the flow and let the adapter
        // undo any authorization it already obtained. The reason is the auth
        // engine's record-keeping, not a behavior switch on this side.
        self.cancel_link(&driver_request)
            .await
            .map_err(|failure| driver_error(&request.binding, failure))
    }
}

/// Prefix for the synthetic flow id a revoke runs under.
///
/// A teardown is not a flow: there is no card, no payload, and nothing to
/// resume. The driver's request type still names one because every other entry
/// point has one, so this mints an id that cannot collide with a live link's
/// (which is an `AuthFlowId` rendering) and whose `forget` is a no-op.
const REVOKE_FLOW_PREFIX: &str = "revoke.";

#[async_trait]
impl LinkedDeviceRevoker for SnapshotDeviceLinkDriver {
    /// End the vendor authorization behind one established linked device.
    ///
    /// Called by `ironclaw_auth`'s cleanup **before** the credential is
    /// unbound, which is the only order in which it can work: after the unbind
    /// the session blob this call needs is deleted, and after the extension is
    /// removed the adapter that could make the vendor call is gone.
    async fn revoke_linked_device(
        &self,
        request: LinkedDeviceRevokeRequest,
    ) -> Result<(), LinkedDeviceRevokeError> {
        // Revision 0 is the provisional (pre-mint) grant a link in progress
        // holds. Accepting it here would open a custody handle onto a
        // half-established link and ask a vendor to log out of a device that
        // was never authorized.
        if request.link_revision == 0 {
            return Err(LinkedDeviceRevokeError::Unavailable);
        }
        let flow_id = DeviceLinkFlowId::new(format!(
            "{REVOKE_FLOW_PREFIX}{account}",
            account = request.account_id
        ))
        .map_err(|error| {
            tracing::debug!(%error, "linked-device revoke id does not form a flow id");
            LinkedDeviceRevokeError::Unavailable
        })?;
        let account = LinkedAccountRef::new(request.account_id.to_string()).map_err(|error| {
            tracing::debug!(%error, "credential account id does not form a linked-account ref");
            LinkedDeviceRevokeError::Unavailable
        })?;
        let driver_request = DeviceLinkRequest {
            flow_id,
            extension_id: request.extension_id,
            // The owner of the durable credential record, never a value an
            // adapter or a card supplied.
            user_id: request.scope.resource.user_id.clone(),
            account: Some(LinkedAccountGrant::new(account, request.link_revision)),
        };
        self.revoke_link(&driver_request)
            .await
            .map_err(revoke_error)
    }
}

/// Project a driver failure onto the revoker port's closed vocabulary.
///
/// Every variant quarantines the same way on the auth side, so this only has to
/// stay honest about *which* half failed: nothing was bound, the vendor
/// refused, or the host could not mediate the call at all.
fn revoke_error(failure: DriverFailure) -> LinkedDeviceRevokeError {
    match failure {
        DriverFailure::NoBinding => LinkedDeviceRevokeError::NoBinding,
        DriverFailure::Link(DeviceLinkError::Vendor { .. } | DeviceLinkError::UnknownFlow) => {
            LinkedDeviceRevokeError::Vendor
        }
        DriverFailure::Link(_) => LinkedDeviceRevokeError::Unavailable,
    }
}

/// Project an auth-side call onto the host driver's request.
///
/// The account is always `None`: this port drives a link that is being
/// *established*, and there is no credential account until custody is durable.
fn host_request(
    flow_id: &AuthFlowId,
    binding: &DeviceLinkBinding,
) -> Result<DeviceLinkRequest, DeviceLinkDriverError> {
    // An `AuthFlowId` is a UUID, so its rendering always satisfies the flow-id
    // grammar; the fallible constructor is still handled rather than unwrapped,
    // and the bound cause is logged because the port's error carries no text.
    let flow_id = DeviceLinkFlowId::new(flow_id.to_string()).map_err(|error| {
        tracing::debug!(%error, "auth flow id does not form a device-link flow id");
        DeviceLinkDriverError::Unavailable
    })?;
    Ok(DeviceLinkRequest {
        flow_id,
        extension_id: binding.extension_id.clone(),
        user_id: binding.user_id.clone(),
        account: None,
    })
}

/// Wrap a step as the port's outcome.
///
/// TODO(design): a `Completed` step must carry the minted credential account,
/// and this crate cannot mint one — see the module header of the parent. Until
/// that is reconciled, a completion reports `account: None`, which the
/// auth-side driver terminalizes rather than announcing a link it cannot back.
/// When the minting seam lands it must build the account through
/// `ironclaw_auth::NewCredentialAccount::for_linked_device`, which is where the
/// §4.5 ownership pin lives.
fn outcome(step: DeviceLinkStep) -> DeviceLinkStepOutcome {
    DeviceLinkStepOutcome {
        step,
        account: None,
    }
}

/// Project a driver failure onto the auth port's sanitized vocabulary.
///
/// Nothing vendor-authored crosses: [`DeviceLinkError`]'s free text is
/// `&'static str` host-authored phrasing, and only the closed code vocabulary
/// travels.
fn driver_error(binding: &DeviceLinkBinding, failure: DriverFailure) -> DeviceLinkDriverError {
    let error = match failure {
        DriverFailure::NoBinding => {
            return DeviceLinkDriverError::NoBinding {
                provider: binding.provider.clone(),
            };
        }
        DriverFailure::Link(error) => error,
    };
    match error {
        DeviceLinkError::UnknownFlow => DeviceLinkDriverError::UnknownFlow,
        DeviceLinkError::Custody(_) => DeviceLinkDriverError::Custody,
        other => DeviceLinkDriverError::Vendor {
            code: other.code(),
            restartable: other.restartable(),
        },
    }
}
