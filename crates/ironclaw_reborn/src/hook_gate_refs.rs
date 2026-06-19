//! Router-backed hook gate refs for Reborn host composition.
//!
//! `ironclaw_hooks` owns the hook middleware contract, but it deliberately
//! does not know how to reserve host approval/auth gates. This module is the
//! Reborn-side adapter: it binds hook-emitted pause decisions to the current
//! run, actor, capability, arguments digest, and router-enforced lease window
//! before returning a `LoopGateRef` to the middleware.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ironclaw_hooks::middleware::HookGateRefFactory;
use ironclaw_host_api::{ApprovalRequestId, CapabilityId, UserId, sha256_digest_token};
use ironclaw_turns::{
    LoopGateRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
        LoopRunContext, VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};

tokio::task_local! {
    static HOOK_GATE_INVOCATION: HookGateInvocationMetadata;
}

/// The gateway lane a hook pause decision reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookGateKind {
    Approval,
    Auth,
}

impl HookGateKind {
    fn gate_ref_prefix(self) -> &'static str {
        match self {
            Self::Approval => "hook-approval",
            Self::Auth => "hook-auth",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::Auth => "auth",
        }
    }
}

/// Actor/session identity bound into a hook gate reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookGateActorBinding {
    pub actor_id: UserId,
    pub session_id: Option<String>,
}

impl HookGateActorBinding {
    pub fn new(actor_id: UserId) -> Self {
        Self {
            actor_id,
            session_id: None,
        }
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Result<Self, HookGateError> {
        let session_id = validate_token(session_id.into(), "session id", 128)?;
        self.session_id = Some(session_id);
        Ok(self)
    }
}

/// Run and actor identity resolved when minting a hook gate reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookGateReservationContext {
    pub run_context: LoopRunContext,
    pub actor: HookGateActorBinding,
}

impl HookGateReservationContext {
    pub fn new(run_context: LoopRunContext, actor: HookGateActorBinding) -> Self {
        Self { run_context, actor }
    }
}

type HookGateReservationContextSource =
    Arc<dyn Fn() -> HookGateReservationContext + Send + Sync + 'static>;

/// Upper bound on a configured reservation TTL. Reservations that never
/// resolve accumulate in router state for the full TTL; an operator
/// misconfiguring a year-long TTL would create a long-tail memory leak
/// in `InMemoryHookGateRouter` and an unbounded grant window in durable
/// backends. 24 hours is plenty for human-in-the-loop approval flows.
/// henrypark133 must-fix #1 on PR #3633.
const MAX_RESERVATION_TTL_HOURS: i64 = 24;

/// Upper bound on the free-form reason text accompanying a gate-ref
/// reservation. Without a cap, a buggy or malicious caller could push
/// arbitrarily large strings through the approval store. 4 KiB is
/// generous for human-facing approval reasons. henrypark133 must-fix #2
/// on PR #3633.
const MAX_REASON_BYTES: usize = 4096;

/// Per-invocation metadata captured outside `ironclaw_hooks` and consumed by
/// [`RouterBackedHookGateRefFactory`] when the hook middleware asks for a ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookGateInvocationMetadata {
    pub capability_id: CapabilityId,
    pub arguments_digest: String,
}

impl HookGateInvocationMetadata {
    pub fn for_invocation(invocation: &CapabilityInvocation) -> Result<Self, HookGateError> {
        let arguments_digest = hook_gate_arguments_digest(invocation);
        validate_digest(&arguments_digest)?;
        Ok(Self {
            capability_id: invocation.capability_id.clone(),
            arguments_digest,
        })
    }
}

/// Stable digest token for the capability invocation arguments visible to the
/// hook gate path. The digest includes the cited capability id and opaque input
/// ref so a later resolution can verify it is consuming the same gated call
/// without exposing raw arguments to the approval surface.
pub fn hook_gate_arguments_digest(invocation: &CapabilityInvocation) -> String {
    let payload = format!(
        "hook-gate-arguments-v1\nsurface={}\ncapability={}\ninput={}",
        invocation.surface_version.as_str(),
        invocation.capability_id.as_str(),
        invocation.input_ref.as_str()
    );
    sha256_digest_token(payload.as_bytes())
}

/// Request handed to the host approval/auth router when a hook pauses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookGateReservationRequest {
    pub kind: HookGateKind,
    pub run_context: LoopRunContext,
    pub actor: HookGateActorBinding,
    pub capability_id: CapabilityId,
    pub arguments_digest: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Router-owned reservation returned for an issued hook gate ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookGateReservation {
    pub gate_ref: LoopGateRef,
    pub kind: HookGateKind,
    pub run_context: LoopRunContext,
    pub actor: HookGateActorBinding,
    pub capability_id: CapabilityId,
    pub arguments_digest: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Request to consume a previously issued hook gate ref.
///
/// **No timestamp field.** Resolution time and consumption time are
/// authoritatively owned by the router via its own clock — see
/// [`HookGateResolution::resolved_at`] for the router-supplied value
/// returned on successful consumption. An earlier revision of this struct
/// exposed a caller-supplied `resolved_at`; that was a trust-boundary bug
/// (serrrfirat HIGH on PR #3633) because any adapter or buggy caller could
/// backdate it and bypass TTL enforcement. The field is gone; expiry and
/// `consumed_at` are now computed inside `resolve_gate` from the router's
/// own wall clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookGateResolutionRequest {
    pub gate_ref: LoopGateRef,
    pub expected_kind: Option<HookGateKind>,
    pub actor: HookGateActorBinding,
    pub run_context: LoopRunContext,
    pub capability_id: CapabilityId,
    pub arguments_digest: String,
}

impl HookGateResolutionRequest {
    pub fn for_invocation(
        gate_ref: LoopGateRef,
        actor: HookGateActorBinding,
        run_context: LoopRunContext,
        invocation: &CapabilityInvocation,
    ) -> Result<Self, HookGateError> {
        Self::for_kind(
            gate_ref,
            HookGateKind::Approval,
            actor,
            run_context,
            invocation,
        )
    }

    pub fn for_kind(
        gate_ref: LoopGateRef,
        expected_kind: HookGateKind,
        actor: HookGateActorBinding,
        run_context: LoopRunContext,
        invocation: &CapabilityInvocation,
    ) -> Result<Self, HookGateError> {
        let metadata = HookGateInvocationMetadata::for_invocation(invocation)?;
        Ok(Self {
            gate_ref,
            expected_kind: Some(expected_kind),
            actor,
            run_context,
            capability_id: metadata.capability_id,
            arguments_digest: metadata.arguments_digest,
        })
    }
}

/// Successful one-shot consumption of a hook gate ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookGateResolution {
    pub gate_ref: LoopGateRef,
    pub kind: HookGateKind,
    pub run_context: LoopRunContext,
    pub actor: HookGateActorBinding,
    pub capability_id: CapabilityId,
    pub arguments_digest: String,
    pub resolved_at: DateTime<Utc>,
}

/// Minimal host-router seam used by the Reborn hook gate-ref factory.
///
/// Existing approval resolution in `ironclaw_approvals::ApprovalResolver`
/// resolves already persisted approval requests into capability leases; it
/// does not expose a hook-facing reservation API with actor/session binding,
/// argument digest matching, TTL, and one-shot consumption. Production hosts
/// should implement this trait over their approval/auth router and durable
/// stores, while tests may use [`InMemoryHookGateRouter`].
#[async_trait]
pub trait HookGateRouter: Send + Sync {
    async fn reserve_gate(
        &self,
        request: HookGateReservationRequest,
    ) -> Result<LoopGateRef, HookGateError>;

    async fn resolve_gate(
        &self,
        request: HookGateResolutionRequest,
    ) -> Result<HookGateResolution, HookGateError>;
}

/// Production `HookGateRefFactory` adapter backed by a host approval/auth
/// router.
pub struct RouterBackedHookGateRefFactory {
    context_source: HookGateReservationContextSource,
    router: Arc<dyn HookGateRouter>,
    reservation_ttl: Duration,
}

impl fmt::Debug for RouterBackedHookGateRefFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RouterBackedHookGateRefFactory")
            .field("reservation_ttl", &self.reservation_ttl)
            .finish_non_exhaustive()
    }
}

impl RouterBackedHookGateRefFactory {
    pub fn try_new<F>(
        router: Arc<dyn HookGateRouter>,
        reservation_ttl: Duration,
        context_source: F,
    ) -> Result<Self, HookGateError>
    where
        F: Fn() -> HookGateReservationContext + Send + Sync + 'static,
    {
        if reservation_ttl <= Duration::zero() {
            return Err(HookGateError::InvalidTtl);
        }
        // henrypark133 must-fix #1: cap to keep unresolved reservations
        // from accumulating in router state for impractical durations.
        let max_ttl = Duration::hours(MAX_RESERVATION_TTL_HOURS);
        if reservation_ttl > max_ttl {
            return Err(HookGateError::InvalidTtl);
        }
        Ok(Self {
            context_source: Arc::new(context_source),
            router,
            reservation_ttl,
        })
    }

    async fn mint(
        &self,
        kind: HookGateKind,
        reason: &str,
    ) -> Result<LoopGateRef, AgentLoopHostError> {
        // henrypark133 must-fix #2: cap reason text before it crosses
        // into router state. The reason is operator-facing and may be
        // persisted; an unbounded string is a memory + display surface
        // problem.
        if reason.len() > MAX_REASON_BYTES {
            return Err(AgentLoopHostError::from(HookGateError::InvalidToken {
                field: "reason",
                reason: format!(
                    "must be at most {MAX_REASON_BYTES} bytes; got {}",
                    reason.len()
                ),
            }));
        }
        let metadata =
            current_hook_gate_invocation().ok_or(HookGateError::MissingInvocationMetadata)?;
        let now = Utc::now();
        let expires_at = now
            .checked_add_signed(self.reservation_ttl)
            .ok_or(HookGateError::InvalidTtl)?;
        let context = (self.context_source)();
        let request = HookGateReservationRequest {
            kind,
            run_context: context.run_context,
            actor: context.actor,
            capability_id: metadata.capability_id,
            arguments_digest: metadata.arguments_digest,
            reason: reason.to_string(),
            created_at: now,
            expires_at,
        };
        self.router
            .reserve_gate(request)
            .await
            .map_err(AgentLoopHostError::from)
    }
}

#[async_trait]
impl HookGateRefFactory for RouterBackedHookGateRefFactory {
    async fn mint_approval_ref(&self, reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        self.mint(HookGateKind::Approval, reason).await
    }

    async fn mint_auth_ref(&self, reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        self.mint(HookGateKind::Auth, reason).await
    }
}

/// Outer capability-port adapter that scopes the current invocation metadata
/// for router-backed hook gate factories without changing `ironclaw_hooks`.
pub struct HookGateInvocationScopePort {
    inner: Arc<dyn LoopCapabilityPort>,
}

impl HookGateInvocationScopePort {
    pub fn new(inner: Arc<dyn LoopCapabilityPort>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl LoopCapabilityPort for HookGateInvocationScopePort {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.inner.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let metadata = HookGateInvocationMetadata::for_invocation(&request)
            .map_err(AgentLoopHostError::from)?;
        HOOK_GATE_INVOCATION
            .scope(metadata, self.inner.invoke_capability(request))
            .await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let CapabilityBatchInvocation {
            invocations,
            stop_on_first_suspension,
        } = request;
        let mut outcomes = Vec::with_capacity(invocations.len());
        let mut stopped_on_suspension = false;
        for invocation in invocations {
            if stopped_on_suspension {
                break;
            }
            let outcome = self.invoke_capability(invocation).await?;
            if outcome.is_suspension() && stop_on_first_suspension {
                stopped_on_suspension = true;
            }
            outcomes.push(outcome);
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

fn current_hook_gate_invocation() -> Option<HookGateInvocationMetadata> {
    HOOK_GATE_INVOCATION.try_with(Clone::clone).ok()
}

/// In-memory router implementation for caller-level tests and local harnesses.
#[derive(Debug, Default)]
pub struct InMemoryHookGateRouter {
    reservations: Mutex<HashMap<String, InMemoryReservation>>,
}

impl InMemoryHookGateRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn reserve(
        &self,
        request: HookGateReservationRequest,
    ) -> Result<LoopGateRef, HookGateError> {
        self.reserve_gate(request).await
    }

    pub async fn resolve(
        &self,
        request: HookGateResolutionRequest,
    ) -> Result<HookGateResolution, HookGateError> {
        self.resolve_gate(request).await
    }

    fn reservations_guard(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, InMemoryReservation>>, HookGateError> {
        self.reservations
            .lock()
            .map_err(|_| HookGateError::Backend("hook gate router mutex poisoned".to_string()))
    }
}

#[async_trait]
impl HookGateRouter for InMemoryHookGateRouter {
    async fn reserve_gate(
        &self,
        request: HookGateReservationRequest,
    ) -> Result<LoopGateRef, HookGateError> {
        validate_digest(&request.arguments_digest)?;
        if request.expires_at <= request.created_at {
            return Err(HookGateError::InvalidTtl);
        }
        let gate_ref = LoopGateRef::new(format!(
            "gate:{}-{}",
            request.kind.gate_ref_prefix(),
            ApprovalRequestId::new()
        ))
        .map_err(|error| HookGateError::InvalidGateRef(error.to_string()))?;
        let reservation = HookGateReservation {
            gate_ref: gate_ref.clone(),
            kind: request.kind,
            run_context: request.run_context,
            actor: request.actor,
            capability_id: request.capability_id,
            arguments_digest: request.arguments_digest,
            reason: request.reason,
            created_at: request.created_at,
            expires_at: request.expires_at,
        };
        let mut reservations = self.reservations_guard()?;
        reservations.insert(
            gate_ref.as_str().to_string(),
            InMemoryReservation {
                reservation,
                consumed_at: None,
            },
        );
        Ok(gate_ref)
    }

    async fn resolve_gate(
        &self,
        request: HookGateResolutionRequest,
    ) -> Result<HookGateResolution, HookGateError> {
        let resolved_at = Utc::now();
        let mut reservations = self.reservations_guard()?;
        let state = reservations
            .get_mut(request.gate_ref.as_str())
            .ok_or_else(|| HookGateError::UnknownGate {
                gate_ref: request.gate_ref.clone(),
            })?;
        if let Some(consumed_at) = state.consumed_at {
            return Err(HookGateError::AlreadyConsumed {
                gate_ref: state.reservation.gate_ref.clone(),
                consumed_at,
            });
        }
        if resolved_at >= state.reservation.expires_at {
            return Err(HookGateError::Expired {
                gate_ref: state.reservation.gate_ref.clone(),
                expires_at: state.reservation.expires_at,
            });
        }
        if let Some(expected_kind) = request.expected_kind
            && expected_kind != state.reservation.kind
        {
            return Err(HookGateError::KindMismatch {
                gate_ref: state.reservation.gate_ref.clone(),
                expected: expected_kind,
                actual: state.reservation.kind,
            });
        }
        if !same_run_context(&request.run_context, &state.reservation.run_context) {
            return Err(HookGateError::RunMismatch {
                gate_ref: state.reservation.gate_ref.clone(),
            });
        }
        if request.actor != state.reservation.actor {
            return Err(HookGateError::ActorMismatch {
                gate_ref: state.reservation.gate_ref.clone(),
            });
        }
        if request.capability_id != state.reservation.capability_id {
            return Err(HookGateError::CapabilityMismatch {
                gate_ref: state.reservation.gate_ref.clone(),
                expected: state.reservation.capability_id.clone(),
                actual: request.capability_id,
            });
        }
        if request.arguments_digest != state.reservation.arguments_digest {
            return Err(HookGateError::ArgumentsDigestMismatch {
                gate_ref: state.reservation.gate_ref.clone(),
            });
        }
        state.consumed_at = Some(resolved_at);
        Ok(HookGateResolution {
            gate_ref: state.reservation.gate_ref.clone(),
            kind: state.reservation.kind,
            run_context: state.reservation.run_context.clone(),
            actor: state.reservation.actor.clone(),
            capability_id: state.reservation.capability_id.clone(),
            arguments_digest: state.reservation.arguments_digest.clone(),
            resolved_at,
        })
    }
}

#[derive(Debug, Clone)]
struct InMemoryReservation {
    reservation: HookGateReservation,
    consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookGateError {
    InvalidTtl,
    MissingInvocationMetadata,
    /// Argument digest didn't satisfy the `sha256:<64-hex>` shape. Distinct
    /// from [`Self::InvalidToken`] (which covers non-digest opaque tokens
    /// like actor/session ids) — henrypark133 must-fix #3 on PR #3633.
    InvalidDigest(String),
    /// A non-digest opaque token (actor id, session id, etc.) failed
    /// validation. `field` names the rejected token type; `reason`
    /// describes the failure (empty, over-limit, control characters).
    InvalidToken {
        field: &'static str,
        reason: String,
    },
    InvalidGateRef(String),
    UnknownGate {
        gate_ref: LoopGateRef,
    },
    AlreadyConsumed {
        gate_ref: LoopGateRef,
        consumed_at: DateTime<Utc>,
    },
    Expired {
        gate_ref: LoopGateRef,
        expires_at: DateTime<Utc>,
    },
    KindMismatch {
        gate_ref: LoopGateRef,
        expected: HookGateKind,
        actual: HookGateKind,
    },
    RunMismatch {
        gate_ref: LoopGateRef,
    },
    ActorMismatch {
        gate_ref: LoopGateRef,
    },
    CapabilityMismatch {
        gate_ref: LoopGateRef,
        expected: CapabilityId,
        actual: CapabilityId,
    },
    ArgumentsDigestMismatch {
        gate_ref: LoopGateRef,
    },
    Backend(String),
}

impl HookGateError {
    pub fn is_already_consumed(&self) -> bool {
        matches!(self, Self::AlreadyConsumed { .. })
    }

    pub fn is_actor_mismatch(&self) -> bool {
        matches!(self, Self::ActorMismatch { .. })
    }

    pub fn is_expired(&self) -> bool {
        matches!(self, Self::Expired { .. })
    }
}

impl fmt::Display for HookGateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTtl => formatter.write_str("hook gate ttl must be positive and bounded"),
            Self::MissingInvocationMetadata => {
                formatter.write_str("hook gate reservation missing invocation metadata")
            }
            Self::InvalidDigest(reason) => write!(formatter, "invalid hook gate digest: {reason}"),
            Self::InvalidToken { field, reason } => {
                write!(formatter, "invalid hook gate {field}: {reason}")
            }
            Self::InvalidGateRef(reason) => {
                write!(formatter, "invalid router-issued hook gate ref: {reason}")
            }
            Self::UnknownGate { gate_ref } => {
                write!(formatter, "hook gate ref {} is unknown", gate_ref.as_str())
            }
            Self::AlreadyConsumed { gate_ref, .. } => {
                write!(
                    formatter,
                    "hook gate ref {} was already consumed",
                    gate_ref.as_str()
                )
            }
            Self::Expired {
                gate_ref,
                expires_at,
            } => write!(
                formatter,
                "hook gate ref {} expired at {expires_at}",
                gate_ref.as_str()
            ),
            Self::KindMismatch {
                gate_ref,
                expected,
                actual,
            } => write!(
                formatter,
                "hook gate ref {} expected {} gate, got {}",
                gate_ref.as_str(),
                expected.as_str(),
                actual.as_str()
            ),
            Self::RunMismatch { gate_ref } => {
                write!(
                    formatter,
                    "hook gate ref {} belongs to another run",
                    gate_ref.as_str()
                )
            }
            Self::ActorMismatch { gate_ref } => write!(
                formatter,
                "hook gate ref {} belongs to another actor/session",
                gate_ref.as_str()
            ),
            Self::CapabilityMismatch {
                gate_ref,
                expected,
                actual,
            } => write!(
                formatter,
                "hook gate ref {} expected capability {}, got {}",
                gate_ref.as_str(),
                expected.as_str(),
                actual.as_str()
            ),
            Self::ArgumentsDigestMismatch { gate_ref } => write!(
                formatter,
                "hook gate ref {} arguments digest did not match",
                gate_ref.as_str()
            ),
            Self::Backend(reason) => write!(formatter, "hook gate router backend error: {reason}"),
        }
    }
}

impl Error for HookGateError {}

impl From<HookGateError> for AgentLoopHostError {
    fn from(error: HookGateError) -> Self {
        // henrypark133 must-fix #5 on PR #3633: collapse consumption-
        // failure variants to an opaque public surface. Distinguishing
        // "this gate ref doesn't exist" from "this gate ref belongs to
        // another run/actor/capability" is an oracle that lets a probing
        // caller learn which gate refs are live. Internally the variants
        // are still distinct (for routing + assertion ergonomics in tests
        // + operator-visible logs); the public `AgentLoopHostError` says
        // only "consumption denied".
        //
        // Also: a `tracing::warn!` here gives operators an observable
        // signal that distinguishes security rejections (consumption
        // failures) from availability failures (router unreachable).
        // henrypark133 non-blocking #9.
        tracing::warn!(
            error = %error,
            "hook gate router rejected request; mapping to opaque AgentLoopHostError"
        );
        let safe_summary: String = match &error {
            // Consumption-failure family: collapse oracle.
            HookGateError::UnknownGate { .. }
            | HookGateError::AlreadyConsumed { .. }
            | HookGateError::Expired { .. }
            | HookGateError::KindMismatch { .. }
            | HookGateError::RunMismatch { .. }
            | HookGateError::ActorMismatch { .. }
            | HookGateError::CapabilityMismatch { .. }
            | HookGateError::ArgumentsDigestMismatch { .. } => {
                "hook gate consumption denied".to_string()
            }
            // Misuse / availability failures: surface the variant detail
            // (these are not oracles — they signal configuration / wiring
            // bugs and need to be operator-visible).
            _ => format!("hook gate router unavailable: {error}"),
        };
        AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, safe_summary)
    }
}

fn validate_digest(value: &str) -> Result<(), HookGateError> {
    if value.starts_with("sha256:")
        && value.len() == "sha256:".len() + 64
        && value["sha256:".len()..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(HookGateError::InvalidDigest(
            "expected sha256:<64 hex characters>".to_string(),
        ))
    }
}

fn validate_token(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, HookGateError> {
    // henrypark133 must-fix #3 on PR #3633: route through `InvalidToken`
    // rather than `InvalidDigest` so the error name matches what's being
    // validated (actor / session id, not a sha256 digest).
    if value.is_empty() {
        return Err(HookGateError::InvalidToken {
            field: label,
            reason: "must not be empty".to_string(),
        });
    }
    if value.len() > max_bytes {
        return Err(HookGateError::InvalidToken {
            field: label,
            reason: format!("must be at most {max_bytes} bytes"),
        });
    }
    if value.chars().any(|character| character.is_control()) {
        return Err(HookGateError::InvalidToken {
            field: label,
            reason: "must not contain control characters".to_string(),
        });
    }
    Ok(value)
}

fn same_run_context(left: &LoopRunContext, right: &LoopRunContext) -> bool {
    left.scope == right.scope
        && left.thread_id == right.thread_id
        && left.turn_id == right.turn_id
        && left.run_id == right.run_id
}
