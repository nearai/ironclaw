//! The durable device-link step driver.
//!
//! Modelled on the OAuth turn-gate driver next door — same setup lock, same
//! reusable-flow lookup, same "one driver for every vendor" shape — and
//! different from it in one structural way: an OAuth flow has a single
//! transition, and a device link has an unbounded sequence of them. That is
//! what the revision compare-and-swap and the **two clocks** are for.
//!
//! # Two clocks, and why they are not one
//!
//! - The **step clock** ([`DEVICE_LINK_STEP_TTL_SECONDS`], or the vendor's own
//!   shorter `expires_in`) governs the current frame. When it lapses the frame
//!   is **re-minted**: the user gets a fresh code and the link carries on.
//!   Collapsing it into the flow clock would mean a 30-second vendor token
//!   killed a link the user is still working through.
//! - The **flow clock** ([`AuthFlowRecord::expires_at`]) governs the attempt.
//!   When it lapses the flow **terminalizes** — and every re-mint may push it
//!   out only as far as [`DEVICE_LINK_FLOW_MAX_TTL_SECONDS`] from creation, so
//!   an unattended card cannot renew itself forever.
//!
//! Ordering is an invariant, not a coincidence:
//! `flow max ≥ flow ≥ step` (pinned by a test below). The vendor-side parked
//! link's own TTL sits above all three and belongs to the extension host.
//!
//! # What must never happen here
//!
//! A transition that already ran must never run twice. `check_password` and
//! its equivalents are not idempotent, so on a lost compare-and-swap this
//! driver **reloads and reports**, and does not retry the vendor call
//! (PROPOSAL §4.3).

use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_extension_contracts::device_link::{
    DeviceLinkErrorCode, DeviceLinkInput, DeviceLinkMode, DeviceLinkStep, DeviceLinkStepKind,
};
use ironclaw_extension_contracts::recipe::VendorAuthRecipe;
use ironclaw_host_api::ids::ExtensionId;
use static_assertions::const_assert;
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowKind, AuthFlowManager,
    AuthFlowRecord, AuthFlowStatus, AuthFlowStepAdvanceInput, AuthProductError, AuthProductScope,
    AuthProviderId, AuthRecipeResolver, DeviceLinkBeginRequest, DeviceLinkBinding,
    DeviceLinkCancelReason, DeviceLinkCancelRequest, DeviceLinkDriver, DeviceLinkDriverError,
    DeviceLinkPollRequest, DeviceLinkStepOutcome, DeviceLinkSubmitRequest, NewAuthFlow, Timestamp,
    is_terminal_status,
};

/// How long one displayed frame stays valid before it is re-minted. A vendor
/// that declares a shorter `expires_in` wins; a longer one is clamped, because
/// a stale frame the host cannot verify is a frame no user should still see.
pub const DEVICE_LINK_STEP_TTL_SECONDS: i64 = 60;

/// How long one link attempt stays alive without progress.
pub const DEVICE_LINK_FLOW_TTL_SECONDS: i64 = 600;

/// The hard ceiling on an attempt, measured from creation. Re-minting extends
/// the flow clock **up to this and no further** — the point of a capped
/// extension is that an abandoned card eventually stops renewing itself.
pub const DEVICE_LINK_FLOW_MAX_TTL_SECONDS: i64 = 1_800;

/// The default pace a card polls at, in milliseconds. A vendor asking for a
/// back-off overrides it for the next poll only.
pub const DEVICE_LINK_POLL_INTERVAL_MILLIS: u64 = 3_000;

/// Polls one flow may serve before it is abandoned. At the default pace this
/// is the flow-clock ceiling with headroom, so the bound that actually fires
/// first is the clock — this one exists to stop a caller polling in a tight
/// loop from doing so indefinitely.
pub const DEVICE_LINK_MAX_POLL_ATTEMPTS: u32 = 1_200;

// The clock ordering the design states as an invariant, enforced at compile
// time rather than by a test: three clocks that disagree is a bug class, and
// a build error is a cheaper place to find it than a lapsed link.
const_assert!(DEVICE_LINK_FLOW_MAX_TTL_SECONDS >= DEVICE_LINK_FLOW_TTL_SECONDS);
const_assert!(DEVICE_LINK_FLOW_TTL_SECONDS >= DEVICE_LINK_STEP_TTL_SECONDS);
const_assert!(DEVICE_LINK_STEP_TTL_SECONDS > 0);

/// Start (or resume) a device link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkStartRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    /// The installed extension whose manifest declares the device-link recipe.
    pub extension_id: ExtensionId,
    pub continuation: AuthContinuationRef,
    pub mode: DeviceLinkMode,
    /// The flow the caller already holds, if any.
    ///
    /// A card that re-renders (a refresh, a second tab, a re-opened settings
    /// pane) must not burn the live payload the user is mid-scan on, and
    /// creating a fresh setup-class flow would supersede it. Presenting the
    /// id is what makes "resume" expressible; a stale, mismatched, or lapsed
    /// id falls through to a fresh link rather than failing.
    pub resume: Option<AuthFlowId>,
}

/// Recipe-driven device-link flow driver. One driver for every vendor: the
/// vendor half arrives through [`DeviceLinkDriver`], and nothing in this type
/// branches on who the vendor is.
pub struct DeviceLinkFlowDriver {
    vendor: Arc<dyn DeviceLinkDriver>,
    flows: Arc<dyn AuthFlowManager>,
    recipes: Arc<dyn AuthRecipeResolver>,
    setup_lock: Arc<AsyncMutex<()>>,
}

impl std::fmt::Debug for DeviceLinkFlowDriver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceLinkFlowDriver")
            .finish_non_exhaustive()
    }
}

impl DeviceLinkFlowDriver {
    pub fn new(
        vendor: Arc<dyn DeviceLinkDriver>,
        flows: Arc<dyn AuthFlowManager>,
        recipes: Arc<dyn AuthRecipeResolver>,
    ) -> Self {
        Self {
            vendor,
            flows,
            recipes,
            setup_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    /// Start a link, or hand back the live one the caller is already on.
    ///
    /// Serialized by the setup lock for the same reason the OAuth gate is:
    /// two concurrent "link my account" clicks must not each mint a flow and
    /// each start a vendor conversation.
    pub async fn start(
        &self,
        request: DeviceLinkStartRequest,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let _setup_guard = self.setup_lock.lock().await;
        if let Some(existing) = self.reusable_flow(&request).await? {
            return self.resume_reusable(&request.scope, existing).await;
        }

        let copy = self.recipe_copy(&request).await?;
        let now = Utc::now();
        let flow_id = AuthFlowId::new();
        let record = self
            .flows
            .create_flow(NewAuthFlow {
                id: Some(flow_id),
                scope: request.scope.clone(),
                kind: AuthFlowKind::IntegrationCredential,
                provider: request.provider.clone(),
                requester_extension: Some(request.extension_id.clone()),
                requested_scopes: Vec::new(),
                // A placeholder frame, replaced by the vendor's first step
                // below. It exists so the record is never challenge-less: a
                // card that renders between create and begin shows "waiting",
                // not the unsupported-challenge fallback.
                challenge: AuthChallenge::DeviceLinkStep {
                    extension_id: request.extension_id.clone(),
                    display_name: copy.display_name,
                    default_mode_label: copy.default_mode_label,
                    alternate_mode_label: copy.alternate_mode_label,
                    mode: request.mode,
                    step: DeviceLinkStep::AwaitingVendor {
                        retry_in: std::time::Duration::from_millis(
                            DEVICE_LINK_POLL_INTERVAL_MILLIS,
                        ),
                    },
                    revision: 0,
                    expires_at: now + ChronoDuration::seconds(DEVICE_LINK_STEP_TTL_SECONDS),
                },
                continuation: request.continuation.clone(),
                update_binding: None,
                opaque_state_hash: None,
                pkce_verifier_hash: None,
                expires_at: now + ChronoDuration::seconds(DEVICE_LINK_FLOW_TTL_SECONDS),
            })
            .await?;

        self.mint_step(&request.scope, &record, request.mode, None)
            .await
    }

    /// Ask the vendor whether an in-flight link has moved.
    ///
    /// Order matters: the flow clock is checked before the step clock, because
    /// one terminalizes and the other only re-mints. Checking the step clock
    /// first would re-mint a frame onto a flow that has already run out.
    pub async fn poll(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let flow = self.live_flow(scope, flow_id).await?;
        if is_terminal_status(flow.status) {
            // Polling a finished link is a read, not an error: a card that
            // polls once more after completion should see the completion.
            return Ok(flow);
        }
        let frame = device_link_frame(&flow)?;
        let now = Utc::now();

        if flow.expires_at <= now {
            return self
                .abandon(
                    scope,
                    &flow,
                    DeviceLinkCancelReason::FlowExpired,
                    DeviceLinkErrorCode::Expired,
                    true,
                    AuthFlowStatus::Expired,
                    AuthErrorCode::UnknownOrExpiredFlow,
                )
                .await;
        }
        if flow.step_state.map_or(0, |state| state.poll_attempts) >= DEVICE_LINK_MAX_POLL_ATTEMPTS {
            return self
                .abandon(
                    scope,
                    &flow,
                    DeviceLinkCancelReason::FlowExpired,
                    DeviceLinkErrorCode::Expired,
                    true,
                    AuthFlowStatus::Expired,
                    AuthErrorCode::UnknownOrExpiredFlow,
                )
                .await;
        }
        if frame.expires_at <= now {
            // Step clock only. The attempt survives; the frame does not.
            return self.mint_step(scope, &flow, frame.mode, Some(now)).await;
        }

        let expected_revision = flow.step_revision();
        let binding = binding_for(&flow)?;
        let outcome = match self
            .vendor
            .poll(DeviceLinkPollRequest {
                flow_id: flow.id,
                binding,
            })
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                return self
                    .fail_from_driver_error(scope, &flow, expected_revision, error)
                    .await;
            }
        };
        self.apply(
            scope,
            &flow,
            expected_revision,
            frame.mode,
            outcome,
            Some(now),
        )
        .await
    }

    /// Hand the vendor one value the current frame asked for.
    ///
    /// `expected_revision` is the revision the card rendered from. A mismatch
    /// is refused outright rather than applied: a stale card submitting a
    /// secret against a frame that has already moved on is exactly the
    /// duplicated-transition hazard the compare-and-swap exists for, and here
    /// the value is a credential, so failing closed is the only safe answer.
    pub async fn submit_input(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
        expected_revision: u64,
        input: DeviceLinkInput,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        input.validate().map_err(|error| {
            AuthProductError::invalid_request(format!("device-link input rejected: {error}"))
        })?;
        let flow = self.live_flow(scope, flow_id).await?;
        if is_terminal_status(flow.status) {
            return Err(AuthProductError::FlowAlreadyTerminal);
        }
        let frame = device_link_frame(&flow)?;
        if flow.expires_at <= Utc::now() {
            return self
                .abandon(
                    scope,
                    &flow,
                    DeviceLinkCancelReason::FlowExpired,
                    DeviceLinkErrorCode::Expired,
                    true,
                    AuthFlowStatus::Expired,
                    AuthErrorCode::UnknownOrExpiredFlow,
                )
                .await;
        }
        if flow.step_revision() != expected_revision {
            return Err(AuthProductError::BackendConflict);
        }
        if frame.kind != DeviceLinkStepKind::InputRequired {
            return Err(AuthProductError::invalid_request(
                "device-link flow is not waiting for user input",
            ));
        }

        let binding = binding_for(&flow)?;
        let outcome = match self
            .vendor
            .submit(DeviceLinkSubmitRequest {
                flow_id: flow.id,
                binding,
                input,
            })
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                return self
                    .fail_from_driver_error(scope, &flow, expected_revision, error)
                    .await;
            }
        };
        self.apply(scope, &flow, expected_revision, frame.mode, outcome, None)
            .await
    }

    /// Abandon a link on the user's behalf.
    ///
    /// The vendor cancel is awaited, never left to `Drop`: once the vendor has
    /// authorized the device, walking away silently leaves a live
    /// authorization nobody knows about, and a synchronous drop cannot make an
    /// asynchronous logout call.
    pub async fn cancel(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let flow = self.live_flow(scope, flow_id).await?;
        if is_terminal_status(flow.status) {
            return Err(AuthProductError::FlowAlreadyTerminal);
        }
        self.abandon(
            scope,
            &flow,
            DeviceLinkCancelReason::UserCanceled,
            DeviceLinkErrorCode::Declined,
            true,
            AuthFlowStatus::Canceled,
            AuthErrorCode::Canceled,
        )
        .await
    }

    async fn live_flow(
        &self,
        scope: &AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        self.flows
            .get_flow(scope, flow_id)
            .await?
            .ok_or(AuthProductError::UnknownOrExpiredFlow)
    }

    /// The live flow a `resume` hint points at, when it is genuinely reusable.
    ///
    /// Fails soft in every direction: a missing, mismatched, terminal, or
    /// lapsed flow yields `None` and the caller mints a fresh one. Only a
    /// cross-scope read is an error, and that comes from `get_flow`.
    async fn reusable_flow(
        &self,
        request: &DeviceLinkStartRequest,
    ) -> Result<Option<AuthFlowRecord>, AuthProductError> {
        let Some(flow_id) = request.resume else {
            return Ok(None);
        };
        let Some(flow) = self.flows.get_flow(&request.scope, flow_id).await? else {
            return Ok(None);
        };
        if flow.provider != request.provider
            || flow.requester_extension.as_ref() != Some(&request.extension_id)
            || is_terminal_status(flow.status)
            || flow.expires_at <= Utc::now()
            || flow.device_link_step().is_none()
        {
            return Ok(None);
        }
        Ok(Some(flow))
    }

    /// Hand back a reusable flow, re-minting its frame only if the step clock
    /// has lapsed. A live frame is returned untouched — re-minting on every
    /// re-render would invalidate the code the user is mid-scan on.
    async fn resume_reusable(
        &self,
        scope: &AuthProductScope,
        flow: AuthFlowRecord,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let frame = device_link_frame(&flow)?;
        if frame.expires_at > Utc::now() {
            return Ok(flow);
        }
        self.mint_step(scope, &flow, frame.mode, None).await
    }

    /// Resolve the recipe copy a card renders: the account name and the two
    /// path labels.
    ///
    /// All three are read once, when the flow starts, and held on the durable
    /// challenge — a card rendered later must not have to re-resolve a
    /// manifest that may no longer be installed.
    async fn recipe_copy(
        &self,
        request: &DeviceLinkStartRequest,
    ) -> Result<RecipeCopy, AuthProductError> {
        let resolved = self
            .recipes
            .resolve(
                Some(&request.extension_id),
                Some(&request.scope.resource.user_id),
                request.provider.as_str(),
            )
            .await
            .ok_or(AuthProductError::MalformedConfig)?;
        match resolved.recipe {
            VendorAuthRecipe::DeviceLink(recipe) => Ok(RecipeCopy {
                display_name: recipe.display_name,
                default_mode_label: recipe.default_mode_label,
                alternate_mode_label: recipe.alternate_mode_label,
            }),
            // Refusing here is the point: starting a device link against a
            // vendor whose manifest declares some other method would drive an
            // adapter that is not bound for it.
            _ => Err(AuthProductError::MalformedConfig),
        }
    }

    /// Ask the vendor for a fresh first frame — used both to start a link and
    /// to re-mint one whose step clock lapsed.
    async fn mint_step(
        &self,
        scope: &AuthProductScope,
        flow: &AuthFlowRecord,
        mode: DeviceLinkMode,
        polled_at: Option<Timestamp>,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let expected_revision = flow.step_revision();
        let binding = binding_for(flow)?;
        let outcome = match self
            .vendor
            .begin(DeviceLinkBeginRequest {
                flow_id: flow.id,
                binding,
                mode,
            })
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                return self
                    .fail_from_driver_error(scope, flow, expected_revision, error)
                    .await;
            }
        };
        self.apply(scope, flow, expected_revision, mode, outcome, polled_at)
            .await
    }

    /// Persist one vendor transition under compare-and-swap.
    ///
    /// The returned record is the winner's either way. A caller that lost the
    /// CAS has already made its vendor call and must not make another — which
    /// is structural here, because this is the last thing every path does.
    async fn apply(
        &self,
        scope: &AuthProductScope,
        flow: &AuthFlowRecord,
        expected_revision: u64,
        mode: DeviceLinkMode,
        outcome: DeviceLinkStepOutcome,
        polled_at: Option<Timestamp>,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        if let Err(error) = outcome.step.validate() {
            tracing::debug!(
                flow_id = %flow.id,
                %error,
                "device-link adapter returned a step the host will not accept"
            );
            return self
                .fail(
                    scope,
                    flow,
                    expected_revision,
                    mode,
                    DeviceLinkErrorCode::Internal,
                    false,
                )
                .await;
        }
        // A completion the driver cannot back with a minted credential account
        // is not a completion. Custody is durable and the account exists
        // before the driver reports `Completed`; reporting it anyway would
        // announce a link nothing can serve.
        if matches!(outcome.step, DeviceLinkStep::Completed { .. }) && outcome.account.is_none() {
            tracing::debug!(
                flow_id = %flow.id,
                "device-link driver reported completion with no credential account"
            );
            return self
                .fail(
                    scope,
                    flow,
                    expected_revision,
                    mode,
                    DeviceLinkErrorCode::Internal,
                    false,
                )
                .await;
        }

        let now = Utc::now();
        let status = status_for(&outcome.step);
        let terminal = is_terminal_status(status);
        let input = AuthFlowStepAdvanceInput {
            flow_id: flow.id,
            expected_revision,
            challenge: challenge_for(flow, mode, &outcome.step, expected_revision + 1, now)?,
            status,
            step_kind: outcome.step.kind(),
            step_expires_at: step_deadline(&outcome.step, now),
            flow_expires_at: if terminal {
                None
            } else {
                extended_flow_deadline(flow, now)
            },
            polled_at,
            error: terminal_error(&outcome.step),
            credential_account_id: outcome.account.as_ref().map(|account| account.account_id),
        };
        self.flows
            .advance_flow_step(scope, input)
            .await
            .map(|advance| advance.record)
    }

    /// Terminalize a flow with a host-authored failure frame.
    async fn fail(
        &self,
        scope: &AuthProductScope,
        flow: &AuthFlowRecord,
        expected_revision: u64,
        mode: DeviceLinkMode,
        code: DeviceLinkErrorCode,
        restartable: bool,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let now = Utc::now();
        let step = DeviceLinkStep::Failed { code, restartable };
        let input = AuthFlowStepAdvanceInput {
            flow_id: flow.id,
            expected_revision,
            challenge: challenge_for(flow, mode, &step, expected_revision + 1, now)?,
            status: AuthFlowStatus::Failed,
            step_kind: DeviceLinkStepKind::Failed,
            step_expires_at: now,
            flow_expires_at: None,
            polled_at: None,
            error: Some(auth_error_for(code)),
            credential_account_id: None,
        };
        self.flows
            .advance_flow_step(scope, input)
            .await
            .map(|advance| advance.record)
    }

    async fn fail_from_driver_error(
        &self,
        scope: &AuthProductScope,
        flow: &AuthFlowRecord,
        expected_revision: u64,
        error: DeviceLinkDriverError,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let mode = device_link_frame(flow)
            .map(|frame| frame.mode)
            .unwrap_or(DeviceLinkMode::Default);
        tracing::debug!(
            flow_id = %flow.id,
            code = ?error.code(),
            "device-link driver call failed"
        );
        self.fail(
            scope,
            flow,
            expected_revision,
            mode,
            error.code(),
            error.restartable(),
        )
        .await
    }

    /// Tear a link down vendor-side, then terminalize the record.
    ///
    /// The vendor call is best-effort and its failure never blocks the local
    /// terminalization — but it is reported honestly rather than swallowed:
    /// an abort whose logout failed may have left a live authorization.
    // arch-exempt: too_many_args, one terminalization shape parameterized by
    // the four axes a caller genuinely varies (vendor reason, vendor code,
    // restartability, and the durable status/error pair); a context struct
    // here would be a single-use bag, plan: the device-link PROPOSAL §4.5
    #[allow(clippy::too_many_arguments)]
    async fn abandon(
        &self,
        scope: &AuthProductScope,
        flow: &AuthFlowRecord,
        reason: DeviceLinkCancelReason,
        code: DeviceLinkErrorCode,
        restartable: bool,
        status: AuthFlowStatus,
        error: AuthErrorCode,
    ) -> Result<AuthFlowRecord, AuthProductError> {
        let expected_revision = flow.step_revision();
        let mode = device_link_frame(flow)
            .map(|frame| frame.mode)
            .unwrap_or(DeviceLinkMode::Default);
        if let Ok(binding) = binding_for(flow)
            && let Err(cancel_error) = self
                .vendor
                .cancel(DeviceLinkCancelRequest {
                    flow_id: flow.id,
                    binding,
                    reason,
                })
                .await
        {
            tracing::debug!(
                flow_id = %flow.id,
                code = ?cancel_error.code(),
                "device-link vendor teardown failed; the vendor may still hold an authorization"
            );
        }

        let now = Utc::now();
        let step = DeviceLinkStep::Failed { code, restartable };
        let input = AuthFlowStepAdvanceInput {
            flow_id: flow.id,
            expected_revision,
            challenge: challenge_for(flow, mode, &step, expected_revision + 1, now)?,
            status,
            step_kind: DeviceLinkStepKind::Failed,
            step_expires_at: now,
            flow_expires_at: None,
            polled_at: None,
            error: Some(error),
            credential_account_id: None,
        };
        self.flows
            .advance_flow_step(scope, input)
            .await
            .map(|advance| advance.record)
    }
}

/// The parts of a device-link challenge the driver reasons about.
struct DeviceLinkFrame {
    mode: DeviceLinkMode,
    kind: DeviceLinkStepKind,
    expires_at: Timestamp,
}

fn device_link_frame(flow: &AuthFlowRecord) -> Result<DeviceLinkFrame, AuthProductError> {
    match flow.challenge.as_ref() {
        Some(AuthChallenge::DeviceLinkStep {
            mode,
            step,
            expires_at,
            ..
        }) => Ok(DeviceLinkFrame {
            mode: *mode,
            kind: step.kind(),
            expires_at: *expires_at,
        }),
        // Not a device-link flow at all: the caller routed a flow of another
        // method here. Reported as unknown rather than mis-served.
        _ => Err(AuthProductError::UnknownOrExpiredFlow),
    }
}

fn binding_for(flow: &AuthFlowRecord) -> Result<DeviceLinkBinding, AuthProductError> {
    let extension_id = flow
        .requester_extension
        .clone()
        // A device link is always requested by an installed extension; a flow
        // without one cannot be routed to an adapter, and guessing would mean
        // driving somebody else's.
        .ok_or(AuthProductError::MalformedConfig)?;
    Ok(DeviceLinkBinding {
        provider: flow.provider.clone(),
        extension_id,
        // The durable flow record's own scope, never a value a card or an
        // adapter supplied. Completion mints the credential account under it.
        scope: flow.scope.clone(),
    })
}

/// The recipe-authored copy a device-link card renders.
struct RecipeCopy {
    display_name: String,
    default_mode_label: Option<String>,
    alternate_mode_label: Option<String>,
}

fn challenge_for(
    flow: &AuthFlowRecord,
    mode: DeviceLinkMode,
    step: &DeviceLinkStep,
    revision: u64,
    now: Timestamp,
) -> Result<AuthChallenge, AuthProductError> {
    let (extension_id, display_name, default_mode_label, alternate_mode_label) =
        match flow.challenge.as_ref() {
            Some(AuthChallenge::DeviceLinkStep {
                extension_id,
                display_name,
                default_mode_label,
                alternate_mode_label,
                ..
            }) => (
                extension_id.clone(),
                display_name.clone(),
                default_mode_label.clone(),
                alternate_mode_label.clone(),
            ),
            _ => return Err(AuthProductError::UnknownOrExpiredFlow),
        };
    Ok(AuthChallenge::DeviceLinkStep {
        extension_id,
        display_name,
        default_mode_label,
        alternate_mode_label,
        mode,
        step: step.clone(),
        revision,
        expires_at: step_deadline(step, now),
    })
}

fn status_for(step: &DeviceLinkStep) -> AuthFlowStatus {
    match step {
        // A displayed payload IS a wait on the vendor: nothing the user types
        // advances it, only the vendor confirming the device does.
        DeviceLinkStep::Display { .. } | DeviceLinkStep::AwaitingVendor { .. } => {
            AuthFlowStatus::AwaitingVendor
        }
        DeviceLinkStep::InputRequired { .. } => AuthFlowStatus::AwaitingUser,
        DeviceLinkStep::Completed { .. } => AuthFlowStatus::Completed,
        DeviceLinkStep::Failed { .. } => AuthFlowStatus::Failed,
    }
}

fn terminal_error(step: &DeviceLinkStep) -> Option<AuthErrorCode> {
    match step {
        DeviceLinkStep::Failed { code, .. } => Some(auth_error_for(*code)),
        _ => None,
    }
}

/// The step clock for a frame: the vendor's own window when it declares a
/// shorter one, the host ceiling otherwise.
fn step_deadline(step: &DeviceLinkStep, now: Timestamp) -> Timestamp {
    let ceiling = now + ChronoDuration::seconds(DEVICE_LINK_STEP_TTL_SECONDS);
    match step {
        DeviceLinkStep::Display { expires_in, .. } => {
            match ChronoDuration::from_std(*expires_in) {
                Ok(window) => (now + window).min(ceiling),
                // An `expires_in` too large for the calendar type is not a
                // longer window, it is a broken one: fall back to the ceiling.
                Err(_) => ceiling,
            }
        }
        _ => ceiling,
    }
}

/// The capped extension: push the flow clock out by one full TTL, but never
/// past the creation-anchored ceiling, and never backwards.
fn extended_flow_deadline(flow: &AuthFlowRecord, now: Timestamp) -> Option<Timestamp> {
    let ceiling = flow.created_at + ChronoDuration::seconds(DEVICE_LINK_FLOW_MAX_TTL_SECONDS);
    let proposed = (now + ChronoDuration::seconds(DEVICE_LINK_FLOW_TTL_SECONDS)).min(ceiling);
    (proposed > flow.expires_at).then_some(proposed)
}

/// Map the closed vendor-failure vocabulary onto the crate's own.
fn auth_error_for(code: DeviceLinkErrorCode) -> AuthErrorCode {
    match code {
        DeviceLinkErrorCode::Expired | DeviceLinkErrorCode::UnknownFlow => {
            AuthErrorCode::UnknownOrExpiredFlow
        }
        DeviceLinkErrorCode::Declined => AuthErrorCode::ProviderDenied,
        DeviceLinkErrorCode::InvalidInput => AuthErrorCode::InvalidRequest,
        // The account can never link — a re-prompt loop would be the wrong
        // behavior, so this reads as "no credential", not "try again".
        DeviceLinkErrorCode::AccountUnavailable | DeviceLinkErrorCode::IdentityConflict => {
            AuthErrorCode::CredentialMissing
        }
        DeviceLinkErrorCode::RateLimited
        | DeviceLinkErrorCode::HostThrottled
        | DeviceLinkErrorCode::LimitReached
        | DeviceLinkErrorCode::VendorUnavailable
        | DeviceLinkErrorCode::CustodyFailed
        | DeviceLinkErrorCode::Internal => AuthErrorCode::BackendUnavailable,
    }
}

#[cfg(test)]
mod tests;
