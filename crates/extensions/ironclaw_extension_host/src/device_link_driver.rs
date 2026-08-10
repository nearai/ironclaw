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
//! # TODO(design): completion cannot mint a credential account here
//!
//! `DeviceLinkStepOutcome` requires `account: Some(..)` on a `Completed` step,
//! and minting one needs an `AuthProductScope` plus `CredentialAccountService`.
//! **The port's request types carry no scope** — `DeviceLinkBinding` is
//! `(provider, extension_id, user_id)` — and synthesizing one from a bare user
//! id would be re-deriving security-relevant scope, which this repo bans
//! outright. So this implementation reports a completed step with
//! `account: None`, which the auth-side driver treats as a contract violation
//! and terminalizes. That is a **fail-closed hole, not a working path**: no
//! link can complete until either the port carries the flow's scope or the
//! minting seam lands host-side. Reconcile before PR 5's risk gate.

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

use crate::entrypoint::declared_device_link_recipe;
use crate::lifecycle::SnapshotWatch;
use crate::linked_session_custody::LinkedSessionStore;

/// Account-ref prefix for a link that has no credential account yet.
///
/// Custody is durable *before* the account is minted (PROPOSAL §4.3:
/// "store blob → mint account → report completed"), so the adapter needs a
/// scoped handle during the handshake. The host mints this provisional ref from
/// the flow id so the blob has a home.
///
/// TODO(design): the credential account minted at completion **must adopt this
/// exact ref**, or the blob written during the handshake is orphaned and the
/// first post-link call rehydrates nothing. That adoption is auth-side (PR 2)
/// and is not expressible here; the two halves have to agree on this string.
const PENDING_ACCOUNT_PREFIX: &str = "pending-link.";

/// The revision a provisional (pre-mint) grant carries. A real account starts
/// at 1 on its first link, so 0 can never collide with one.
const PENDING_LINK_REVISION: u64 = 0;

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
            max_active_flows: 64,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkRequest {
    pub flow_id: DeviceLinkFlowId,
    pub extension_id: ExtensionId,
    pub user_id: UserId,
    /// The established account, when one exists. `None` during a link that has
    /// not completed — there is no credential account until custody is durable.
    pub account: Option<LinkedAccountGrant>,
}

/// Resolves device-link adapters from the published snapshot and drives them
/// under host-owned limits.
pub struct SnapshotDeviceLinkDriver {
    snapshots: SnapshotWatch,
    sessions: Arc<LinkedSessionStore>,
    limits: DeviceLinkLimits,
    state: Mutex<DriverState>,
}

impl SnapshotDeviceLinkDriver {
    pub fn new(
        snapshots: SnapshotWatch,
        sessions: Arc<LinkedSessionStore>,
        limits: DeviceLinkLimits,
    ) -> Result<Self, DeviceLinkLimitsError> {
        limits.validate()?;
        Ok(Self {
            snapshots,
            sessions,
            limits,
            state: Mutex::new(DriverState::default()),
        })
    }

    pub fn limits(&self) -> DeviceLinkLimits {
        self.limits
    }

    /// Start a link on the chosen path.
    pub async fn begin(
        &self,
        request: &DeviceLinkRequest,
        mode: DeviceLinkMode,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.begin_at(request, mode, Instant::now())
            .await
            .map_err(DriverFailure::into_link_error)
    }

    /// Advance a link that is waiting on the vendor.
    pub async fn poll(
        &self,
        request: &DeviceLinkRequest,
    ) -> Result<DeviceLinkStep, DeviceLinkError> {
        self.poll_at(request, Instant::now())
            .await
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
        if request.account.is_none() {
            return Err(DeviceLinkError::Internal {
                reason: "device-link revoke requires an established account grant",
            });
        }
        let resolved = self
            .resolve(&request.extension_id)
            .map_err(DriverFailure::into_link_error)?;
        self.forget(&request.flow_id);
        let grant = self.custody_grant(request)?;
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, &resolved, session.as_ref());
        resolved.adapter.revoke(&context).await
    }

    async fn cancel_link(&self, request: &DeviceLinkRequest) -> Result<(), DriverFailure> {
        let resolved = self.resolve(&request.extension_id)?;
        self.forget(&request.flow_id);
        let grant = self.custody_grant(request)?;
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, &resolved, session.as_ref());
        Ok(resolved.adapter.cancel(&context).await?)
    }

    async fn begin_at(
        &self,
        request: &DeviceLinkRequest,
        mode: DeviceLinkMode,
        now: Instant,
    ) -> Result<DeviceLinkStep, DriverFailure> {
        let resolved = self.resolve(&request.extension_id)?;
        // The recipe, not the adapter, says which paths exist. Checking here
        // means an extension that declares no fallback cannot be talked into
        // running one.
        if mode == DeviceLinkMode::Alternate && !resolved.alternate_mode_declared {
            return Err(DeviceLinkError::UnsupportedMode { mode }.into());
        }

        self.reap_expired(now).await;
        self.admit_begin(request, now)?;

        let grant = self.custody_grant(request)?;
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, &resolved, session.as_ref());
        let outcome = resolved.adapter.begin(&context, mode).await;
        self.settle(request, outcome, now)
    }

    async fn poll_at(
        &self,
        request: &DeviceLinkRequest,
        now: Instant,
    ) -> Result<DeviceLinkStep, DriverFailure> {
        let resolved = self.resolve(&request.extension_id)?;
        match self.admit_poll(&request.flow_id, now) {
            PollAdmission::Unknown => Err(DeviceLinkError::UnknownFlow.into()),
            // A flow the host has already given up on is cancelled before it is
            // reported, so a vendor-side authorization cannot outlive it.
            PollAdmission::Expired => {
                self.cancel_flow(request, &resolved).await;
                Ok(expired_step())
            }
            // The adapter is not called at all: a hot-looping card must not be
            // able to turn into vendor traffic.
            PollAdmission::TooSoon { retry_in } => Ok(DeviceLinkStep::AwaitingVendor { retry_in }),
            PollAdmission::Admitted => {
                let grant = self.custody_grant(request)?;
                let session = self.sessions.open(&request.extension_id, &grant);
                let context = self.context(request, &resolved, session.as_ref());
                let outcome = resolved.adapter.poll(&context).await;
                self.settle(request, outcome, now)
            }
        }
    }

    async fn submit_input_at(
        &self,
        request: &DeviceLinkRequest,
        input: DeviceLinkInput,
        now: Instant,
    ) -> Result<DeviceLinkStep, DriverFailure> {
        // Bound the paste before anything vendor-shaped sees it.
        input.validate()?;
        let resolved = self.resolve(&request.extension_id)?;
        match self.admit_input(request, &input, now)? {
            InputAdmission::Expired => {
                self.cancel_flow(request, &resolved).await;
                return Ok(expired_step());
            }
            InputAdmission::Admitted => {}
        }

        let grant = self.custody_grant(request)?;
        let session = self.sessions.open(&request.extension_id, &grant);
        let context = self.context(request, &resolved, session.as_ref());
        let outcome = resolved.adapter.submit_input(&context, input).await;
        self.settle(request, outcome, now)
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
        Ok(ResolvedDeviceLink {
            adapter: binding.adapter,
            config: binding.config,
            alternate_mode_declared: recipe.alternate_mode_label.is_some(),
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
            user_id: &request.user_id,
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
        let provisional = format!("{PENDING_ACCOUNT_PREFIX}{}", request.flow_id);
        match LinkedAccountRef::new(provisional) {
            Ok(account) => Ok(LinkedAccountGrant::new(account, PENDING_LINK_REVISION)),
            Err(error) => {
                // `DeviceLinkFlowId` already rejects whitespace, control bytes,
                // and anything over 128 bytes, so this is unreachable in
                // practice; log the bound cause rather than dropping it, since
                // the typed error can only carry fixed text.
                debug!(%error, "device-link provisional account ref rejected");
                Err(DeviceLinkError::Internal {
                    reason: "device-link flow id does not form a valid account ref",
                })
            }
        }
    }

    /// Record an adapter outcome: bound its display text, clamp its clocks, and
    /// drop the flow once it stops advancing.
    fn settle(
        &self,
        request: &DeviceLinkRequest,
        outcome: Result<DeviceLinkStep, DeviceLinkError>,
        now: Instant,
    ) -> Result<DeviceLinkStep, DriverFailure> {
        let step = match outcome {
            Ok(step) => step,
            Err(error) => {
                // A failed transition ends the flow here, so a later call
                // cannot re-invoke a vendor step that already ran.
                self.forget(&request.flow_id);
                return Err(error.into());
            }
        };
        if let Err(error) = step.validate() {
            self.forget(&request.flow_id);
            return Err(error.into());
        }
        let remaining = self.remaining(&request.flow_id, now);
        let step = self.clamp(step, remaining);
        if step.is_terminal() {
            self.forget(&request.flow_id);
        }
        Ok(step)
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

    /// Cancel one flow's vendor-side state, best effort, and forget it.
    async fn cancel_flow(&self, request: &DeviceLinkRequest, resolved: &ResolvedDeviceLink) {
        self.forget(&request.flow_id);
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
                            user_id: flow.user.clone(),
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
                user_id: flow.user_id,
                account: None,
            };
            self.cancel_flow(&request, &resolved).await;
        }
    }

    fn admit_begin(
        &self,
        request: &DeviceLinkRequest,
        now: Instant,
    ) -> Result<(), DeviceLinkError> {
        let mut state = self.lock();
        if state.flows.contains_key(&request.flow_id) {
            // Never re-invoke a transition that already ran: a second `begin`
            // on a live flow would start a parallel vendor conversation.
            return Err(DeviceLinkError::Internal {
                reason: "device-link flow has already begun",
            });
        }
        if state.flows.len() >= self.limits.max_active_flows {
            return Err(rate_limited());
        }
        let window = self.limits.rate_window;
        if !state
            .deployment_begins
            .admit(now, window, self.limits.max_begins_per_deployment)
        {
            return Err(rate_limited());
        }
        let per_user = state.begins.entry(request.user_id.clone()).or_default();
        if !per_user.admit(now, window, self.limits.max_begins_per_user) {
            return Err(rate_limited());
        }
        state.flows.insert(
            request.flow_id.clone(),
            FlowState {
                extension: request.extension_id.clone(),
                user: request.user_id.clone(),
                started_at: now,
                last_poll_at: None,
                secret_attempts: 0,
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
                    .entry(request.user_id.clone())
                    .or_default();
                if !budget.admit(
                    now,
                    self.limits.rate_window,
                    self.limits.max_identifiers_per_user,
                    digest,
                ) {
                    return Err(rate_limited());
                }
            }
            DeviceLinkInput::Code(_) | DeviceLinkInput::Password(_) => {
                if flow.secret_attempts >= self.limits.max_secret_attempts_per_flow {
                    return Err(rate_limited());
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

    fn forget(&self, flow_id: &DeviceLinkFlowId) {
        self.lock().flows.remove(flow_id);
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
    config: Arc<BTreeMap<String, String>>,
    alternate_mode_declared: bool,
}

struct ExpiredFlow {
    extension_id: ExtensionId,
    user_id: UserId,
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

#[derive(Default)]
struct DriverState {
    flows: HashMap<DeviceLinkFlowId, FlowState>,
    begins: HashMap<UserId, RateCounter>,
    deployment_begins: RateCounter,
    identifiers: HashMap<UserId, IdentifierBudget>,
}

struct FlowState {
    extension: ExtensionId,
    user: UserId,
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

/// The host has no error variant of its own on the closed contract enum, so a
/// host-side budget rejection rides the code a card and the audit trail can
/// both read: `RateLimited`, restartable once the window rolls.
fn rate_limited() -> DeviceLinkError {
    DeviceLinkError::Vendor {
        code: DeviceLinkErrorCode::RateLimited,
        restartable: true,
    }
}

fn expired_step() -> DeviceLinkStep {
    DeviceLinkStep::Failed {
        code: DeviceLinkErrorCode::Expired,
        restartable: true,
    }
}

mod auth_port;

#[cfg(test)]
mod tests;
