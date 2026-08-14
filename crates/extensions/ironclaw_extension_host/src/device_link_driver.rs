//! The host half of a device-link flow: resolve the bound adapter, hand it a
//! pre-scoped context, and enforce every bound the vendor half must not own.
//!
//! # Where this sits
//!
//! `ironclaw_auth` owns the durable flow record, its revision compare-and-swap,
//! the step→challenge projection, and the credential lifecycle. The extension
//! package owns the protocol conversation. **This module is the glue**: it
//! resolves `extension → bound adapter` from the published active snapshot,
//! builds the [`DeviceLinkContext`] the adapter is allowed to see, and applies
//! the limits and TTLs host-side. Precedent: [`crate::recipes`], which
//! implements the auth crate's recipe-resolver port the same way.
//!
//! # Why the limits live here and not in the adapter
//!
//! The adapter is the untrusted half of an auth flow — that is the whole point
//! of the ADR (`ADR-device-link-auth-hook.md`, in this feature's design record
//! under `docs/internal/design/`).
//! Asking it to rate-limit itself would place the control inside the code the
//! control exists to bound. Concretely, nothing else limits a vendor's
//! "send me a login code" call, so an unbounded identifier path is a
//! harassment-amplification vector *and* burns the flood budget every later
//! call depends on. So the host owns:
//!
//! * **A poll floor.** A too-early poll is answered with `AwaitingVendor`
//!   without the adapter being called at all.
//! * **Flow and step TTLs**, with the ordering `flow_ttl ≥ step_ttl ≥
//!   min_poll_interval` checked at construction. A step an adapter says is
//!   valid for an hour is clamped to what the flow has left.
//! * **Begin budgets** per user and per deployment, a cap on distinct
//!   identifiers one user may submit, and a cap on secret attempts per flow
//!   (an unbounded password retry is an account-lockout vector).
//! * **Cancel-on-expiry.** A reaped flow gets an awaited `cancel`, because once
//!   a vendor has authorized the device, walking away leaves a live
//!   authorization nobody remembers. `Drop` cannot make that call.
//!
//! # Two faces, one engine
//!
//! [`SnapshotDeviceLinkDriver`] implements `ironclaw_auth`'s
//! [`DeviceLinkDriver`](ironclaw_auth::DeviceLinkDriver) port — the seam the
//! auth step machine drives — and also exposes the same five operations as
//! inherent methods in the contracts vocabulary. The inherent set is not a
//! duplicate surface: `revoke` has no leg on the port (unlink runs through
//! extension credential cleanup), and the inherent methods are what makes the
//! engine testable without constructing auth flow records.
//!
//! # Completion mints the credential account here
//!
//! `DeviceLinkStepOutcome` requires `account: Some(..)` on a `Completed`
//! step. The flow's `AuthProductScope` rides `DeviceLinkBinding` (and this
//! module's own `DeviceLinkRequest`) from the durable flow record, so the
//! settle path can hand the provisional session blob to the auth domain's one
//! completion operation, `CredentialAccountService::complete_linked_device_link`
//! — which owns the create-or-reuse policy, the §4.5 ownership pin (enforced
//! again by `bump_link_revision`, so a hand-rolled literal cannot complete a
//! link), and the load-then-CAS blob write. The mint runs strictly before the
//! flow is forgotten, because forgetting discards the provisional blob the
//! mint reads.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ironclaw_extension_contracts::device_link::{
    DeviceLinkAdapter, DeviceLinkContext, DeviceLinkError, DeviceLinkErrorCode, DeviceLinkFlowId,
    DeviceLinkInput, DeviceLinkMode, DeviceLinkStep,
};
use ironclaw_extension_contracts::linked_session::{
    LinkedAccountGrant, LinkedAccountRef, LinkedSessionPort,
};
use ironclaw_host_api::ids::{ExtensionId, UserId};
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::device_link_channel_identity::{
    DeviceLinkChannelIdentityBinder, DeviceLinkChannelIdentityError,
};
use crate::entrypoint::declared_device_link_recipe;
use crate::lifecycle::SnapshotWatch;
use crate::linked_session_custody::LinkedSessionStore;
use crate::linked_session_custody::PENDING_LINK_REVISION;

/// Account-ref prefix for a link that has no credential account yet.
///
/// The blob is stored *before* the account is minted (PROPOSAL §4.3:
/// "store blob → mint account → report completed"), so the adapter needs a
/// scoped handle during the handshake. The host mints this provisional ref
/// from the flow id; the completion mint reads the blob back through the same
/// ref ([`provisional_account_ref`] is the single source of the string) and
/// stores it durably under the minted account before the flow — and with it
/// the provisional blob — is forgotten.
const PENDING_ACCOUNT_PREFIX: &str = "pending-link.";

/// Host-side bounds for device-link flows. Every field is a declared constant
/// with a test, not a magic number at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceLinkLimits {
    /// How long one flow may stay non-terminal before it is cancelled.
    pub flow_ttl: Duration,
    /// The longest a displayed payload may claim to be valid.
    pub step_ttl: Duration,
    /// The floor between two adapter polls for one flow.
    pub min_poll_interval: Duration,
    /// The window the begin/identifier budgets are counted over.
    pub rate_window: Duration,
    /// Begins one user may start per window.
    pub max_begins_per_user: u32,
    /// Begins the whole deployment may start per window.
    pub max_begins_per_deployment: u32,
    /// Distinct identifiers (phone numbers, handles) one user may submit per
    /// window, counted by digest so the values themselves are never retained.
    pub max_identifiers_per_user: u32,
    /// Code/password submissions allowed on one flow.
    pub max_secret_attempts_per_flow: u32,
    /// Non-terminal flows the driver tracks at once.
    pub max_active_flows: usize,
}

impl DeviceLinkLimits {
    /// The default ceiling on concurrent non-terminal flows.
    ///
    /// Public because custody's provisional-blob cap must equal it: a
    /// provisional blob exists only while a flow is mid-handshake, so a cap
    /// below this one would reject a blob for a flow the driver still admits.
    /// The two used to agree by comment only.
    pub const DEFAULT_MAX_ACTIVE_FLOWS: usize = 64;
}

impl Default for DeviceLinkLimits {
    fn default() -> Self {
        Self {
            flow_ttl: Duration::from_secs(10 * 60),
            step_ttl: Duration::from_secs(60),
            min_poll_interval: Duration::from_secs(2),
            rate_window: Duration::from_secs(60 * 60),
            max_begins_per_user: 10,
            max_begins_per_deployment: 100,
            max_identifiers_per_user: 3,
            max_secret_attempts_per_flow: 5,
            max_active_flows: Self::DEFAULT_MAX_ACTIVE_FLOWS,
        }
    }
}

/// The clock ordering three separate TTLs would otherwise disagree about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceLinkLimitsError {
    #[error("device-link step TTL must not exceed the flow TTL")]
    StepTtlAboveFlowTtl,
    #[error("device-link poll floor must not exceed the step TTL")]
    PollFloorAboveStepTtl,
    #[error("device-link limits must all be non-zero")]
    ZeroBound,
}

impl DeviceLinkLimits {
    pub fn validate(&self) -> Result<(), DeviceLinkLimitsError> {
        if self.flow_ttl.is_zero()
            || self.step_ttl.is_zero()
            || self.min_poll_interval.is_zero()
            || self.rate_window.is_zero()
            || self.max_begins_per_user == 0
            || self.max_begins_per_deployment == 0
            || self.max_identifiers_per_user == 0
            || self.max_secret_attempts_per_flow == 0
            || self.max_active_flows == 0
        {
            return Err(DeviceLinkLimitsError::ZeroBound);
        }
        if self.step_ttl > self.flow_ttl {
            return Err(DeviceLinkLimitsError::StepTtlAboveFlowTtl);
        }
        if self.min_poll_interval > self.step_ttl {
            return Err(DeviceLinkLimitsError::PollFloorAboveStepTtl);
        }
        Ok(())
    }
}

/// One call into the driver: which flow, whose, and against which account.
///
/// `scope` is the durable flow's own product scope, carried whole because the
/// completion mint needs it — synthesizing one from a bare user id would be
/// re-deriving security-relevant scope. The flow's owner is
/// `scope.resource.user_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkRequest {
    pub flow_id: DeviceLinkFlowId,
    pub extension_id: ExtensionId,
    pub scope: ironclaw_auth::AuthProductScope,
    /// The established account, when one exists. `None` during a link that has
    /// not completed — there is no credential account until custody is durable.
    pub account: Option<LinkedAccountGrant>,
}

impl DeviceLinkRequest {
    fn user_id(&self) -> &UserId {
        &self.scope.resource.user_id
    }
}

/// A settled transition: the bounded step, plus the credential account the
/// completion minted (present exactly when `step` is `Completed`).
#[derive(Debug)]
pub struct SettledDeviceLinkStep {
    pub step: DeviceLinkStep,
    pub account: Option<ironclaw_auth::DeviceLinkLinkedAccount>,
}

/// Resolves device-link adapters from the published snapshot and drives them
/// under host-owned limits.
pub struct SnapshotDeviceLinkDriver {
    snapshots: SnapshotWatch,
    sessions: Arc<LinkedSessionStore>,
    limits: DeviceLinkLimits,
    state: Mutex<DriverState>,
    /// The auth domain's credential service, for the completion mint: a
    /// `Completed` step must be backed by a minted account before it is
    /// reported, and `complete_linked_device_link` is the one place that
    /// policy lives (the §4.5 ownership pin is inside it).
    accounts: Arc<dyn ironclaw_auth::CredentialAccountService>,
    channel_identities: DeviceLinkChannelIdentityBinder,
}

impl SnapshotDeviceLinkDriver {
    pub fn new(
        snapshots: SnapshotWatch,
        sessions: Arc<LinkedSessionStore>,
        limits: DeviceLinkLimits,
        accounts: Arc<dyn ironclaw_auth::CredentialAccountService>,
        channel_identities: Arc<crate::FilesystemChannelIdentityStore>,
    ) -> Result<Self, DeviceLinkLimitsError> {
        limits.validate()?;
        Ok(Self {
            snapshots,
            sessions,
            limits,
            state: Mutex::new(DriverState::default()),
            accounts,
            channel_identities: DeviceLinkChannelIdentityBinder::new(channel_identities),
        })
    }

    pub fn limits(&self) -> DeviceLinkLimits {
        self.limits
    }

    /// The custody store this driver parks provisional blobs in and registers
    /// minted accounts with. For the sibling auth-port module only.
    pub(crate) fn sessions(&self) -> &Arc<LinkedSessionStore> {
        &self.sessions
    }

    /// Start a link on the chosen path.
    pub async fn begin(
        &self,
        request: &DeviceLinkRequest,
        mode: DeviceLinkMode,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.begin_at(request, mode, Instant::now())
            .await
            .map(|settled| settled.step)
            .map_err(DriverFailure::into_link_error)
    }

    /// Advance a link that is waiting on the vendor.
    pub async fn poll(
        &self,
        request: &DeviceLinkRequest,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.poll_at(request, Instant::now())
            .await
            .map(|settled| settled.step)
            .map_err(DriverFailure::into_link_error)
    }

    /// Submit one value the previous step asked for.
    pub async fn submit_input(
        &self,
        request: &DeviceLinkRequest,
        input: DeviceLinkInput,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.submit_input_at(request, input, Instant::now())
            .await
            .map(|settled| settled.step)
            .map_err(DriverFailure::into_link_error)
    }

    /// Abandon an in-progress link, undoing any authorization it obtained.
    pub async fn cancel(&self, request: &DeviceLinkRequest) -> Result<(), DeviceLinkError> {
        self.cancel_link(request)
            .await
            .map_err(DriverFailure::into_link_error)
    }

    /// Tear down an established link.
    ///
    /// Unlike every other entry point this one requires a real account:
    /// revoking "the account that does not exist yet" is a caller bug, not a
    /// vendor outcome. Deliberately **not** part of `ironclaw_auth`'s
    /// `DeviceLinkDriver` port, which has no revoke leg — unlink runs through
    /// extension credential cleanup, and this is the entry point it calls.
    pub async fn revoke(&self, request: &DeviceLinkRequest) -> Result<(), DeviceLinkError> {
        self.revoke_link(request)
            .await
            .map_err(DriverFailure::into_link_error)
    }

    /// The revoke engine, kept separate from the public face for the same
    /// reason `cancel_link` is: "no extension binds an adapter" is a different
    /// answer from "the vendor refused", and a caller that can act on the
    /// difference (the linked-device revoker port) must not be handed a
    /// flattened one.
    async fn revoke_link(&self, request: &DeviceLinkRequest) -> Result<(), DriverFailure> {
        if request.account.is_none() {
            return Err(DeviceLinkError::Internal {
                reason: "device-link revoke requires an established account grant",
            }
            .into());
        }
        let resolved = self.resolve(&request.extension_id)?;
        self.forget(&request.extension_id, &request.flow_id);
        let grant = self.custody_grant(request)?;
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, &resolved, session.as_ref());
        Ok(resolved.adapter.revoke(&context).await?)
    }

    async fn cancel_link(&self, request: &DeviceLinkRequest) -> Result<(), DriverFailure> {
        let resolved = self.resolve(&request.extension_id)?;
        let grant = self.custody_grant(request)?;
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, &resolved, session.as_ref());
        let outcome = resolved.adapter.cancel(&context).await;
        self.forget(&request.extension_id, &request.flow_id);
        Ok(outcome?)
    }

    async fn begin_at(
        &self,
        request: &DeviceLinkRequest,
        mode: DeviceLinkMode,
        now: Instant,
    ) -> Result<SettledDeviceLinkStep, DriverFailure> {
        let resolved = self.resolve(&request.extension_id)?;
        // The recipe, not the adapter, says which paths exist. Checking here
        // means an extension that declares no fallback cannot be talked into
        // running one.
        if mode == DeviceLinkMode::Alternate && !resolved.alternate_mode_declared {
            return Err(DeviceLinkError::UnsupportedMode { mode }.into());
        }

        self.reap_expired(now).await;

        // A `begin` naming a flow the host already knows is a RE-MINT, not a
        // second attempt: the auth tier's step clock lapsed and it wants a
        // fresh frame for the same link (its `mint_step` is documented as
        // "used both to start a link and to re-mint one"). Refusing it outright
        // — which this driver used to do — terminalized every link whose first
        // frame lapsed, because the host flow TTL outlives the step TTL by an
        // order of magnitude, so the flow is always still live at a lapse.
        //
        // Drop the stale vendor conversation FIRST, which is the hazard the
        // refusal was really guarding: an adapter must never run two
        // conversations for one flow. Then re-admit under the same flow, and
        // carry the previous attempt's clock and secret-attempt count forward
        // so a re-mint can neither extend the attempt nor reset an abuse
        // counter.
        let resumed = self.resumable_flow(&request.flow_id);
        if resumed.is_some() {
            self.cancel_flow(request, &resolved).await;
        }
        self.admit_begin(request, now, resumed)?;

        let grant = self.custody_grant(request)?;
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, &resolved, session.as_ref());
        let outcome = resolved.adapter.begin(&context, mode).await;
        self.settle(request, &resolved, outcome, now).await
    }

    async fn poll_at(
        &self,
        request: &DeviceLinkRequest,
        now: Instant,
    ) -> Result<SettledDeviceLinkStep, DriverFailure> {
        let resolved = self.resolve(&request.extension_id)?;
        match self.admit_poll(&request.flow_id, now) {
            PollAdmission::Unknown => Err(DeviceLinkError::UnknownFlow.into()),
            // A flow the host has already given up on is cancelled before it is
            // reported, so a vendor-side authorization cannot outlive it.
            PollAdmission::Expired => {
                self.cancel_flow(request, &resolved).await;
                Ok(SettledDeviceLinkStep {
                    step: expired_step(),
                    account: None,
                })
            }
            // The adapter is not called at all: a hot-looping card must not be
            // able to turn into vendor traffic.
            PollAdmission::TooSoon { retry_in } => Ok(SettledDeviceLinkStep {
                step: DeviceLinkStep::AwaitingVendor { retry_in },
                account: None,
            }),
            PollAdmission::Admitted => {
                let grant = self.custody_grant(request)?;
                let session = self.sessions.open(&request.extension_id, &grant);
                let context = self.context(request, &resolved, session.as_ref());
                let outcome = resolved.adapter.poll(&context).await;
                self.settle(request, &resolved, outcome, now).await
            }
        }
    }

    async fn submit_input_at(
        &self,
        request: &DeviceLinkRequest,
        input: DeviceLinkInput,
        now: Instant,
    ) -> Result<SettledDeviceLinkStep, DriverFailure> {
        // Bound the paste before anything vendor-shaped sees it.
        input.validate()?;
        let resolved = self.resolve(&request.extension_id)?;
        match self.admit_input(request, &input, now)? {
            InputAdmission::Expired => {
                self.cancel_flow(request, &resolved).await;
                return Ok(SettledDeviceLinkStep {
                    step: expired_step(),
                    account: None,
                });
            }
            InputAdmission::Admitted => {}
        }

        let grant = self.custody_grant(request)?;
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, &resolved, session.as_ref());
        let outcome = resolved.adapter.submit_input(&context, input).await;
        self.settle(request, &resolved, outcome, now).await
    }

    /// Resolve the bound adapter for an extension from the published snapshot.
    fn resolve(&self, extension_id: &ExtensionId) -> Result<ResolvedDeviceLink, DriverFailure> {
        let snapshot = self.snapshots.current();
        let binding = snapshot
            .resolve_device_link(extension_id)
            .ok_or(DriverFailure::NoBinding)?;
        // `check_binding` refuses an adapter without a declared surface, so a
        // resolved binding whose recipe is missing means the snapshot and the
        // binding rule disagree — fail rather than guess the mode set.
        let recipe =
            declared_device_link_recipe(&binding.declaration).ok_or(DeviceLinkError::Internal {
                reason: "bound device-link adapter has no declared device-link recipe",
            })?;
        let alternate_mode_declared = recipe.alternate_mode_label.is_some();
        Ok(ResolvedDeviceLink {
            adapter: binding.adapter,
            declaration: binding.declaration,
            installation_id: binding.installation_id,
            config: binding.config,
            alternate_mode_declared,
        })
    }

    fn context<'a>(
        &self,
        request: &'a DeviceLinkRequest,
        resolved: &'a ResolvedDeviceLink,
        session: &'a dyn LinkedSessionPort,
    ) -> DeviceLinkContext<'a> {
        DeviceLinkContext {
            flow_id: &request.flow_id,
            extension_id: &request.extension_id,
            user_id: request.user_id(),
            config: resolved.config.as_ref(),
            session,
            account: request.account.as_ref(),
        }
    }

    /// The grant a custody handle is scoped to for this call.
    fn custody_grant(
        &self,
        request: &DeviceLinkRequest,
    ) -> Result<LinkedAccountGrant, DeviceLinkError> {
        if let Some(grant) = &request.account {
            return Ok(grant.clone());
        }
        match provisional_account_ref(&request.flow_id) {
            Some(account) => Ok(LinkedAccountGrant::new(account, PENDING_LINK_REVISION)),
            None => {
                // `DeviceLinkFlowId` already rejects whitespace, control bytes,
                // and anything over 128 bytes, so this is unreachable in
                // practice.
                Err(DeviceLinkError::Internal {
                    reason: "device-link flow id does not form a valid account ref",
                })
            }
        }
    }

    /// Record an adapter outcome: bound its display text, clamp its clocks,
    /// mint the credential account behind a completion, and drop the flow once
    /// it stops advancing.
    async fn settle(
        &self,
        request: &DeviceLinkRequest,
        resolved: &ResolvedDeviceLink,
        outcome: Result<DeviceLinkStep, DeviceLinkError>,
        now: Instant,
    ) -> Result<SettledDeviceLinkStep, DriverFailure> {
        let step = match outcome {
            Ok(step) => step,
            Err(error) => {
                // A failed transition may have happened after the vendor
                // accepted a login. Tear the vendor conversation down before
                // forgetting it; dropping only the host record strands a
                // device authorization neither side can address afterwards.
                self.cancel_flow(request, resolved).await;
                return Err(error.into());
            }
        };
        if let Err(error) = step.validate() {
            self.cancel_flow(request, resolved).await;
            return Err(error.into());
        }
        let remaining = self.remaining(&request.flow_id, now);
        let step = self.clamp(step, remaining);
        let account = if matches!(step, DeviceLinkStep::Completed { .. }) {
            // Mint BEFORE forgetting: `forget` discards the provisional blob
            // the mint reads. A completion the mint cannot back is reported as
            // a custody failure, never as a completion.
            match self.mint_completed_account(request, &step).await {
                Ok(account) => {
                    self.finalize_vendor_side(request, resolved).await;
                    Some(account)
                }
                Err(failure) => {
                    self.cancel_flow(request, resolved).await;
                    return Err(failure);
                }
            }
        } else {
            None
        };
        if step.is_terminal() {
            self.forget(&request.extension_id, &request.flow_id);
        }
        Ok(SettledDeviceLinkStep { step, account })
    }

    /// The completion mint: read the provisional blob the adapter stored
    /// during the handshake, hand it to the auth domain's one completion
    /// operation (which owns the create-or-reuse policy, the ownership pin,
    /// and the load-then-CAS blob write), and teach custody where the minted
    /// account's material lives.
    async fn mint_completed_account(
        &self,
        request: &DeviceLinkRequest,
        step: &DeviceLinkStep,
    ) -> Result<ironclaw_auth::DeviceLinkLinkedAccount, DriverFailure> {
        let DeviceLinkStep::Completed {
            account_label,
            vendor_user_ref,
        } = step
        else {
            return Err(DeviceLinkError::Internal {
                reason: "device-link mint requires a completed step",
            }
            .into());
        };
        let provisional = provisional_account_ref(&request.flow_id).ok_or_else(|| {
            DriverFailure::from(DeviceLinkError::Internal {
                reason: "device-link flow id does not form a valid account ref",
            })
        })?;
        let Some(blob) = self
            .sessions
            .provisional_blob(&request.extension_id, &provisional)
        else {
            // The adapter reported completion without ever storing a session.
            // Nothing can serve the link; refusing here is what keeps the
            // store-blob → mint → report ordering honest.
            debug!("device-link completion arrived with no provisional session blob");
            return Err(DeviceLinkError::Custody(
                ironclaw_extension_contracts::linked_session::LinkedSessionError::Unavailable {
                    reason: "device-link completion stored no session material",
                },
            )
            .into());
        };
        let vendor_user_ref = ironclaw_auth::DeviceLinkVendorUserRef::new(vendor_user_ref.clone())
            .map_err(|error| {
                debug!(%error, "device-link vendor user ref failed validation");
                DriverFailure::from(DeviceLinkError::InvalidStep {
                    reason: "completed step carries an invalid vendor user ref",
                })
            })?;
        let provider = self.device_link_provider(&request.extension_id)?;
        let resolved = self.resolve(&request.extension_id)?;
        let label = linked_account_label(account_label)?;
        let rollback = self
            .channel_identities
            .begin(
                resolved.declaration.as_ref(),
                &resolved.installation_id,
                provider.as_str(),
                vendor_user_ref.as_str(),
                request.user_id(),
            )
            .await
            .map_err(map_channel_identity_error)?;
        let account_result = self
            .accounts
            .complete_linked_device_link(ironclaw_auth::LinkedDeviceLinkCompletion {
                scope: request.scope.clone(),
                provider,
                owner_extension: request.extension_id.clone(),
                label,
                material: blob,
            })
            .await;
        let account = match account_result {
            Ok(account) => account,
            Err(error) => {
                debug!(error = %error, "device-link completion mint failed");
                if let Some(rollback) = rollback
                    && let Err(rollback_error) = rollback.rollback().await
                {
                    debug!(error = %rollback_error, "device-link channel identity rollback failed");
                }
                return Err(DriverFailure::from(DeviceLinkError::Custody(
                    ironclaw_extension_contracts::linked_session::LinkedSessionError::Unavailable {
                        reason: "device-link credential account could not be minted",
                    },
                )));
            }
        };
        let account_ref = LinkedAccountRef::new(account.id.to_string()).map_err(|error| {
            debug!(%error, "minted account id does not form a linked-account ref");
            DriverFailure::from(DeviceLinkError::Internal {
                reason: "minted credential account id does not form an account ref",
            })
        })?;
        self.sessions.register_account(
            request.extension_id.clone(),
            account_ref,
            account.scope.clone(),
            account.id,
        );
        Ok(ironclaw_auth::DeviceLinkLinkedAccount {
            account_id: account.id,
            label: account.label.clone(),
            vendor_user_ref,
            link_revision: account.link_revision,
        })
    }

    /// The vendor id behind the extension's device-link auth surface, as an
    /// auth provider id. Read from the resolved manifest — never a name in
    /// host code.
    fn device_link_provider(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<ironclaw_auth::AuthProviderId, DriverFailure> {
        let snapshot = self.snapshots.current();
        let binding = snapshot
            .resolve_device_link(extension_id)
            .ok_or(DriverFailure::NoBinding)?;
        let vendor = binding.declaration.auth.iter().find_map(|surface| {
            matches!(
                surface.recipe,
                Some(ironclaw_extension_contracts::recipe::VendorAuthRecipe::DeviceLink(_))
            )
            .then(|| surface.vendor.clone())
        });
        let Some(vendor) = vendor else {
            return Err(DeviceLinkError::Internal {
                reason: "bound device-link adapter has no declared device-link recipe",
            }
            .into());
        };
        ironclaw_auth::AuthProviderId::new(vendor.as_str()).map_err(|error| {
            debug!(%error, "device-link vendor id does not form an auth provider id");
            DriverFailure::from(DeviceLinkError::Internal {
                reason: "device-link vendor id does not form an auth provider id",
            })
        })
    }

    /// Clamp adapter-declared durations into the host's clocks. A vendor cannot
    /// extend a flow by claiming a longer window than the host granted.
    fn clamp(&self, step: DeviceLinkStep, remaining: Duration) -> DeviceLinkStep {
        match step {
            DeviceLinkStep::Display {
                kind,
                payload,
                expires_in,
            } => DeviceLinkStep::Display {
                kind,
                payload,
                expires_in: expires_in.min(self.limits.step_ttl).min(remaining),
            },
            DeviceLinkStep::AwaitingVendor { retry_in } => DeviceLinkStep::AwaitingVendor {
                retry_in: retry_in.max(self.limits.min_poll_interval).min(remaining),
            },
            other => other,
        }
    }

    /// Accept a provisional vendor completion after all host-side state is
    /// durable. The adapter owns no fallible work here; it only relinquishes
    /// rollback state.
    async fn finalize_vendor_side(
        &self,
        request: &DeviceLinkRequest,
        resolved: &ResolvedDeviceLink,
    ) {
        let Ok(grant) = self.custody_grant(request) else {
            return;
        };
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, resolved, session.as_ref());
        resolved.adapter.finalize(&context).await;
    }

    /// Cancel one flow's vendor-side state, best effort, then forget its host
    /// state. Keeping the provisional blob available until cancellation lets
    /// adapters use the same custody context they completed with.
    async fn cancel_flow(&self, request: &DeviceLinkRequest, resolved: &ResolvedDeviceLink) {
        self.cancel_vendor_side(request, resolved).await;
        self.forget(&request.extension_id, &request.flow_id);
    }

    /// The vendor half of [`Self::cancel_flow`], without forgetting the flow.
    async fn cancel_vendor_side(&self, request: &DeviceLinkRequest, resolved: &ResolvedDeviceLink) {
        let Ok(grant) = self.custody_grant(request) else {
            return;
        };
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, resolved, session.as_ref());
        if let Err(error) = resolved.adapter.cancel(&context).await {
            // Best effort by contract: the flow is gone either way, and the
            // residual (a vendor authorization we asked to drop and could not
            // confirm) is disclosed rather than retried here.
            debug!(code = ?error.code(), "device-link cancel on expiry failed");
        }
    }

    /// Cancel every flow whose TTL has elapsed. Called on `begin`, which is the
    /// only path that can grow the map.
    async fn reap_expired(&self, now: Instant) {
        let expired = {
            let mut state = self.lock();
            state.evict_spent_budgets(now, self.limits.rate_window);
            let ttl = self.limits.flow_ttl;
            let expired: Vec<(DeviceLinkFlowId, ExpiredFlow)> = state
                .flows
                .iter()
                .filter(|(_, flow)| now.saturating_duration_since(flow.started_at) >= ttl)
                .map(|(id, flow)| {
                    (
                        id.clone(),
                        ExpiredFlow {
                            extension_id: flow.extension.clone(),
                            scope: flow.scope.clone(),
                        },
                    )
                })
                .collect();
            for (id, _) in &expired {
                state.flows.remove(id);
            }
            expired
        };
        for (flow_id, flow) in expired {
            let Ok(resolved) = self.resolve(&flow.extension_id) else {
                continue;
            };
            let request = DeviceLinkRequest {
                flow_id,
                extension_id: flow.extension_id,
                scope: flow.scope,
                account: None,
            };
            self.cancel_flow(&request, &resolved).await;
        }
    }

    /// The per-user budget key for a request: user AND extension.
    fn budget_key(request: &DeviceLinkRequest) -> BudgetKey {
        (request.user_id().clone(), request.extension_id.clone())
    }

    /// The clock and abuse counters a re-mint must carry forward, snapshotted
    /// before the stale conversation is cancelled.
    fn resumable_flow(&self, flow_id: &DeviceLinkFlowId) -> Option<ResumedFlow> {
        self.lock().flows.get(flow_id).map(|flow| ResumedFlow {
            started_at: flow.started_at,
            secret_attempts: flow.secret_attempts,
        })
    }

    /// Admit a `begin`. `resumed` carries the previous attempt when this is a
    /// re-mint of a flow that is already live (see [`Self::begin_at`]).
    ///
    /// A re-mint deliberately skips the begin budgets and the active-flow cap:
    /// both exist to bound how many *attempts* a user or a deployment may
    /// start, and a re-mint is the same attempt asking for a fresh frame. With
    /// a 60s step clock inside a 600s flow clock, charging each re-mint would
    /// let one attended link spend an entire hourly budget. What a re-mint may
    /// never do is buy time or forgiveness, so the flow clock and the
    /// secret-attempt count come across unchanged.
    fn admit_begin(
        &self,
        request: &DeviceLinkRequest,
        now: Instant,
        resumed: Option<ResumedFlow>,
    ) -> Result<(), DeviceLinkError> {
        let mut state = self.lock();
        if resumed.is_none() {
            if state.flows.len() >= self.limits.max_active_flows {
                return Err(limit_reached());
            }
            let window = self.limits.rate_window;
            if !state
                .deployment_begins
                .admit(now, window, self.limits.max_begins_per_deployment)
            {
                return Err(host_throttled());
            }
            let per_user = state.begins.entry(Self::budget_key(request)).or_default();
            if !per_user.admit(now, window, self.limits.max_begins_per_user) {
                return Err(host_throttled());
            }
        }
        state.flows.insert(
            request.flow_id.clone(),
            FlowState {
                extension: request.extension_id.clone(),
                scope: request.scope.clone(),
                started_at: resumed.map(|r| r.started_at).unwrap_or(now),
                last_poll_at: None,
                secret_attempts: resumed.map(|r| r.secret_attempts).unwrap_or(0),
            },
        );
        Ok(())
    }

    fn admit_poll(&self, flow_id: &DeviceLinkFlowId, now: Instant) -> PollAdmission {
        let mut state = self.lock();
        let Some(flow) = state.flows.get_mut(flow_id) else {
            return PollAdmission::Unknown;
        };
        if now.saturating_duration_since(flow.started_at) >= self.limits.flow_ttl {
            return PollAdmission::Expired;
        }
        if let Some(last) = flow.last_poll_at {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < self.limits.min_poll_interval {
                return PollAdmission::TooSoon {
                    retry_in: self.limits.min_poll_interval - elapsed,
                };
            }
        }
        flow.last_poll_at = Some(now);
        PollAdmission::Admitted
    }

    fn admit_input(
        &self,
        request: &DeviceLinkRequest,
        input: &DeviceLinkInput,
        now: Instant,
    ) -> Result<InputAdmission, DeviceLinkError> {
        let mut state = self.lock();
        let Some(flow) = state.flows.get_mut(&request.flow_id) else {
            return Err(DeviceLinkError::UnknownFlow);
        };
        if now.saturating_duration_since(flow.started_at) >= self.limits.flow_ttl {
            return Ok(InputAdmission::Expired);
        }
        match input {
            DeviceLinkInput::Identifier(value) => {
                let digest = identifier_digest(value);
                let budget = state
                    .identifiers
                    .entry(Self::budget_key(request))
                    .or_default();
                if !budget.admit(
                    now,
                    self.limits.rate_window,
                    self.limits.max_identifiers_per_user,
                    digest,
                ) {
                    return Err(host_throttled());
                }
            }
            DeviceLinkInput::Code(_) | DeviceLinkInput::Password(_) => {
                if flow.secret_attempts >= self.limits.max_secret_attempts_per_flow {
                    return Err(host_throttled());
                }
                flow.secret_attempts += 1;
            }
        }
        Ok(InputAdmission::Admitted)
    }

    /// How much of the flow's TTL is left, for clamping an adapter's clocks.
    /// A flow the driver no longer tracks has the full step budget: the answer
    /// is about to be terminal anyway.
    fn remaining(&self, flow_id: &DeviceLinkFlowId, now: Instant) -> Duration {
        let state = self.lock();
        match state.flows.get(flow_id) {
            Some(flow) => self
                .limits
                .flow_ttl
                .saturating_sub(now.saturating_duration_since(flow.started_at)),
            None => self.limits.step_ttl,
        }
    }

    /// Drop a flow's driver state and its parked provisional blob. Runs after
    /// the completion mint, which is the one reader of that blob.
    fn forget(&self, extension: &ExtensionId, flow_id: &DeviceLinkFlowId) {
        self.lock().flows.remove(flow_id);
        if let Some(provisional) = provisional_account_ref(flow_id) {
            self.sessions.discard_provisional(extension, &provisional);
        }
    }

    /// A poisoned map still holds the flows a running link depends on; keeping
    /// the driver serving beats propagating the panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, DriverState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Internal failure shape, split so the two public faces can answer honestly.
///
/// "No extension binds a device-link adapter" is a *different* answer from
/// "the adapter failed", and the auth port has a dedicated variant for it
/// (`DeviceLinkDriverError::NoBinding`, not restartable — installing an
/// extension is not "try again"). Collapsing both into the contracts-level
/// `DeviceLinkError` would lose that distinction, since that enum has no
/// not-bound variant.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DriverFailure {
    NoBinding,
    Link(DeviceLinkError),
}

impl From<DeviceLinkError> for DriverFailure {
    fn from(error: DeviceLinkError) -> Self {
        Self::Link(error)
    }
}

impl DriverFailure {
    /// Project onto the contracts vocabulary, for callers that are not the
    /// auth port. `Internal` is the only honest landing place for "not bound".
    fn into_link_error(self) -> DeviceLinkError {
        match self {
            Self::NoBinding => DeviceLinkError::Internal {
                reason: "no active extension binds a device-link adapter",
            },
            Self::Link(error) => error,
        }
    }
}

/// The bound adapter plus what the host needs to decide around it.
struct ResolvedDeviceLink {
    adapter: Arc<dyn DeviceLinkAdapter>,
    declaration: Arc<ironclaw_extension_registry::ResolvedExtensionManifest>,
    installation_id: String,
    config: Arc<BTreeMap<String, String>>,
    alternate_mode_declared: bool,
}

struct ExpiredFlow {
    extension_id: ExtensionId,
    scope: ironclaw_auth::AuthProductScope,
}

enum PollAdmission {
    Unknown,
    Expired,
    TooSoon { retry_in: Duration },
    Admitted,
}

enum InputAdmission {
    Expired,
    Admitted,
}

/// Per-user budgets are keyed by extension as well as user.
///
/// Pooling them across extensions would make one vendor's link attempts
/// consume another's budget — a user who exhausted their attempts linking one
/// account could not start linking a different service at all. The
/// deployment-wide counter is deliberately NOT keyed: it is the total-abuse
/// ceiling, and total means total.
type BudgetKey = (UserId, ExtensionId);

#[derive(Default)]
struct DriverState {
    flows: HashMap<DeviceLinkFlowId, FlowState>,
    begins: HashMap<BudgetKey, RateCounter>,
    deployment_begins: RateCounter,
    identifiers: HashMap<BudgetKey, IdentifierBudget>,
}

impl DriverState {
    /// Drop counters whose window has fully elapsed.
    ///
    /// Without this the two maps are append-only for the process lifetime:
    /// every user who ever started a link keeps a live entry, keyed by a value
    /// the reaper never revisits. Called from the same reap pass that expires
    /// flows.
    fn evict_spent_budgets(&mut self, now: Instant, window: Duration) {
        self.begins
            .retain(|_, counter| counter.is_live(now, window));
        self.identifiers
            .retain(|_, budget| budget.is_live(now, window));
    }
}

/// What a re-mint carries forward from the attempt it replaces.
#[derive(Debug, Clone, Copy)]
struct ResumedFlow {
    started_at: Instant,
    secret_attempts: u32,
}

struct FlowState {
    extension: ExtensionId,
    scope: ironclaw_auth::AuthProductScope,
    started_at: Instant,
    last_poll_at: Option<Instant>,
    secret_attempts: u32,
}

/// A fixed-window counter. Fixed rather than sliding on purpose: the budgets
/// here are small and human-paced, and a fixed window is the shape a reviewer
/// can check by reading it.
#[derive(Default)]
struct RateCounter {
    window_started_at: Option<Instant>,
    count: u32,
}

impl RateCounter {
    /// Whether this counter still constrains anything: a counter whose window
    /// has elapsed would reset on its next use, so holding it is pure memory.
    fn is_live(&self, now: Instant, window: Duration) -> bool {
        self.window_started_at
            .is_some_and(|started| now.saturating_duration_since(started) < window)
    }

    fn admit(&mut self, now: Instant, window: Duration, max: u32) -> bool {
        match self.window_started_at {
            Some(started) if now.saturating_duration_since(started) < window => {}
            _ => {
                self.window_started_at = Some(now);
                self.count = 0;
            }
        }
        if self.count >= max {
            return false;
        }
        self.count += 1;
        true
    }
}

/// Distinct-identifier budget, held as digests so no identifier is retained.
#[derive(Default)]
struct IdentifierBudget {
    window_started_at: Option<Instant>,
    digests: BTreeSet<[u8; 32]>,
}

impl IdentifierBudget {
    /// See [`RateCounter::is_live`].
    fn is_live(&self, now: Instant, window: Duration) -> bool {
        self.window_started_at
            .is_some_and(|started| now.saturating_duration_since(started) < window)
    }

    fn admit(&mut self, now: Instant, window: Duration, max: u32, digest: [u8; 32]) -> bool {
        match self.window_started_at {
            Some(started) if now.saturating_duration_since(started) < window => {}
            _ => {
                self.window_started_at = Some(now);
                self.digests.clear();
            }
        }
        if self.digests.contains(&digest) {
            // Re-submitting the same identifier (a typo'd code retried against
            // the same number) is not a new target.
            return true;
        }
        if self.digests.len() as u32 >= max {
            return false;
        }
        self.digests.insert(digest);
        true
    }
}

fn identifier_digest(value: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher.finalize().into()
}

/// The provisional custody ref a link-in-progress stores its blob under.
///
/// The completion mint reads the blob back through exactly this ref, so the
/// two sites must agree on the string — which is why both call this one
/// function.
fn provisional_account_ref(flow_id: &DeviceLinkFlowId) -> Option<LinkedAccountRef> {
    LinkedAccountRef::new(format!("{PENDING_ACCOUNT_PREFIX}{flow_id}")).ok()
}

/// Clamp an adapter-supplied account label into the credential-account label
/// grammar (≤ 256 bytes, no control characters). The step validator already
/// bounds it at 512 bytes; this walks char boundaries so a multi-byte
/// character is never split, and falls back to fixed host text if nothing
/// survives.
fn linked_account_label(
    value: &str,
) -> Result<ironclaw_auth::CredentialAccountLabel, DeviceLinkError> {
    const MAX_LABEL_BYTES: usize = 256;
    let mut clamped = String::new();
    for character in value.chars().filter(|c| !c.is_control()) {
        if clamped.len() + character.len_utf8() > MAX_LABEL_BYTES {
            break;
        }
        clamped.push(character);
    }
    let clamped = clamped.trim();
    ironclaw_auth::CredentialAccountLabel::new(clamped)
        .or_else(|_| ironclaw_auth::CredentialAccountLabel::new("Linked account"))
        .map_err(|_| DeviceLinkError::Internal {
            reason: "linked account label could not be formed",
        })
}

/// A host-side budget rejection, said in the host's own voice.
///
/// Deliberately NOT `RateLimited`: that code means the vendor pushed back, and
/// emitting it for a host budget told a user and the audit trail that a vendor
/// was called when none was. Restartable once the window rolls.
fn host_throttled() -> DeviceLinkError {
    DeviceLinkError::Vendor {
        code: DeviceLinkErrorCode::HostThrottled,
        restartable: true,
    }
}

/// A ceiling waiting will not clear — the host is already tracking as many
/// concurrent links as it will hold. Restartable only once one of them ends.
fn limit_reached() -> DeviceLinkError {
    DeviceLinkError::Vendor {
        code: DeviceLinkErrorCode::LimitReached,
        restartable: true,
    }
}

fn expired_step() -> DeviceLinkStep {
    DeviceLinkStep::Failed {
        code: DeviceLinkErrorCode::Expired,
        restartable: true,
    }
}

fn map_channel_identity_error(error: DeviceLinkChannelIdentityError) -> DriverFailure {
    match error {
        DeviceLinkChannelIdentityError::DifferentIdentityConnected
        | DeviceLinkChannelIdentityError::IdentityOwnedByAnotherUser => DeviceLinkError::Vendor {
            code: DeviceLinkErrorCode::IdentityConflict,
            restartable: false,
        }
        .into(),
        DeviceLinkChannelIdentityError::StorageUnavailable => DeviceLinkError::Custody(
            ironclaw_extension_contracts::linked_session::LinkedSessionError::Unavailable {
                reason: "device-link channel identity could not be stored",
            },
        )
        .into(),
        DeviceLinkChannelIdentityError::InvalidDeclaration => DeviceLinkError::Internal {
            reason: "device-link channel declaration is inconsistent",
        }
        .into(),
    }
}

mod auth_port;

#[cfg(test)]
mod tests;
