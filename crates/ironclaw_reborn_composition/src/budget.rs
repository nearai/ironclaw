//! Shared budget-accountant composition helpers.
//!
//! Both the local-dev runtime (`build_reborn_runtime`) and any
//! production loop composer go through [`build_default_budget_accountant`]
//! so the same `BudgetDefaults`-derived seeding policy + overestimate
//! factor reach every code path that needs to enforce daily caps. Without
//! a shared helper, local-dev would seed defaults and production wouldn't
//! — the kind of split-brain configuration the #3899 review pass
//! flagged (review feedback High #2).

use std::sync::Arc;

use ironclaw_loop_support::{BudgetSeedingPolicy, GovernorBackedAccountant, ModelCostTable};
use ironclaw_reborn_config::{BudgetDefaults, BudgetDefaultsError};
use ironclaw_resources::{
    BudgetEventSink, BudgetGateStore, BudgetPeriod, BudgetThresholds, ResourceGovernor,
};
use ironclaw_turns::run_profile::LoopModelBudgetAccountant;
use rust_decimal::Decimal;

/// Build a production-shaped `GovernorBackedAccountant` from the
/// supplied substrate handles.
///
/// The accountant gets:
///
/// 1. The caller's `ResourceGovernor` (in-memory for local-dev,
///    `PersistentResourceGovernor` for libsql / postgres production).
/// 2. The caller's `ModelCostTable` (typically derived from
///    `LlmModelProfilePolicy::build_cost_table()` at startup).
/// 3. A `BudgetGateStore` (in-memory for local-dev,
///    `FilesystemBudgetGateStore` scoped to the tenant for production).
/// 4. A `BudgetSeedingPolicy` derived from
///    `BudgetDefaults::compiled_defaults().with_env()` so fresh
///    user/project accounts pick up the default daily cap on first
///    model call.
/// 5. The configured overestimate factor from the same defaults.
///
/// Returns `Err` only when `BudgetDefaults::with_env()` rejects an
/// invalid env-var value (parse error, negative USD, etc.) — every
/// other failure mode is captured by the input substrate handles.
///
/// **Production composition note**: this helper is the single source of
/// truth for how the accountant gets built. Production runtime
/// composers should call it with the persistent governor + filesystem
/// gate store + LLM-policy-derived cost table + the same event sink
/// wired into the governor, then thread the returned
/// `Arc<dyn LoopModelBudgetAccountant>` into the
/// `RebornLoopDriverHostFactory::with_model_budget_accountant` builder.
///
/// The accountant uses the supplied `event_sink` to emit
/// `BudgetEvent::GateOpened` after persisting a pending gate so SSE /
/// audit consumers receive the real `BudgetGateId` (the governor's
/// earlier `ApprovalRequested` event carries no gate id).
pub fn build_default_budget_accountant(
    governor: Arc<dyn ResourceGovernor>,
    cost_table: Arc<dyn ModelCostTable>,
    gate_store: Arc<dyn BudgetGateStore>,
    event_sink: Arc<dyn BudgetEventSink>,
) -> Result<Arc<dyn LoopModelBudgetAccountant>, BudgetDefaultsError> {
    let defaults = BudgetDefaults::compiled_defaults().with_env()?;
    let user_daily_usd = Decimal::from_f64_retain(defaults.user_daily_usd).unwrap_or_default();
    let project_daily_usd =
        Decimal::from_f64_retain(defaults.project_daily_usd).unwrap_or_default();
    let overestimate_factor =
        Decimal::from_f64_retain(defaults.overestimate_factor).unwrap_or(Decimal::ONE);
    let seeding_policy = BudgetSeedingPolicy::new(
        user_daily_usd,
        project_daily_usd,
        BudgetPeriod::Rolling24h,
        BudgetThresholds {
            warn_at: defaults.warn_at,
            pause_at: defaults.pause_at,
        },
    );
    let accountant = GovernorBackedAccountant::new(governor, cost_table)
        .with_overestimate_factor(overestimate_factor)
        .with_seeding_policy(seeding_policy)
        .with_gate_store(gate_store)
        .with_event_sink(event_sink);
    Ok(Arc::new(accountant))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, ResourceEstimate, ResourceScope, TenantId, UserId};
    use ironclaw_loop_support::ZeroCostTable;
    use ironclaw_resources::{
        InMemoryBudgetEventSink, InMemoryBudgetGateStore, InMemoryResourceGovernor, ResourceAccount,
    };
    use rust_decimal_macros::dec;

    /// The helper installs the compiled-default $5 user cap on the
    /// first reservation against a fresh user. Equivalent to the e2e
    /// `d3_seeding_policy_installs_default_cap_on_first_touch` test
    /// but at the composition-helper tier — proves the wiring is
    /// reusable independent of `build_reborn_runtime`.
    #[tokio::test]
    async fn seeds_compiled_default_user_cap_on_first_touch() {
        let governor: Arc<dyn ResourceGovernor> = Arc::new(InMemoryResourceGovernor::new());
        let gate_store: Arc<dyn BudgetGateStore> = Arc::new(InMemoryBudgetGateStore::new());
        let cost_table: Arc<dyn ModelCostTable> = Arc::new(ZeroCostTable);
        let event_sink: Arc<dyn BudgetEventSink> = Arc::new(InMemoryBudgetEventSink::new());
        let accountant = build_default_budget_accountant(
            Arc::clone(&governor),
            cost_table,
            gate_store,
            event_sink,
        )
        .expect("budget defaults env-load succeeds");

        // Drive one `pre_model_call` to fire the seeding policy.
        let context = test_run_context("tenant-shared-helper", "alice-shared-helper");
        let request = ironclaw_turns::run_profile::LoopModelRequest {
            messages: vec![],
            surface_version: None,
            model_preference: None,
            capability_view: None,
        };
        let _ = governor
            .reserve(
                ResourceScope {
                    tenant_id: TenantId::new("tenant-shared-helper").unwrap(),
                    user_id: UserId::new("alice-shared-helper").unwrap(),
                    agent_id: None,
                    project_id: None,
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
                ResourceEstimate::default(),
            )
            .unwrap();
        accountant.pre_model_call(&context, &request).await.unwrap();

        // The user account now carries the $5 compiled default.
        let user_account = ResourceAccount::user(
            TenantId::new("tenant-shared-helper").unwrap(),
            UserId::new("alice-shared-helper").unwrap(),
        );
        let snapshot = governor
            .account_snapshot(&user_account)
            .unwrap()
            .expect("user account seeded");
        let limits = snapshot
            .limits
            .expect("seeding policy installed default limits");
        assert_eq!(
            limits.max_usd,
            Some(dec!(5.00)),
            "compiled default $5 cap must be installed on first touch"
        );
    }

    fn test_run_context(tenant: &str, user: &str) -> ironclaw_turns::run_profile::LoopRunContext {
        use ironclaw_host_api::ThreadId;
        use ironclaw_turns::{
            AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnActor, TurnId,
            TurnRunId, TurnScope,
            run_profile::{
                CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy,
                CheckpointSchemaId, ConcurrencyClass, ContextProfileId, LoopDriverId,
                LoopRunContext, ModelProfileId, PersonalContextPolicy,
                RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy,
                ResourceBudgetTier, RunClassId, RunProfileFingerprint, RuntimeProfileConstraints,
                SchedulingClass, SteeringPolicy,
            },
        };

        let scope = TurnScope::new(
            TenantId::new(tenant).unwrap(),
            None,
            None,
            ThreadId::new(format!("thread-{tenant}")).unwrap(),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("budget_helper_test").unwrap(),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(CheckpointSchemaId::new("budget_helper_chk").unwrap()),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("budget_helper").unwrap(),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: descriptor.clone(),
            checkpoint_schema_id: descriptor.checkpoint_schema_id.unwrap(),
            checkpoint_schema_version: descriptor.checkpoint_schema_version.unwrap(),
            model_profile_id: ModelProfileId::new("budget_helper_model").unwrap(),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new("budget_helper_caps")
                .unwrap(),
            context_profile_id: ContextProfileId::new("budget_helper_ctx").unwrap(),
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
                tier: ResourceBudgetTier::new("budget_helper_tier").unwrap(),
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
            resolution_fingerprint: RunProfileFingerprint::new("budget-helper-fp").unwrap(),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), profile)
            .with_actor(TurnActor::new(UserId::new(user).unwrap()))
    }
}
