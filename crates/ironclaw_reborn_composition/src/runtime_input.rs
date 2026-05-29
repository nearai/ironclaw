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

/// Full input for `build_reborn_runtime` — substrate config plus the extras
/// needed to assemble a runnable Reborn agent.
#[derive(Default)]
pub struct RebornRuntimeInput {
    pub services: Option<RebornBuildInput>,
    #[cfg(feature = "root-llm-provider")]
    pub llm: Option<ResolvedRebornLlm>,
    pub runner: TurnRunnerSettings,
    pub poll: PollSettings,
    pub identity: RebornRuntimeIdentity,
    pub regex_skill_activation_enabled: bool,
    pub skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) model_gateway_override: Option<Arc<dyn HostManagedModelGateway>>,
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
            runner: TurnRunnerSettings::default(),
            poll: PollSettings::default(),
            identity: RebornRuntimeIdentity::default(),
            regex_skill_activation_enabled: true,
            skill_context_source: None,
            #[cfg(any(test, feature = "test-support"))]
            model_gateway_override: None,
        }
    }

    #[cfg(feature = "root-llm-provider")]
    pub fn with_resolved_llm(mut self, llm: ResolvedRebornLlm) -> Self {
        self.llm = Some(llm);
        self
    }

    pub fn with_runner_settings(mut self, runner: TurnRunnerSettings) -> Self {
        self.runner = runner;
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

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
        self
    }

    pub fn grants_trusted_laptop_access(&self) -> bool {
        self.services
            .as_ref()
            .is_some_and(|services| services.grants_trusted_laptop_access())
    }

    /// Inject a custom `HostManagedModelGateway` in place of whatever the
    /// build flow would otherwise derive from `[llm]` config. Exposed for
    /// the crate's own tests plus downstream integration tests that need
    /// to drive `build_reborn_runtime` against a recording / replay gateway
    /// without standing up a live provider.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_model_gateway_override(
        mut self,
        gateway: Arc<dyn HostManagedModelGateway>,
    ) -> Self {
        self.model_gateway_override = Some(gateway);
        self
    }
}
