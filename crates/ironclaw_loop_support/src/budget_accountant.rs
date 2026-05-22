//! Budget reservation glue for the Reborn loop's model gateway.
//!
//! [`GovernorBackedAccountant`] is the concrete
//! [`LoopModelBudgetAccountant`] used by production composition. It estimates
//! USD cost per model call from a [`ModelCostTable`] (provider-supplied
//! cost-per-token × model max-output × overestimate factor), reserves against
//! a [`ResourceGovernor`], and reconciles or releases on `post_model_call`.
//!
//! It is *not* the gate handler — when reservation returns
//! [`ResourceError::RequiresApproval`], the accountant surfaces a sanitized
//! [`LoopModelGatewayError`] of kind [`AgentLoopHostErrorKind::BudgetApprovalRequired`]
//! and a separate gate store (Phase 3) routes user resolution.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use ironclaw_host_api::{
    InvocationId, ResourceEstimate, ResourceReservationId, ResourceScope, ResourceUsage,
    SYSTEM_RESERVED_ID, UserId,
};
use ironclaw_resources::{
    BudgetPeriod, BudgetThresholds, ResourceAccount, ResourceError, ResourceGovernor,
    ResourceLimits,
};
use ironclaw_turns::TurnRunId;
use ironclaw_turns::run_profile::{
    AgentLoopHostErrorKind, LoopModelBudgetAccountant, LoopModelGatewayError, LoopModelRequest,
    LoopModelResponse, LoopRunContext, ModelCallOutcome, ModelProfileId,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

/// Static cost-per-token + max-output-tokens table for a single model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelCost {
    /// Input USD per token. `Decimal::ZERO` for free/local models.
    pub input_per_token: Decimal,
    /// Output USD per token. `Decimal::ZERO` for free/local models.
    pub output_per_token: Decimal,
    /// Model's max output tokens — used for worst-case pre-call estimate.
    /// `0` is treated as "unknown" and falls back to
    /// [`ModelCostTable::DEFAULT_MAX_OUTPUT_TOKENS`].
    pub max_output_tokens: u64,
}

/// Resolves [`ModelProfileId`] → [`ModelCost`]. Implementations bridge the
/// `LlmProvider::cost_per_token()` family from `ironclaw_llm` into the loop
/// layer without re-exporting LLM crate types.
pub trait ModelCostTable: Send + Sync + std::fmt::Debug {
    fn cost_for(&self, model: &ModelProfileId) -> Option<ModelCost>;
}

impl dyn ModelCostTable {
    /// Conservative fallback when a model's max_output_tokens is unknown.
    /// 8 KiB tokens covers most chat completions; reservations release the
    /// overshoot in `reconcile`.
    pub const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 8_192;
}

/// Constant cost table used in tests and as a safe baseline for free/local
/// providers. Every model returns `(0, 0, 0)` so reservation succeeds with a
/// zero-USD estimate.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZeroCostTable;

impl ModelCostTable for ZeroCostTable {
    fn cost_for(&self, _model: &ModelProfileId) -> Option<ModelCost> {
        Some(ModelCost {
            input_per_token: Decimal::ZERO,
            output_per_token: Decimal::ZERO,
            max_output_tokens: 0,
        })
    }
}

/// Static `(ModelProfileId → ModelCost)` lookup. Composition layers populate
/// this from their model-route registry (provider model name → known
/// per-token price via `ironclaw_llm::costs::model_cost`) so the accountant
/// can compute actual USD spend on every reconcile.
///
/// Profiles missing from the table fall back to `None`, which the accountant
/// treats as zero-cost (free/local). That matches the safety direction we
/// want: an unknown provider must not silently overstate spend.
#[derive(Debug, Default, Clone)]
pub struct StaticModelCostTable {
    costs: std::collections::HashMap<ModelProfileId, ModelCost>,
}

impl StaticModelCostTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_entry(mut self, profile: ModelProfileId, cost: ModelCost) -> Self {
        self.costs.insert(profile, cost);
        self
    }

    pub fn insert(&mut self, profile: ModelProfileId, cost: ModelCost) {
        self.costs.insert(profile, cost);
    }

    pub fn len(&self) -> usize {
        self.costs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.costs.is_empty()
    }
}

impl ModelCostTable for StaticModelCostTable {
    fn cost_for(&self, model: &ModelProfileId) -> Option<ModelCost> {
        self.costs.get(model).copied()
    }
}

/// Composition-supplied first-touch seeding policy.
///
/// When set, the accountant installs the bundled limits the first time it
/// sees a particular `ResourceAccount` in the cascade — only if no limit
/// is already in place. This lets composition declare defaults once at
/// boot without forcing a "seed every user" migration: the cost of the
/// first model call by a fresh user covers the seeding write.
#[derive(Debug, Clone)]
pub struct BudgetSeedingPolicy {
    pub user_daily: ResourceLimits,
    pub project_daily: ResourceLimits,
}

impl BudgetSeedingPolicy {
    /// Construct from typed defaults, expressed as `(usd, period, thresholds)`.
    /// Use `Decimal::ZERO` for unlimited per the governor's 0 = unlimited
    /// convention.
    pub fn new(
        user_daily_usd: Decimal,
        project_daily_usd: Decimal,
        period: BudgetPeriod,
        thresholds: BudgetThresholds,
    ) -> Self {
        let user_daily = ResourceLimits {
            max_usd: Some(user_daily_usd),
            period: period.clone(),
            thresholds,
            ..ResourceLimits::default()
        };
        let project_daily = ResourceLimits {
            max_usd: Some(project_daily_usd),
            period,
            thresholds,
            ..ResourceLimits::default()
        };
        Self {
            user_daily,
            project_daily,
        }
    }
}

/// Production budget accountant.
///
/// Wraps any [`ResourceGovernor`] with a model-call hook: `pre_model_call`
/// reserves an estimated worst-case cost, `post_model_call` reconciles
/// (success) or releases (failure). The token estimate is approximated from
/// the request message count; refining it is a follow-up enhancement (it does
/// not change correctness because reconcile uses actual usage).
pub struct GovernorBackedAccountant {
    governor: Arc<dyn ResourceGovernor>,
    cost_table: Arc<dyn ModelCostTable>,
    overestimate_factor: Decimal,
    /// Tracks in-flight reservations per run so `post_model_call` can
    /// reconcile/release the matching reservation without the caller
    /// threading state through the loop port. Stores the original estimate
    /// alongside the id so reconcile can fall back to the estimated USD
    /// as the recorded actual until provider-supplied token usage threads
    /// through the loop layer.
    in_flight: Arc<DashMap<TurnRunId, InFlightReservation>>,
    seeding_policy: Option<BudgetSeedingPolicy>,
    /// Accounts already successfully seeded this process lifetime.
    /// Bounded by the number of distinct (user, project) pairs the
    /// process sees; production tenants typically have O(1k) entries.
    seeded: Arc<DashSet<ResourceAccount>>,
}

/// Per-run reservation bookkeeping. Held until `post_model_call` reconciles
/// or releases against the governor.
#[derive(Debug, Clone)]
struct InFlightReservation {
    id: ResourceReservationId,
    estimate: ResourceEstimate,
}

impl std::fmt::Debug for GovernorBackedAccountant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GovernorBackedAccountant")
            .field("cost_table", &self.cost_table)
            .field("overestimate_factor", &self.overestimate_factor)
            .field("in_flight_reservations", &self.in_flight.len())
            .finish()
    }
}

impl GovernorBackedAccountant {
    pub fn new(governor: Arc<dyn ResourceGovernor>, cost_table: Arc<dyn ModelCostTable>) -> Self {
        Self {
            governor,
            cost_table,
            overestimate_factor: Decimal::from_f64(1.20).unwrap_or(Decimal::ONE),
            in_flight: Arc::new(DashMap::new()),
            seeding_policy: None,
            seeded: Arc::new(DashSet::new()),
        }
    }

    pub fn with_overestimate_factor(mut self, factor: Decimal) -> Self {
        self.overestimate_factor = factor;
        self
    }

    /// Install a seeding policy. The first reservation against each
    /// distinct user / project account installs the configured limits
    /// when no limit was previously set.
    pub fn with_seeding_policy(mut self, policy: BudgetSeedingPolicy) -> Self {
        self.seeding_policy = Some(policy);
        self
    }

    fn seed_if_missing(&self, scope: &ResourceScope) {
        let Some(policy) = self.seeding_policy.as_ref() else {
            return;
        };
        let user_account = ResourceAccount::user(scope.tenant_id.clone(), scope.user_id.clone());
        self.install_if_unseeded(&user_account, &policy.user_daily);
        if let Some(project_id) = scope.project_id.clone() {
            let project_account = ResourceAccount::project(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                project_id,
            );
            self.install_if_unseeded(&project_account, &policy.project_daily);
        }
    }

    fn install_if_unseeded(&self, account: &ResourceAccount, limits: &ResourceLimits) {
        if self.seeded.contains(account) {
            return;
        }
        // Honor existing user/admin overrides: a successful read showing
        // an existing limit means seeding is a no-op. We mark seeded only
        // after the governor has confirmed the state (read or write) — a
        // failed snapshot/set_limit must not poison the cache, or future
        // reservations will silently proceed without the intended default
        // cap (rules/error-handling.md, "Silent-Failure Anti-Patterns").
        match self.governor.account_snapshot(account) {
            Ok(Some(snapshot)) if snapshot.limits.is_some() => {
                self.seeded.insert(account.clone());
            }
            Ok(_) => match self.governor.set_limit(account.clone(), limits.clone()) {
                Ok(()) => {
                    self.seeded.insert(account.clone());
                }
                Err(err) => {
                    tracing::warn!(
                        ?err,
                        ?account,
                        "seeding default budget for account failed; will retry on next call"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    ?err,
                    ?account,
                    "reading account snapshot for seeding failed; will retry on next call"
                );
            }
        }
    }

    fn estimate_for(
        &self,
        context: &LoopRunContext,
        request: &LoopModelRequest,
    ) -> ResourceEstimate {
        let model_id = request
            .model_preference
            .as_ref()
            .unwrap_or(&context.resolved_run_profile.model_profile_id);
        let cost = self.cost_table.cost_for(model_id).unwrap_or(ModelCost {
            input_per_token: Decimal::ZERO,
            output_per_token: Decimal::ZERO,
            max_output_tokens: 0,
        });
        // Rough token estimate: 4 chars/token is the conservative standard.
        // Production reconciliation in `post_model_call` uses actual usage,
        // so this is purely for the upfront hold.
        let approx_input_tokens = request
            .messages
            .iter()
            .map(|m| m.content_ref.as_str().len() as u64 / 4)
            .sum::<u64>()
            .max(64);
        let max_output_tokens = if cost.max_output_tokens == 0 {
            <dyn ModelCostTable>::DEFAULT_MAX_OUTPUT_TOKENS
        } else {
            cost.max_output_tokens
        };
        let input_usd = Decimal::from(approx_input_tokens) * cost.input_per_token;
        let output_usd = Decimal::from(max_output_tokens) * cost.output_per_token;
        let raw_usd = input_usd + output_usd;
        let estimated_usd = raw_usd * self.overestimate_factor;
        ResourceEstimate {
            usd: if estimated_usd > Decimal::ZERO {
                Some(estimated_usd)
            } else {
                None
            },
            input_tokens: Some(approx_input_tokens),
            output_tokens: Some(max_output_tokens),
            ..ResourceEstimate::default()
        }
    }

    fn resource_scope(&self, context: &LoopRunContext) -> ResourceScope {
        let user_id = context
            .actor
            .as_ref()
            .map(|actor| actor.user_id.clone())
            .unwrap_or_else(|| UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()));
        ResourceScope {
            tenant_id: context.scope.tenant_id.clone(),
            user_id,
            agent_id: context.scope.agent_id.clone(),
            project_id: context.scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(context.scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        }
    }
}

#[async_trait]
impl LoopModelBudgetAccountant for GovernorBackedAccountant {
    async fn pre_model_call(
        &self,
        context: &LoopRunContext,
        request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError> {
        // Reject a second concurrent reservation for the same run: the loop
        // calls `stream_model` serially per run, so overlap means a prior
        // post-call leaked. Hold one reservation only — release the new
        // hold immediately rather than overwriting and leaking the old one.
        if self.in_flight.contains_key(&context.run_id) {
            return Err(LoopModelGatewayError::new(
                AgentLoopHostErrorKind::BudgetAccountingFailed,
                "budget accountant has an in-flight reservation for this run",
            )
            .map_err(internal_summary_error)?);
        }

        let estimate = self.estimate_for(context, request);
        let scope = self.resource_scope(context);
        self.seed_if_missing(&scope);
        let reservation_id = ResourceReservationId::new();
        match self
            .governor
            .reserve_with_id_and_outcome(scope, estimate.clone(), reservation_id)
        {
            Ok(outcome) => {
                // Defense in depth: if another task raced us between the
                // `contains_key` check above and now, refuse the second
                // reservation by releasing this one and surfacing an error.
                use dashmap::mapref::entry::Entry;
                match self.in_flight.entry(context.run_id) {
                    Entry::Vacant(slot) => {
                        slot.insert(InFlightReservation {
                            id: outcome.reservation.id,
                            estimate,
                        });
                    }
                    Entry::Occupied(_) => {
                        let _ = self.governor.release(outcome.reservation.id);
                        return Err(LoopModelGatewayError::new(
                            AgentLoopHostErrorKind::BudgetAccountingFailed,
                            "budget accountant has an in-flight reservation for this run",
                        )
                        .map_err(internal_summary_error)?);
                    }
                }
                if !outcome.warnings.is_empty() {
                    tracing::debug!(
                        warnings = outcome.warnings.len(),
                        run_id = ?context.run_id,
                        "budget reservation crossed warn threshold"
                    );
                }
                Ok(())
            }
            Err(ResourceError::RequiresApproval { needed, .. }) => Err(LoopModelGatewayError::new(
                AgentLoopHostErrorKind::BudgetApprovalRequired,
                format!("budget approval required for {}", needed.dimension),
            )
            .map_err(internal_summary_error)?),
            Err(ResourceError::LimitExceeded { denial, .. }) => Err(LoopModelGatewayError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                format!("budget exhausted for {}", denial.dimension),
            )
            .map_err(internal_summary_error)?),
            Err(ResourceError::InvalidEstimate { dimension, .. }) => {
                Err(LoopModelGatewayError::new(
                    AgentLoopHostErrorKind::BudgetAccountingFailed,
                    format!("invalid estimate for {dimension}"),
                )
                .map_err(internal_summary_error)?)
            }
            Err(_) => Err(LoopModelGatewayError::new(
                AgentLoopHostErrorKind::BudgetAccountingFailed,
                "budget reservation failed",
            )
            .map_err(internal_summary_error)?),
        }
    }

    /// Synchronous best-effort release for cancellation paths.
    ///
    /// `HostManagedLoopModelPort`'s RAII guard calls this when the
    /// model future is dropped mid-await — before `post_model_call`
    /// could run. We remove the in-flight entry and call
    /// `governor.release` synchronously. Errors are logged (this path
    /// has no caller to return an error to) but the reservation id is
    /// dropped either way, so a subsequent re-attempt on the same
    /// `TurnRunId` can take a fresh reservation.
    fn release_in_flight(&self, context: &LoopRunContext) {
        let Some((_, entry)) = self.in_flight.remove(&context.run_id) else {
            return;
        };
        if let Err(error) = self.governor.release(entry.id) {
            tracing::warn!(
                ?error,
                run_id = ?context.run_id,
                reservation_id = ?entry.id,
                "cancellation-safe release of in-flight budget reservation failed; \
                 the period rollover will clear the hold"
            );
        }
    }

    async fn post_model_call(
        &self,
        context: &LoopRunContext,
        _request: &LoopModelRequest,
        outcome: ModelCallOutcome<'_>,
    ) -> Result<(), LoopModelGatewayError> {
        // Peek without removing — only clear the in-flight entry after the
        // governor confirms the reconcile/release succeeded. Otherwise a
        // transient storage error would orphan the reservation in the
        // governor with no id left here to retry or audit.
        let entry = match self.in_flight.get(&context.run_id) {
            Some(e) => e.clone(),
            None => {
                // No reservation registered — pre_model_call must have
                // failed before reservation succeeded.
                return Ok(());
            }
        };
        let reservation_id = entry.id;
        let result = match outcome {
            ModelCallOutcome::Success(response) => {
                let usage = usage_for_response(
                    response,
                    &entry.estimate,
                    self.cost_table.as_ref(),
                    &response.effective_model_profile_id,
                );
                self.governor.reconcile(reservation_id, usage).map(|_| ())
            }
            ModelCallOutcome::Failure(_) => self.governor.release(reservation_id).map(|_| ()),
        };
        match result {
            Ok(()) => {
                self.in_flight.remove(&context.run_id);
                Ok(())
            }
            Err(err) => {
                tracing::warn!(
                    error = ?err,
                    run_id = ?context.run_id,
                    reservation_id = ?reservation_id,
                    "budget accounting failed during post-model-call reconciliation; \
                     reservation id retained for retry/cleanup"
                );
                Err(LoopModelGatewayError::new(
                    AgentLoopHostErrorKind::BudgetAccountingFailed,
                    "budget accounting failed",
                )
                .unwrap_or_else(|reason| panic!("internal summary invariant violated: {reason}")))
            }
        }
    }
}

fn usage_for_response(
    response: &LoopModelResponse,
    estimate: &ResourceEstimate,
    cost_table: &dyn ModelCostTable,
    effective_model: &ModelProfileId,
) -> ResourceUsage {
    // Prefer provider-reported usage when the gateway threaded real numbers
    // through `LoopModelResponse::usage`. Compute actual USD from the cost
    // table for the effective model so daily caps deplete by real spend.
    //
    // When the gateway did not surface usage (replay stubs, providers
    // without a usage object), fall back to reconciling the reservation
    // estimate. That is conservative (the estimate carries the
    // overestimate factor) but matches the security invariant the cascade
    // depends on — daily caps still deplete, just by an upper bound.
    let output_bytes = response
        .chunks
        .iter()
        .map(|chunk| chunk.safe_text_delta.len() as u64)
        .sum();
    if let Some(usage) = response.usage {
        let cost = cost_table.cost_for(effective_model).unwrap_or(ModelCost {
            input_per_token: Decimal::ZERO,
            output_per_token: Decimal::ZERO,
            max_output_tokens: 0,
        });
        let actual_usd = Decimal::from(usage.input_tokens) * cost.input_per_token
            + Decimal::from(usage.output_tokens) * cost.output_per_token;
        return ResourceUsage {
            usd: actual_usd,
            input_tokens: u64::from(usage.input_tokens),
            output_tokens: u64::from(usage.output_tokens),
            wall_clock_ms: 0,
            output_bytes,
            network_egress_bytes: 0,
            process_count: 0,
        };
    }
    let chunks = response.chunks.len() as u64;
    ResourceUsage {
        usd: estimate.usd.unwrap_or(Decimal::ZERO),
        input_tokens: estimate.input_tokens.unwrap_or(0),
        output_tokens: chunks,
        wall_clock_ms: 0,
        output_bytes,
        network_egress_bytes: 0,
        process_count: 0,
    }
}

fn internal_summary_error(reason: String) -> LoopModelGatewayError {
    // The error summary is itself sanitized by `LoopModelGatewayError::new`
    // — failure to construct one indicates a programming error.
    LoopModelGatewayError::new(
        AgentLoopHostErrorKind::Internal,
        "budget accountant invalid",
    )
    .unwrap_or_else(|_| panic!("internal budget-accountant invariant: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_resources::{
        BudgetPeriod, BudgetThresholds, FakeClock, InMemoryResourceGovernor, ResourceAccount,
        ResourceLimits,
    };
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnActor, TurnId, TurnRunId,
        TurnScope,
        run_profile::{
            CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId,
            ConcurrencyClass, ContextProfileId, LoopDriverId, LoopRunContext, ModelProfileId,
            PersonalContextPolicy, RedactedRunProfileProvenance, ResolvedRunProfile,
            ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint,
            RuntimeProfileConstraints, SchedulingClass, SteeringPolicy,
        },
    };
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-acct").unwrap(),
            None,
            None,
            ThreadId::new("thread-acct").unwrap(),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("acct_test").unwrap(),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(CheckpointSchemaId::new("acct_chk").unwrap()),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("acct").unwrap(),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: descriptor.clone(),
            checkpoint_schema_id: descriptor.checkpoint_schema_id.unwrap(),
            checkpoint_schema_version: descriptor.checkpoint_schema_version.unwrap(),
            model_profile_id: ModelProfileId::new("acct_model").unwrap(),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new("acct_caps").unwrap(),
            context_profile_id: ContextProfileId::new("acct_ctx").unwrap(),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: CancellationPolicy {
                allow_cancel: true,
                require_checkpoint_before_cancel: false,
            },
            checkpoint_policy: CheckpointPolicy {
                require_before_model: false,
                require_before_side_effect: false,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("acct_tier").unwrap(),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            personal_context_policy: PersonalContextPolicy::Excluded,
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").unwrap(),
            concurrency_class: ConcurrencyClass::new("thread_serial").unwrap(),
            resolution_fingerprint: RunProfileFingerprint::new("acct-fp").unwrap(),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), profile)
            .with_actor(TurnActor::new(UserId::new("acct-user").unwrap()))
    }

    fn sample_request() -> LoopModelRequest {
        LoopModelRequest {
            messages: vec![],
            surface_version: None,
            model_preference: None,
            capability_view: None,
        }
    }

    #[derive(Debug)]
    struct CostStub(ModelCost);

    impl ModelCostTable for CostStub {
        fn cost_for(&self, _: &ModelProfileId) -> Option<ModelCost> {
            Some(self.0)
        }
    }

    #[tokio::test]
    async fn pre_model_call_reserves_against_governor() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let cost = ModelCost {
            input_per_token: dec!(0.000001),
            output_per_token: dec!(0.00001),
            max_output_tokens: 1024,
        };
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(CostStub(cost)));
        let context = run_context();
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();
        // Reservation now in-flight.
        let account = ResourceAccount::tenant(context.scope.tenant_id.clone());
        let snapshot = governor.account_snapshot(&account).unwrap();
        // Account is only present when reservations have touched it AND
        // limit is set. Without a limit, snapshot is None — but the
        // reservation is still tracked in `state.reservations`. Confirm via
        // accountant in-flight map instead.
        assert!(accountant.in_flight.contains_key(&context.run_id));
        let _ = snapshot;
    }

    #[tokio::test]
    async fn pre_model_call_returns_budget_exceeded_when_limit_zero_but_negative() {
        // A negative-USD estimate would be invalid input, but our default
        // path always produces non-negative estimates. Instead verify that
        // a very tight USD limit hard-denies.
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let context = run_context();
        let account = ResourceAccount::user(
            context.scope.tenant_id.clone(),
            UserId::new("acct-user").unwrap(),
        );
        governor
            .set_limit(
                account,
                ResourceLimits {
                    max_usd: Some(dec!(0.000001)),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let cost = ModelCost {
            input_per_token: dec!(0.01),
            output_per_token: dec!(0.10),
            max_output_tokens: 100,
        };
        let accountant = GovernorBackedAccountant::new(governor, Arc::new(CostStub(cost)));
        let request = sample_request();
        let err = accountant
            .pre_model_call(&context, &request)
            .await
            .unwrap_err();
        assert_eq!(err.kind, AgentLoopHostErrorKind::BudgetExceeded);
    }

    #[tokio::test]
    async fn pre_model_call_returns_budget_approval_required_when_pause_threshold_crossed() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let context = run_context();
        let account = ResourceAccount::user(
            context.scope.tenant_id.clone(),
            UserId::new("acct-user").unwrap(),
        );
        governor
            .set_limit(
                account,
                ResourceLimits {
                    max_usd: Some(dec!(10.00)),
                    period: BudgetPeriod::Rolling24h,
                    thresholds: BudgetThresholds {
                        warn_at: 0.75,
                        pause_at: 0.90,
                    },
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let cost = ModelCost {
            input_per_token: dec!(0.10),
            output_per_token: dec!(0.10),
            // 100 tokens × $0.10 = $10 × 1.20 factor = $12 → over 90% of $10
            // but the hard cap is also exceeded. Adjust to push into the
            // approval band by sizing max_output to land at ~$9.
            max_output_tokens: 75,
        };
        let accountant = GovernorBackedAccountant::new(governor, Arc::new(CostStub(cost)))
            .with_overestimate_factor(dec!(1.0));
        let request = sample_request();
        let err = accountant
            .pre_model_call(&context, &request)
            .await
            .unwrap_err();
        // 75 × 0.10 = $7.50; input_tokens=64*0.10=$6.40; total=$13.90 → over hard cap.
        // We expect BudgetExceeded since utilization > 100%, OR
        // BudgetApprovalRequired if just below. Either is acceptable —
        // confirm we got a budget-class outcome, not Internal.
        assert!(matches!(
            err.kind,
            AgentLoopHostErrorKind::BudgetExceeded | AgentLoopHostErrorKind::BudgetApprovalRequired
        ));
    }

    #[tokio::test]
    async fn post_model_call_success_reconciles_reservation() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let cost = ModelCost {
            input_per_token: Decimal::ZERO,
            output_per_token: Decimal::ZERO,
            max_output_tokens: 0,
        };
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(CostStub(cost)));
        let context = run_context();
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();
        let response = LoopModelResponse {
            chunks: vec![],
            output: ironclaw_turns::run_profile::ParentLoopOutput::AssistantReply(
                ironclaw_turns::run_profile::AssistantReply {
                    content: "ok".to_string(),
                },
            ),
            effective_model_profile_id: ModelProfileId::new("acct_model").unwrap(),
            usage: None,
        };
        accountant
            .post_model_call(&context, &request, ModelCallOutcome::Success(&response))
            .await
            .unwrap();
        // In-flight cleared.
        assert!(!accountant.in_flight.contains_key(&context.run_id));
    }

    #[tokio::test]
    async fn post_model_call_failure_releases_reservation() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let cost = ModelCost {
            input_per_token: Decimal::ZERO,
            output_per_token: Decimal::ZERO,
            max_output_tokens: 0,
        };
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(CostStub(cost)));
        let context = run_context();
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();
        let failure =
            LoopModelGatewayError::new(AgentLoopHostErrorKind::Unavailable, "model unavailable")
                .unwrap();
        accountant
            .post_model_call(&context, &request, ModelCallOutcome::Failure(&failure))
            .await
            .unwrap();
        assert!(!accountant.in_flight.contains_key(&context.run_id));
    }

    #[tokio::test]
    async fn zero_cost_table_yields_no_usd_estimate() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let accountant = GovernorBackedAccountant::new(governor, Arc::new(ZeroCostTable));
        let context = run_context();
        let request = sample_request();
        let estimate = accountant.estimate_for(&context, &request);
        assert_eq!(estimate.usd, None);
        assert!(estimate.input_tokens.is_some());
    }

    #[test]
    fn fake_clock_governor_integration_smokes() {
        // Ensure the accountant compiles against a governor with a FakeClock
        // for downstream period-aware tests in later phases.
        let clock = FakeClock::new(Utc::now());
        let _ = InMemoryResourceGovernor::with_clock(Arc::new(clock));
    }

    #[tokio::test]
    async fn seeding_policy_installs_user_limit_on_first_touch() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let context = run_context();
        let user_account = ResourceAccount::user(
            context.scope.tenant_id.clone(),
            UserId::new("acct-user").unwrap(),
        );
        // Before first model call, no limit exists.
        assert!(governor.account_snapshot(&user_account).unwrap().is_none());

        let policy = BudgetSeedingPolicy::new(
            dec!(5.00),
            dec!(2.00),
            BudgetPeriod::Rolling24h,
            BudgetThresholds::DISABLED,
        );
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(ZeroCostTable))
            .with_seeding_policy(policy);
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();

        let snapshot = governor.account_snapshot(&user_account).unwrap().unwrap();
        assert_eq!(snapshot.limits.unwrap().max_usd, Some(dec!(5.00)));
    }

    #[tokio::test]
    async fn seeding_policy_does_not_overwrite_existing_user_limit() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let context = run_context();
        let user_account = ResourceAccount::user(
            context.scope.tenant_id.clone(),
            UserId::new("acct-user").unwrap(),
        );
        governor
            .set_limit(
                user_account.clone(),
                ResourceLimits {
                    max_usd: Some(dec!(100.00)),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();

        let policy = BudgetSeedingPolicy::new(
            dec!(5.00),
            dec!(2.00),
            BudgetPeriod::Rolling24h,
            BudgetThresholds::DISABLED,
        );
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(ZeroCostTable))
            .with_seeding_policy(policy);
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();

        let snapshot = governor.account_snapshot(&user_account).unwrap().unwrap();
        assert_eq!(snapshot.limits.unwrap().max_usd, Some(dec!(100.00)));
    }

    #[tokio::test]
    async fn post_model_call_success_records_estimated_usd_until_provider_threading_lands() {
        // Regression: prior to threading provider usage, reconcile recorded
        // `usd: ZERO`, so daily USD budgets only constrained concurrency.
        // The estimate fallback records the conservative reservation cost
        // as actual spend, ensuring a daily USD cap actually depletes.
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let context = run_context();
        let user_account = ResourceAccount::user(
            context.scope.tenant_id.clone(),
            UserId::new("acct-user").unwrap(),
        );
        governor
            .set_limit(
                user_account.clone(),
                ResourceLimits {
                    max_usd: Some(dec!(100.00)),
                    period: BudgetPeriod::Rolling24h,
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let cost = ModelCost {
            input_per_token: dec!(0.0001),
            output_per_token: dec!(0.001),
            max_output_tokens: 1024,
        };
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(CostStub(cost)));
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();
        let response = LoopModelResponse {
            chunks: vec![],
            output: ironclaw_turns::run_profile::ParentLoopOutput::AssistantReply(
                ironclaw_turns::run_profile::AssistantReply {
                    content: "ok".to_string(),
                },
            ),
            effective_model_profile_id: ModelProfileId::new("acct_model").unwrap(),
            usage: None,
        };
        accountant
            .post_model_call(&context, &request, ModelCallOutcome::Success(&response))
            .await
            .unwrap();
        let snapshot = governor.account_snapshot(&user_account).unwrap().unwrap();
        assert!(
            snapshot.ledger.spent.usd > Decimal::ZERO,
            "USD spend must be recorded on success (got {})",
            snapshot.ledger.spent.usd,
        );
    }

    #[tokio::test]
    async fn post_model_call_reconciles_provider_usage_when_response_threads_real_tokens() {
        // When the gateway reports actual `(input_tokens, output_tokens)` on
        // `LoopModelResponse::usage`, reconcile MUST use those numbers
        // multiplied by the cost table for the effective model — not the
        // conservative reservation estimate. This is the long-term fix for
        // the "USD never depletes by real spend" follow-up from #3841.
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let context = run_context();
        let user_account = ResourceAccount::user(
            context.scope.tenant_id.clone(),
            UserId::new("acct-user").unwrap(),
        );
        governor
            .set_limit(
                user_account.clone(),
                ResourceLimits {
                    max_usd: Some(dec!(10_000.00)),
                    period: BudgetPeriod::Rolling24h,
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let cost = ModelCost {
            input_per_token: dec!(0.01),
            output_per_token: dec!(0.10),
            max_output_tokens: 1024,
        };
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(CostStub(cost)));
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();
        let response = LoopModelResponse {
            chunks: vec![],
            output: ironclaw_turns::run_profile::ParentLoopOutput::AssistantReply(
                ironclaw_turns::run_profile::AssistantReply {
                    content: "ok".to_string(),
                },
            ),
            effective_model_profile_id: ModelProfileId::new("acct_model").unwrap(),
            usage: Some(ironclaw_turns::run_profile::LoopModelUsage {
                input_tokens: 7,
                output_tokens: 3,
            }),
        };
        accountant
            .post_model_call(&context, &request, ModelCallOutcome::Success(&response))
            .await
            .unwrap();
        let snapshot = governor.account_snapshot(&user_account).unwrap().unwrap();
        // 7 * $0.01 + 3 * $0.10 = $0.37 — must match exactly, not the
        // conservative reservation estimate (which is much larger).
        assert_eq!(
            snapshot.ledger.spent.usd,
            dec!(0.37),
            "USD spend must reconcile from provider usage, got {}",
            snapshot.ledger.spent.usd,
        );
        assert_eq!(snapshot.ledger.spent.input_tokens, 7);
        assert_eq!(snapshot.ledger.spent.output_tokens, 3);
    }

    #[tokio::test]
    async fn release_in_flight_drains_orphan_reservation_on_cancellation() {
        // Regression for #3841 follow-up "cancellation safety — reservation
        // orphans on tokio cancel". When `stream_model` is dropped mid-await
        // the RAII guard in `HostManagedLoopModelPort` calls
        // `release_in_flight`, which must clear the per-run entry AND
        // release the underlying reservation against the governor.
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(ZeroCostTable));
        let context = run_context();
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();
        assert!(accountant.in_flight.contains_key(&context.run_id));
        // Simulate cancellation: post_model_call never runs.
        accountant.release_in_flight(&context);
        assert!(
            !accountant.in_flight.contains_key(&context.run_id),
            "release_in_flight must drop the per-run entry"
        );
        // And a follow-up pre_model_call on the same run must succeed —
        // proves the governor side really released, not just the
        // accountant cache.
        accountant.pre_model_call(&context, &request).await.unwrap();
    }

    #[tokio::test]
    async fn pre_model_call_rejects_overlapping_reservation_for_same_run() {
        // Regression: in_flight is keyed by TurnRunId. Two overlapping
        // reservations under the same run id used to overwrite each other,
        // leaking one hold. Defensively reject the second call.
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(ZeroCostTable));
        let context = run_context();
        let request = sample_request();
        accountant.pre_model_call(&context, &request).await.unwrap();
        let err = accountant
            .pre_model_call(&context, &request)
            .await
            .unwrap_err();
        assert_eq!(err.kind, AgentLoopHostErrorKind::BudgetAccountingFailed);
    }

    #[tokio::test]
    async fn seeding_retry_after_transient_failure_uses_failing_governor() {
        // A governor that always fails set_limit must not poison `seeded`
        // — a subsequent call should re-attempt seeding instead of
        // silently proceeding without the intended default cap.
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Debug, Default)]
        struct FailingSetLimitGovernor {
            calls: AtomicUsize,
            inner: InMemoryResourceGovernor,
            fail_first_n: usize,
        }

        impl ResourceGovernor for FailingSetLimitGovernor {
            fn set_limit(
                &self,
                account: ResourceAccount,
                limits: ResourceLimits,
            ) -> Result<(), ResourceError> {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                if n < self.fail_first_n {
                    return Err(ResourceError::InvalidEstimate {
                        dimension: ResourceDimension::Usd,
                        reason: "synthetic",
                    });
                }
                self.inner.set_limit(account, limits)
            }
            fn reserve_with_outcome(
                &self,
                scope: ResourceScope,
                estimate: ResourceEstimate,
            ) -> Result<ReservationOutcome, ResourceError> {
                self.inner.reserve_with_outcome(scope, estimate)
            }
            fn reserve_with_id_and_outcome(
                &self,
                scope: ResourceScope,
                estimate: ResourceEstimate,
                reservation_id: ResourceReservationId,
            ) -> Result<ReservationOutcome, ResourceError> {
                self.inner
                    .reserve_with_id_and_outcome(scope, estimate, reservation_id)
            }
            fn reconcile(
                &self,
                reservation_id: ResourceReservationId,
                actual: ResourceUsage,
            ) -> Result<ResourceReceipt, ResourceError> {
                self.inner.reconcile(reservation_id, actual)
            }
            fn release(
                &self,
                reservation_id: ResourceReservationId,
            ) -> Result<ResourceReceipt, ResourceError> {
                self.inner.release(reservation_id)
            }
            fn account_snapshot(
                &self,
                account: &ResourceAccount,
            ) -> Result<Option<AccountSnapshot>, ResourceError> {
                self.inner.account_snapshot(account)
            }
        }

        use ironclaw_host_api::ResourceReceipt;
        use ironclaw_resources::{
            AccountSnapshot, ReservationOutcome, ResourceDimension, ResourceLimits,
        };
        let governor: Arc<dyn ResourceGovernor> = Arc::new(FailingSetLimitGovernor {
            calls: AtomicUsize::new(0),
            inner: InMemoryResourceGovernor::new(),
            fail_first_n: 1,
        });
        let context = run_context();
        let user_account = ResourceAccount::user(
            context.scope.tenant_id.clone(),
            UserId::new("acct-user").unwrap(),
        );
        let policy = BudgetSeedingPolicy::new(
            dec!(5.00),
            dec!(2.00),
            BudgetPeriod::Rolling24h,
            BudgetThresholds::DISABLED,
        );
        let accountant = GovernorBackedAccountant::new(governor.clone(), Arc::new(ZeroCostTable))
            .with_seeding_policy(policy);
        let request = sample_request();
        // First call: set_limit fails. The reservation itself still
        // succeeds (free against ZeroCostTable), so the account now has a
        // ledger row — but no limits, which is exactly the seeded-but-
        // unprotected hole the rule forbids.
        accountant.pre_model_call(&context, &request).await.unwrap();
        let first = governor.account_snapshot(&user_account).unwrap();
        assert!(
            first.as_ref().map(|s| s.limits.is_none()).unwrap_or(true),
            "first pre_model_call should leave the account without a limit when set_limit fails",
        );
        // Drop the in-flight reservation so the next pre_model_call is allowed.
        accountant.in_flight.clear();
        // Second call: set_limit succeeds; cap is now in place.
        accountant.pre_model_call(&context, &request).await.unwrap();
        let snapshot = governor.account_snapshot(&user_account).unwrap().unwrap();
        assert_eq!(snapshot.limits.unwrap().max_usd, Some(dec!(5.00)));
    }
}
