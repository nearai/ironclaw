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
//!
//! The CLI builds this struct from env vars / config; it does not call into
//! `ironclaw_reborn` or `ironclaw_llm` directly.

#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use ironclaw_loop_support::HostManagedModelGateway;

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
            request_timeout_secs: 120,
            extra_headers: Vec::new(),
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
            heartbeat_interval: Duration::from_secs(10),
            poll_interval: Duration::from_secs(2),
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
    pub llm: Option<RebornLlmConfig>,
    pub runner: TurnRunnerSettings,
    pub poll: PollSettings,
    pub identity: RebornRuntimeIdentity,
    #[cfg(test)]
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
            #[cfg(test)]
            model_gateway_override: None,
        }
    }

    #[cfg(feature = "root-llm-provider")]
    pub fn with_llm(mut self, llm: RebornLlmConfig) -> Self {
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

    #[cfg(test)]
    pub(crate) fn with_model_gateway_override(
        mut self,
        gateway: Arc<dyn HostManagedModelGateway>,
    ) -> Self {
        self.model_gateway_override = Some(gateway);
        self
    }
}
