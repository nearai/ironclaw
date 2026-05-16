//! Input DTO for the assembled Reborn runtime (`build_reborn_runtime`).
//!
//! `RebornRuntimeInput` is the **versioned**, forward-compatible contract
//! the composition root takes. It bundles together five independent
//! concerns:
//!
//! | Field | What | Today | After epic #3036 |
//! |-------|------|-------|------------------|
//! | `api_version` | schema stamp | `V1` mandatory | major bump = `try_from` migration |
//! | `substrate` | storage + runtime services bundle | drives `build_reborn_services` | unchanged |
//! | `identity` | tenant / agent / owner-user scope | hardcoded via `cli_default` from CLI | sourced from `RebornTenantRepo` |
//! | `policy` | DeploymentMode + RuntimeProfile + ApprovalPolicy | recorded; not yet enforced in loop | resolved through `RuntimePolicyRepo` per request |
//! | `drivers` | which loop drivers to register | text-only only | operator-picked via TOML; planned-driver wires in |
//! | `harness` | active use-case harness for new conversations | no-op + warning when `Some` | drives `HarnessActivationService` + `build_instruction_bundle` overlay + `visible_capabilities` filter |
//! | `llm` (feat) | host-managed model gateway provider | env-resolved by CLI | sourced from `ProviderRepo` per blueprint apply |
//! | `runner` | worker timing knobs | direct | direct |
//!
//! Caller-side: the CLI sets `RebornIdentityConfig::cli_default()`,
//! `RebornPolicyConfig::cli_default()`, `RebornDriverConfig::text_only_only()`,
//! and `harness = None`. Once epic #3036 ships, those values flow from a
//! parsed-and-applied blueprint + admin-scoped activation calls instead.

mod api_version;
mod drivers;
mod harness;
mod identity;
mod policy;

pub use api_version::{RebornRuntimeApiVersion, RebornRuntimeApiVersionError};
pub use drivers::{RebornDriverChoice, RebornDriverConfig};
pub use harness::{RebornHarnessId, RebornHarnessIdError, RebornHarnessSelection};
pub use identity::{ConversationIdentity, RebornIdentityConfig};
pub use policy::RebornPolicyConfig;

use std::time::Duration;

use crate::input::RebornBuildInput;

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
    /// `ironclaw_llm::ProviderProtocol`. Accepted values:
    /// `"openai_completions"`, `"anthropic"`, `"ollama"`, `"deepseek"`,
    /// `"gemini"`, `"openrouter"`, `"github_copilot"`.
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
            protocol: "openai_completions".to_string(),
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

/// Full input for `build_reborn_runtime` — substrate config plus the
/// independent layers that the composition root assembles into a running
/// agent.
///
/// Construct via [`RebornRuntimeInput::from_services`] and build up with
/// the `with_*` chain.
pub struct RebornRuntimeInput {
    pub api_version: RebornRuntimeApiVersion,
    pub services: Option<RebornBuildInput>,
    pub identity: RebornIdentityConfig,
    pub policy: RebornPolicyConfig,
    pub drivers: RebornDriverConfig,
    pub harness: Option<RebornHarnessSelection>,
    #[cfg(feature = "root-llm-provider")]
    pub llm: Option<RebornLlmConfig>,
    pub runner: TurnRunnerSettings,
}

impl RebornRuntimeInput {
    /// Start from a substrate build input. The substrate input must be
    /// provided — there is no in-memory-only fallback at this layer
    /// because the substrate decisions (local-dev root, libsql handle,
    /// etc.) belong to the caller, not the assembly.
    ///
    /// All other fields are populated with CLI-shaped defaults. Override
    /// via the `with_*` chain.
    pub fn from_services(services: RebornBuildInput) -> Self {
        // CLI default identity is allowed to fail validation in principle
        // (the strings are validated by host-api). The strings used here
        // are static and known-valid; if validation ever rejects them
        // that's a workspace-wide change we want to know about loudly.
        let identity = RebornIdentityConfig::cli_default()
            .expect("cli_default identity strings must satisfy host-api id validation");
        Self {
            api_version: RebornRuntimeApiVersion::current(),
            services: Some(services),
            identity,
            policy: RebornPolicyConfig::cli_default(),
            drivers: RebornDriverConfig::text_only_only(),
            harness: None,
            #[cfg(feature = "root-llm-provider")]
            llm: None,
            runner: TurnRunnerSettings::default(),
        }
    }

    pub fn with_api_version(mut self, version: RebornRuntimeApiVersion) -> Self {
        self.api_version = version;
        self
    }

    pub fn with_identity(mut self, identity: RebornIdentityConfig) -> Self {
        self.identity = identity;
        self
    }

    pub fn with_policy(mut self, policy: RebornPolicyConfig) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_drivers(mut self, drivers: RebornDriverConfig) -> Self {
        self.drivers = drivers;
        self
    }

    pub fn with_harness(mut self, harness: RebornHarnessSelection) -> Self {
        self.harness = Some(harness);
        self
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
}
