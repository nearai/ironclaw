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

/// Configuration for the host-managed LLM model gateway wired into the
/// composed Reborn runtime.
///
/// Only available when this crate is built with the `root-llm-provider`
/// feature. Mirrors `ironclaw_llm::RegistryProviderConfig` but stays in
/// composition-owned types so callers (the CLI) never name `ironclaw_llm`
/// directly.
#[cfg(feature = "root-llm-provider")]
pub const DEFAULT_LLM_REQUEST_TIMEOUT_SECS: u64 = 120;

pub const DEFAULT_TURN_RUNNER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_TURN_RUNNER_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[cfg(feature = "root-llm-provider")]
#[derive(Debug, Clone)]
pub struct RebornLlmConfig {
    /// Provider id (e.g. `"openai"`, `"anthropic"`, `"ollama"`).
    pub provider_id: String,
    /// Model id passed to the provider (e.g. `"gpt-4o-mini"`).
    pub model: String,
    /// Provider API base URL.
    pub base_url: String,
    /// API key, if the provider requires one. `None` for keyless providers
    /// like Ollama.
    pub api_key: Option<secrecy::SecretString>,
    /// API protocol identifier — maps onto
    /// `ironclaw_llm::ProviderProtocol`. Canonical accepted values:
    /// `"open_ai_completions"`, `"anthropic"`, `"ollama"`, `"deep_seek"`,
    /// `"gemini"`, `"open_router"`, `"github_copilot"`.
    /// Legacy aliases `"openai"`, `"openai_completions"`, `"deepseek"`,
    /// and `"openrouter"` are also accepted.
    pub protocol: String,
    /// Request timeout in seconds passed to the underlying HTTP client.
    pub request_timeout_secs: u64,
    /// Extra HTTP headers injected on every request.
    pub extra_headers: Vec<(String, String)>,
}

#[cfg(feature = "root-llm-provider")]
impl RebornLlmConfig {
    /// Convenience constructor for the common OpenAI Chat Completions case.
    pub fn openai_compat(
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: secrecy::SecretString,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            model: model.into(),
            base_url: base_url.into(),
            api_key: Some(api_key),
            protocol: "open_ai_completions".to_string(),
            request_timeout_secs: DEFAULT_LLM_REQUEST_TIMEOUT_SECS,
            extra_headers: Vec::new(),
        }
    }
}

#[cfg(feature = "root-llm-provider")]
#[derive(Debug, Clone)]
pub struct ResolvedRebornLlm {
    provider_id: String,
    model: String,
    pub(crate) source: ResolvedRebornLlmSource,
}

#[cfg(feature = "root-llm-provider")]
#[derive(Debug, Clone)]
pub(crate) enum ResolvedRebornLlmSource {
    Catalog(RebornLlmConfig),
    RegistryProvider {
        config: ironclaw_llm::RegistryProviderConfig,
        request_timeout_secs: u64,
    },
}

#[cfg(feature = "root-llm-provider")]
impl ResolvedRebornLlm {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn from_catalog(config: RebornLlmConfig) -> Self {
        Self {
            provider_id: config.provider_id.clone(),
            model: config.model.clone(),
            source: ResolvedRebornLlmSource::Catalog(config),
        }
    }

    pub(crate) fn from_registry_provider(
        config: ironclaw_llm::RegistryProviderConfig,
        request_timeout_secs: u64,
    ) -> Self {
        Self {
            provider_id: config.provider_id.clone(),
            model: config.model.clone(),
            source: ResolvedRebornLlmSource::RegistryProvider {
                config,
                request_timeout_secs,
            },
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
    pub skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
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
            runner: TurnRunnerSettings::default(),
            poll: PollSettings::default(),
            identity: RebornRuntimeIdentity::default(),
            skill_context_source: None,
            #[cfg(any(test, feature = "test-support"))]
            model_gateway_override: None,
            #[cfg(any(test, feature = "test-support"))]
            model_cost_table_override: None,
        }
    }

    #[cfg(feature = "root-llm-provider")]
    pub fn with_llm(mut self, llm: RebornLlmConfig) -> Self {
        self.llm = Some(ResolvedRebornLlm::from_catalog(llm));
        self
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

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
        self
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
