//! Input DTO for the assembled Reborn runtime (`build_reborn_runtime`).
//!
//! `RebornRuntimeInput` extends `RebornBuildInput` (which is substrate-only)
//! with the additional knobs needed to assemble a runnable agent:
//!
//! - **LLM configuration** (optional, behind the `root-llm-provider` feature).
//!   Used by the composition root to construct an `LlmProviderModelGateway`
//!   that satisfies the loop-support `HostManagedModelGateway` contract.
//! - **Turn-runner configuration** — poll/heartbeat intervals for the worker
//!   loop.
//! - **Completion polling configuration** — interval/timeout policy for
//!   waiting on submitted turns to finish.
//! - **Runtime identity** — tenant/agent and source/reply binding identifiers
//!   supplied by the caller so this composition root stays channel-agnostic.
//! - **Skill context source** — optional caller-supplied override for
//!   model-visible skill instructions. When absent, supported runtime profiles
//!   wire the first-party filesystem skill source from scoped Reborn skill
//!   roots.
//!
//! The CLI builds this struct from env vars / config; it does not call into
//! `ironclaw_reborn` or `ironclaw_llm` directly.

use std::sync::Arc;
use std::time::Duration;

#[cfg(any(test, feature = "test-support"))]
use ironclaw_loop_support::HostManagedModelGateway;
use ironclaw_loop_support::HostSkillContextSource;
use ironclaw_reborn_config::BudgetDefaults;
#[cfg(feature = "root-llm-provider")]
use ironclaw_reborn_config::RebornBootConfig;
use ironclaw_triggers::TriggerPollerWorkerConfig;

use crate::input::RebornBuildInput;

/// Caller-owned identity for an assembled Reborn runtime.
///
/// The CLI uses the `reborn-cli` values, but future ingress adapters should
/// pass their own tenant/agent and binding identifiers instead of inheriting
/// CLI-specific labels from the composition root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornRuntimeIdentity {
    pub tenant_id: String,
    pub agent_id: String,
    pub source_binding_id: String,
    pub reply_target_binding_id: String,
}

impl RebornRuntimeIdentity {
    pub fn reborn_cli() -> Self {
        Self {
            tenant_id: "reborn-cli".to_string(),
            agent_id: "reborn-cli-agent".to_string(),
            source_binding_id: "reborn-cli".to_string(),
            reply_target_binding_id: "reborn-cli".to_string(),
        }
    }
}

impl Default for RebornRuntimeIdentity {
    fn default() -> Self {
        Self::reborn_cli()
    }
}

pub const DEFAULT_TURN_RUNNER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_TURN_RUNNER_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[cfg(feature = "root-llm-provider")]
#[derive(Debug, Clone)]
pub struct ResolvedRebornLlm {
    provider_id: String,
    model: String,
    pub(crate) config: ironclaw_llm::LlmConfig,
}

#[cfg(feature = "root-llm-provider")]
impl ResolvedRebornLlm {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn from_llm_config(config: ironclaw_llm::LlmConfig) -> Self {
        Self {
            provider_id: config.active_provider_id(),
            model: config.active_model_name(),
            config,
        }
    }
}

/// Configuration for the turn-runner worker spawned by the runtime.
#[derive(Debug, Clone)]
pub struct TurnRunnerSettings {
    pub heartbeat_interval: Duration,
    pub poll_interval: Duration,
}

impl Default for TurnRunnerSettings {
    fn default() -> Self {
        Self {
            heartbeat_interval: DEFAULT_TURN_RUNNER_HEARTBEAT_INTERVAL,
            poll_interval: DEFAULT_TURN_RUNNER_POLL_INTERVAL,
        }
    }
}

/// Completion polling policy for `RebornRuntime::send_user_message`.
#[derive(Debug, Clone)]
pub struct PollSettings {
    pub interval: Duration,
    pub max_total: Duration,
}

impl Default for PollSettings {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(100),
            max_total: Duration::from_secs(180),
        }
    }
}

/// Configuration for the composition-owned scheduled-trigger poller.
///
/// This is intentionally separate from [`PollSettings`], which controls
/// caller-side waiting for an already submitted turn. The trigger poller is a
/// background worker that scans due trigger records and submits trusted inbound
/// turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPollerSettings {
    pub enabled: bool,
    pub worker: TriggerPollerWorkerConfig,
    pub startup_jitter_max: Duration,
    pub tick_jitter_max: Duration,
    pub(crate) authorizer: TriggerPollerAuthorizerConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TriggerPollerAuthorizerConfig {
    CreatorMembershipRequired,
    #[cfg(any(test, feature = "test-support"))]
    TenantScopedPlaceholderForTest,
}

impl Default for TriggerPollerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            worker: TriggerPollerWorkerConfig::default(),
            startup_jitter_max: Duration::ZERO,
            tick_jitter_max: Duration::ZERO,
            authorizer: TriggerPollerAuthorizerConfig::CreatorMembershipRequired,
        }
    }
}

impl TriggerPollerSettings {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    pub fn with_worker_config(mut self, worker: TriggerPollerWorkerConfig) -> Self {
        self.worker = worker;
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled_with_tenant_scoped_authorizer_for_test() -> Self {
        Self::enabled().with_tenant_scoped_authorizer_for_test()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_tenant_scoped_authorizer_for_test(mut self) -> Self {
        self.authorizer = TriggerPollerAuthorizerConfig::TenantScopedPlaceholderForTest;
        self
    }
}

/// Full input for `build_reborn_runtime` — substrate config plus the extras
/// needed to assemble a runnable Reborn agent.
#[derive(Default)]
pub struct RebornRuntimeInput {
    pub services: Option<RebornBuildInput>,
    #[cfg(feature = "root-llm-provider")]
    pub llm: Option<ResolvedRebornLlm>,
    /// Operator boot config. When present (and `root-llm-provider` is on), the
    /// WebUI facade composes the LLM-config settings service from it so the
    /// settings surface can read/write `providers.json` + `config.toml`.
    #[cfg(feature = "root-llm-provider")]
    pub boot: Option<RebornBootConfig>,
    pub runner: TurnRunnerSettings,
    pub trigger_poller: TriggerPollerSettings,
    pub poll: PollSettings,
    pub identity: RebornRuntimeIdentity,
    pub regex_skill_activation_enabled: bool,
    pub skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    /// Pre-resolved budget defaults to seed the model-budget accountant.
    /// The caller owns the config-layer precedence (compiled -> section
    /// -> env) and must call [`BudgetDefaults::validate`] before
    /// supplying. When unset, `build_reborn_runtime` falls back to
    /// `BudgetDefaults::compiled_defaults().with_env()` + validate so
    /// existing call sites keep working; new call sites should provide
    /// a resolved value to avoid the runtime reading process env
    /// (review feedback Thermo-Nuclear #1).
    pub budget_defaults: Option<BudgetDefaults>,
    /// Observer that receives every `BudgetEvent` emitted by the model
    /// budget accountant / resource governor. When unset, the runtime
    /// installs [`TracingBudgetEventObserver`](crate::TracingBudgetEventObserver)
    /// so events still reach the tracing pipeline; production owners
    /// supply their own observer (SSE projection, WS fan-out,
    /// telemetry export) here.
    pub budget_event_observer: Option<Arc<dyn crate::BudgetEventObserver>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) model_gateway_override: Option<Arc<dyn HostManagedModelGateway>>,
    /// Cost table to pair with the model-gateway override. Without this,
    /// tests that use `with_test_model_gateway` would lose the accountant
    /// entirely (the LLM-resolved cost table comes from
    /// `LlmModelProfilePolicy::build_cost_table()` which the test
    /// override skips).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) model_cost_table_override: Option<Arc<dyn ironclaw_loop_support::ModelCostTable>>,
}

impl RebornRuntimeInput {
    /// Start from a substrate build input. The substrate input must be
    /// provided — there is no in-memory-only fallback at this layer because
    /// the substrate decisions (local-dev root, libsql handle, etc.) belong
    /// to the caller, not the assembly.
    pub fn from_services(services: RebornBuildInput) -> Self {
        Self {
            services: Some(services),
            #[cfg(feature = "root-llm-provider")]
            llm: None,
            #[cfg(feature = "root-llm-provider")]
            boot: None,
            runner: TurnRunnerSettings::default(),
            trigger_poller: TriggerPollerSettings::default(),
            poll: PollSettings::default(),
            identity: RebornRuntimeIdentity::default(),
            regex_skill_activation_enabled: true,
            skill_context_source: None,
            budget_defaults: None,
            budget_event_observer: None,
            #[cfg(any(test, feature = "test-support"))]
            model_gateway_override: None,
            #[cfg(any(test, feature = "test-support"))]
            model_cost_table_override: None,
        }
    }

    /// Supply pre-resolved budget defaults. The caller is responsible
    /// for applying the desired config-layer precedence (compiled,
    /// TOML, env) and calling [`BudgetDefaults::validate`] before
    /// passing. Without this, `build_reborn_runtime` falls back to
    /// `compiled_defaults().with_env()` + validate (review feedback
    /// Thermo-Nuclear #1: budget defaults belong to the composition
    /// root, not a wiring helper).
    pub fn with_budget_defaults(mut self, defaults: BudgetDefaults) -> Self {
        self.budget_defaults = Some(defaults);
        self
    }

    /// Install a custom observer for the model budget event stream.
    /// Production callers wire this to project events onto SSE / WS /
    /// telemetry; without it, the runtime installs the tracing-only
    /// observer so events still surface in structured logs.
    pub fn with_budget_event_observer(
        mut self,
        observer: Arc<dyn crate::BudgetEventObserver>,
    ) -> Self {
        self.budget_event_observer = Some(observer);
        self
    }

    #[cfg(feature = "root-llm-provider")]
    pub fn with_resolved_llm(mut self, llm: ResolvedRebornLlm) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Supply the operator boot config so the WebUI facade can compose the
    /// LLM-config settings service.
    #[cfg(feature = "root-llm-provider")]
    pub fn with_boot_config(mut self, boot: RebornBootConfig) -> Self {
        self.boot = Some(boot);
        self
    }

    pub fn with_runner_settings(mut self, runner: TurnRunnerSettings) -> Self {
        self.runner = runner;
        self
    }

    pub fn with_trigger_poller_settings(mut self, trigger_poller: TriggerPollerSettings) -> Self {
        self.trigger_poller = trigger_poller;
        self
    }

    pub fn with_poll_settings(mut self, poll: PollSettings) -> Self {
        self.poll = poll;
        self
    }

    pub fn with_identity(mut self, identity: RebornRuntimeIdentity) -> Self {
        self.identity = identity;
        self
    }

    pub fn with_regex_skill_activation_enabled(mut self, enabled: bool) -> Self {
        self.regex_skill_activation_enabled = enabled;
        self
    }

    /// Override the runtime owner id after the input (and its host-access
    /// disclosure gate) has been built. The WebChat v2 serve path uses this to
    /// align the runtime owner with the authenticated WebUI user. No-op when
    /// the services input is absent.
    pub fn with_owner_id(mut self, owner_id: impl Into<String>) -> Self {
        self.services = self
            .services
            .map(|services| services.with_owner_id(owner_id));
        self
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
        self
    }

    pub fn grants_trusted_laptop_access(&self) -> bool {
        self.services
            .as_ref()
            .is_some_and(|services| services.grants_trusted_laptop_access())
    }

    /// Test-only hook: drive `build_reborn_runtime` with a stub
    /// `HostManagedModelGateway` (e.g. [`crate::test_support::BudgetTestGateway`])
    /// instead of the LLM-backed gateway. Gated on `cfg(any(test,
    /// feature = "test-support"))` so it is available to this crate's
    /// own tests and to downstream integration tests that opt in via
    /// the `test-support` feature.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_model_gateway_override(
        mut self,
        gateway: Arc<dyn HostManagedModelGateway>,
    ) -> Self {
        self.model_gateway_override = Some(gateway);
        self
    }

    /// Test-only hook: pair the model gateway override with a custom
    /// cost table. Without this, gateway overrides produce no
    /// accountant and budget tests cannot assert ledger state — the
    /// LLM-derived cost table comes from
    /// `LlmModelProfilePolicy::build_cost_table()` which the test
    /// override skips.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_model_cost_table_override(
        mut self,
        cost_table: Arc<dyn ironclaw_loop_support::ModelCostTable>,
    ) -> Self {
        self.model_cost_table_override = Some(cost_table);
        self
    }
}
