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

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
#[cfg(any(test, feature = "test-support"))]
use ironclaw_loop_support::HostManagedModelGateway;
use ironclaw_loop_support::HostSkillContextSource;
use ironclaw_reborn::runtime::{
    DEFAULT_MAX_CONCURRENT_RUNS_PER_USER, DEFAULT_MAX_CONCURRENT_TRIGGER_RUNS,
    DEFAULT_TURN_RUNNER_WORKER_COUNT, ToolDisclosureMode,
};
use ironclaw_reborn_config::BudgetDefaults;
#[cfg(feature = "root-llm-provider")]
use ironclaw_reborn_config::RebornBootConfig;
use ironclaw_triggers::{TriggerId, TriggerPollerWorkerConfig};

use crate::input::RebornBuildInput;
use crate::observability::hooks::HooksActivationConfig;

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

/// Fire-time access request for a persisted trigger.
///
/// This is the host/composition-facing access check shape. Checks are exact:
/// `None` for `agent_id` or `project_id` means the trigger has no value for
/// that scope dimension, not that the checker should treat it as a wildcard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerFireAccessCheck {
    /// Tenant that owns the persisted trigger.
    pub tenant_id: TenantId,
    /// User that created the persisted trigger and whose access is evaluated
    /// again at fire time.
    pub creator_user_id: UserId,
    /// Optional agent scope stored on the trigger.
    pub agent_id: Option<AgentId>,
    /// Optional project scope stored on the trigger.
    pub project_id: Option<ProjectId>,
    /// Trigger being fired. Included so production access checks can audit or
    /// apply trigger-specific policy without changing this request shape.
    pub trigger_id: TriggerId,
    /// Deterministic fire slot being submitted. Included for audit and policy
    /// decisions that depend on scheduled fire identity.
    pub fire_slot: Timestamp,
}

/// Result of a fire-time trigger access check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerFireAccessDecision {
    /// The trigger creator is still authorized for the exact trigger scope.
    Allowed,
    /// The trigger creator is not authorized for the exact trigger scope.
    Denied { reason: String },
}

/// Error returned when the access backend cannot answer the request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TriggerFireAccessError {
    /// The backing access source was unavailable; trigger fire handling should
    /// treat this as retryable rather than a permanent denial.
    #[error("trigger fire access backend unavailable: {reason}")]
    Unavailable { reason: String },
}

/// Fire-time trigger access checker supplied by the composition root.
#[async_trait]
pub trait TriggerFireAccessChecker: Send + Sync {
    /// Check whether the persisted trigger creator may fire the trigger for
    /// the exact stored tenant/agent/project scope.
    async fn check_trigger_fire_access(
        &self,
        request: TriggerFireAccessCheck,
    ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError>;
}

#[cfg(feature = "root-llm-provider")]
#[derive(Clone)]
pub struct ResolvedRebornLlm {
    provider_id: String,
    model: String,
    pub(crate) config: ironclaw_llm::LlmConfig,
    /// Optional decorator applied to the provider the gateway builds from
    /// `config`. `config` is always the construction source (so it stays the
    /// single source of truth for `provider_id`/`model` and budget cost-table
    /// derivation); the factory only *wraps* the built provider — e.g. a
    /// benchmark harness layering token/reasoning instrumentation over it.
    /// When `None` the gateway uses the config-built provider as-is.
    pub(crate) provider_factory: Option<RebornProviderFactory>,
}

/// Decorator over the config-built LLM provider. See
/// [`ResolvedRebornLlm::with_provider_factory`].
#[cfg(feature = "root-llm-provider")]
pub type RebornProviderFactory = Arc<
    dyn Fn(Arc<dyn ironclaw_llm::LlmProvider>) -> Arc<dyn ironclaw_llm::LlmProvider> + Send + Sync,
>;

// `LlmProvider` is not `Debug`, so derive can't see through `provider_override`.
#[cfg(feature = "root-llm-provider")]
impl std::fmt::Debug for ResolvedRebornLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedRebornLlm")
            .field("provider_id", &self.provider_id)
            .field("model", &self.model)
            .field("provider_factory", &self.provider_factory.is_some())
            .finish_non_exhaustive()
    }
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
            provider_factory: None,
        }
    }

    /// Wrap the config-built provider with `factory` before the gateway drives
    /// it — e.g. to layer token/reasoning/cost instrumentation over the real
    /// provider.
    ///
    /// This is the instrumentation seam (feature-gated on `root-llm-provider`).
    /// The composition still constructs the provider from `config` and hands it
    /// to the factory, so `config` remains the single source of truth and the
    /// raw `ironclaw_llm::LlmProvider` substrate handle is never accepted
    /// wholesale through the facade — the caller only supplies a decorator over
    /// a provider the composition built. `build_llm_gateway` applies the factory
    /// and never re-exposes the provider.
    pub fn with_provider_factory(mut self, factory: RebornProviderFactory) -> Self {
        self.provider_factory = Some(factory);
        self
    }
}

/// Configuration for the turn-runner worker spawned by the runtime.
#[derive(Debug, Clone)]
pub struct TurnRunnerSettings {
    pub heartbeat_interval: Duration,
    pub poll_interval: Duration,
    /// Number of concurrent turn-runner slots (the scheduler semaphore permit
    /// count). `None` = unlimited — the scheduler is sized to
    /// `tokio::sync::Semaphore::MAX_PERMITS`, leaving the per-user / per-origin
    /// caps below as the only concurrency bound.
    pub worker_count: Option<std::num::NonZeroUsize>,
    /// Max runs in `TurnStatus::Running` per (tenant_id, owner user_id).
    /// `None` = unlimited. Owner-less / actor-fallback runs are never counted.
    pub max_concurrent_runs_per_user: Option<std::num::NonZeroU32>,
    /// Max runs in `TurnStatus::Running` for `ScheduledTrigger` origin.
    /// `None` = unlimited.
    pub max_concurrent_trigger_runs: Option<std::num::NonZeroU32>,
    /// Max runs in `TurnStatus::Running` for `Inbound` or `WebUi` origin.
    /// `None` = unlimited.
    pub max_concurrent_conversation_runs: Option<std::num::NonZeroU32>,
}

impl Default for TurnRunnerSettings {
    fn default() -> Self {
        Self {
            heartbeat_interval: DEFAULT_TURN_RUNNER_HEARTBEAT_INTERVAL,
            poll_interval: DEFAULT_TURN_RUNNER_POLL_INTERVAL,
            worker_count: Some(DEFAULT_TURN_RUNNER_WORKER_COUNT),
            max_concurrent_runs_per_user: Some(DEFAULT_MAX_CONCURRENT_RUNS_PER_USER),
            max_concurrent_trigger_runs: Some(DEFAULT_MAX_CONCURRENT_TRIGGER_RUNS),
            // `None` = conversations may use every slot not held by triggers.
            max_concurrent_conversation_runs: None,
        }
    }
}

impl TurnRunnerSettings {
    pub fn set_heartbeat_interval(mut self, heartbeat_interval: Duration) -> Self {
        self.heartbeat_interval = heartbeat_interval;
        self
    }

    pub fn set_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    pub fn set_worker_count(mut self, worker_count: std::num::NonZeroUsize) -> Self {
        self.worker_count = Some(worker_count);
        self
    }

    pub fn set_max_concurrent_runs_per_user(
        mut self,
        max_concurrent_runs_per_user: std::num::NonZeroU32,
    ) -> Self {
        self.max_concurrent_runs_per_user = Some(max_concurrent_runs_per_user);
        self
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

/// Configuration for the background Google OAuth credential keepalive worker.
///
/// The worker handles background keepalive refreshes (B2/B3): it periodically
/// refreshes Google OAuth accounts that are idle (by `updated_at`) to prevent
/// the 7-day refresh-token death window from expiring during periods of
/// inactivity.
///
/// The inline access-token expiry gate is controlled by the fixed
/// `DEFAULT_ACCESS_REFRESH_MARGIN` constant in
/// `product_auth_runtime_credentials.rs`; it is not configurable here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRefreshSettings {
    /// Whether the worker is enabled. Defaults to `false`; use
    /// `CredentialRefreshSettings::enabled()` to turn on.
    pub enabled: bool,
    /// How often the worker wakes and sweeps for idle accounts.
    ///
    /// Default: 6 hours.
    pub interval: Duration,
    /// How old (by `updated_at`) an account must be before it is considered
    /// idle and eligible for a proactive refresh.
    ///
    /// Default: 2 days — well under the 7-day refresh-token idle-death window,
    /// with headroom for downtime or deployment gaps.
    pub idle_threshold: Duration,
    /// Maximum random jitter applied once at worker startup before the first
    /// tick. Spreading startup jitter across the multi-process deployment
    /// prevents a thundering herd at first boot. The advisory-lock wrapper
    /// (A4) serializes concurrent refreshes, but jitter reduces unnecessary
    /// contention. Default: `Duration::ZERO`.
    pub startup_jitter_max: Duration,
    /// Maximum random jitter appended to each inter-tick sleep.
    /// Default: `Duration::ZERO`.
    pub tick_jitter_max: Duration,
    /// Maximum number of candidate accounts processed per tick. Bounds the
    /// work done in a single sweep to avoid a large initial backfill
    /// overloading the token endpoint.
    ///
    /// Default: 5.
    pub max_per_tick: usize,
}

impl Default for CredentialRefreshSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(6 * 3600),
            idle_threshold: Duration::from_secs(2 * 24 * 3600),
            startup_jitter_max: Duration::ZERO,
            tick_jitter_max: Duration::ZERO,
            max_per_tick: 5,
        }
    }
}

impl CredentialRefreshSettings {
    /// Return a settings value with the worker enabled and all other fields at
    /// their defaults.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            // 5-minute spread prevents fleet-wide sweep storms on simultaneous startup.
            startup_jitter_max: Duration::from_secs(300),
            ..Self::default()
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
    CreatorAccessRequired,
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
            authorizer: TriggerPollerAuthorizerConfig::CreatorAccessRequired,
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
    pub tool_disclosure: Option<ToolDisclosureMode>,
    pub trigger_poller: TriggerPollerSettings,
    pub credential_refresh: CredentialRefreshSettings,
    pub trigger_fire_access_checker: Option<Arc<dyn TriggerFireAccessChecker>>,
    pub poll: PollSettings,
    pub identity: RebornRuntimeIdentity,
    /// Optional project scope for runtime-owned thread I/O. Channel adapters
    /// that stamp a project onto inbound turns must set the same project here,
    /// otherwise the loop host rejects the run before model execution.
    pub default_project_id: Option<ProjectId>,
    pub regex_skill_activation_enabled: bool,
    pub skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    /// Hook-framework activation knobs. Default OFF. Callers resolve
    /// environment or config into this typed value once at the edge.
    pub hooks: HooksActivationConfig,
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
    /// Observer that receives each capability/tool invocation + result during a
    /// run, so a downstream caller can reconstruct the full step-by-step
    /// trajectory (the sealed runtime otherwise exposes only the final reply).
    pub trajectory_observer: Option<Arc<dyn crate::RebornTrajectoryObserver>>,
    /// Mints the one-time API bearer returned when an admin creates a user. The
    /// serve layer supplies a session-store-backed minter; when unset, the admin
    /// user-management surface stays unwired (create reports unavailable).
    #[cfg(feature = "webui-v2-beta")]
    pub admin_api_token_minter: Option<Arc<dyn crate::AdminApiTokenMinter>>,
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
            tool_disclosure: None,
            trigger_poller: TriggerPollerSettings::default(),
            credential_refresh: CredentialRefreshSettings::default(),
            trigger_fire_access_checker: None,
            poll: PollSettings::default(),
            identity: RebornRuntimeIdentity::default(),
            default_project_id: None,
            regex_skill_activation_enabled: true,
            skill_context_source: None,
            hooks: HooksActivationConfig::default(),
            budget_defaults: None,
            budget_event_observer: None,
            trajectory_observer: None,
            #[cfg(feature = "webui-v2-beta")]
            admin_api_token_minter: None,
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

    /// Install the admin API-token minter used when an admin creates a user.
    /// The serve layer builds a session-store-backed minter; without it the
    /// admin user-management surface stays unwired.
    #[cfg(feature = "webui-v2-beta")]
    pub fn with_admin_api_token_minter(
        mut self,
        minter: Arc<dyn crate::AdminApiTokenMinter>,
    ) -> Self {
        self.admin_api_token_minter = Some(minter);
        self
    }

    /// Install a trajectory observer that receives each capability/tool call +
    /// result during a run (for downstream step-by-step trajectory capture).
    ///
    /// The observer receives a **bounded safe preview** of arguments/results
    /// (long strings truncated, large arrays capped — see
    /// [`crate::observability::trajectory_observer`]), keeping a downstream logs/UI/telemetry
    /// sink within the same boundary the model-visible display path enforces.
    /// A consumer that needs the unbounded raw payloads (and owns its own
    /// redaction/access control) must opt in via
    /// [`Self::with_raw_trajectory_observer`].
    ///
    /// **Local-dev / bench only.** The observer is wired through the local-dev
    /// capability path; it has no effect on production-profile runtimes, which
    /// have no capability/result hook to forward to. `build_reborn_runtime`
    /// fails fast with `InvalidArgument` if an observer is supplied for a
    /// profile without a local runtime, rather than silently dropping it.
    pub fn with_trajectory_observer(
        mut self,
        observer: Arc<dyn crate::RebornTrajectoryObserver>,
    ) -> Self {
        self.trajectory_observer = Some(
            crate::observability::trajectory_observer::SafePreviewTrajectoryObserver::wrap(
                observer,
            ),
        );
        self
    }

    /// Install a trajectory observer that receives the **raw, unbounded**
    /// capability arguments and results — no safe-preview truncation.
    ///
    /// Capability results can contain file contents, command output, or
    /// credentials, so this bypasses the truncation boundary that
    /// [`Self::with_trajectory_observer`] applies by default. Use it only for a
    /// trusted, in-process consumer that needs the verbatim trajectory (e.g. a
    /// benchmark harness rendering exact tool I/O) and owns its own redaction
    /// and access control for whatever sink it projects to.
    ///
    /// **Local-dev / bench only**, with the same fail-fast contract as
    /// [`Self::with_trajectory_observer`].
    pub fn with_raw_trajectory_observer(
        mut self,
        observer: Arc<dyn crate::RebornTrajectoryObserver>,
    ) -> Self {
        self.trajectory_observer = Some(observer);
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

    pub fn with_tool_disclosure(mut self, mode: ToolDisclosureMode) -> Self {
        self.tool_disclosure = Some(mode);
        self
    }

    pub fn with_trigger_poller_settings(mut self, trigger_poller: TriggerPollerSettings) -> Self {
        self.trigger_poller = trigger_poller;
        self
    }

    pub fn with_credential_refresh_settings(
        mut self,
        credential_refresh: CredentialRefreshSettings,
    ) -> Self {
        self.credential_refresh = credential_refresh;
        self
    }

    pub fn with_trigger_fire_access_checker(
        mut self,
        checker: Arc<dyn TriggerFireAccessChecker>,
    ) -> Self {
        self.trigger_fire_access_checker = Some(checker);
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

    pub fn with_default_project_id(mut self, project_id: ProjectId) -> Self {
        self.default_project_id = Some(project_id);
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

    pub fn with_hooks_config(mut self, hooks: HooksActivationConfig) -> Self {
        self.hooks = hooks;
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
