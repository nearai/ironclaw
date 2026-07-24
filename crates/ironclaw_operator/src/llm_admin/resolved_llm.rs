use std::sync::Arc;

#[derive(Clone)]
pub struct ResolvedRebornLlm {
    provider_id: String,
    model: String,
    config: ironclaw_llm::LlmConfig,
    provider_factory: Option<RebornProviderFactory>,
}

/// Decorator over the config-built LLM provider.
pub type RebornProviderFactory = Arc<
    dyn Fn(Arc<dyn ironclaw_llm::LlmProvider>) -> Arc<dyn ironclaw_llm::LlmProvider> + Send + Sync,
>;

// `LlmProvider` is not `Debug`, so derive can't see through `provider_factory`.
impl std::fmt::Debug for ResolvedRebornLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedRebornLlm")
            .field("provider_id", &self.provider_id)
            .field("model", &self.model)
            .field("provider_factory", &self.provider_factory.is_some())
            .finish_non_exhaustive()
    }
}

impl ResolvedRebornLlm {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn config(&self) -> &ironclaw_llm::LlmConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut ironclaw_llm::LlmConfig {
        &mut self.config
    }

    pub fn into_config(self) -> ironclaw_llm::LlmConfig {
        self.config
    }

    /// Whether a caller-installed provider decorator will run at cold boot.
    pub fn has_provider_factory(&self) -> bool {
        self.provider_factory.is_some()
    }

    pub fn provider_factory(&self) -> Option<RebornProviderFactory> {
        self.provider_factory.clone()
    }

    /// Base URL of the backend `serve` actually boots with, when the
    /// backend has one. See [`ironclaw_llm::LlmConfig::active_base_url`].
    pub fn base_url(&self) -> Option<String> {
        self.config.active_base_url()
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
    /// it.
    pub fn with_provider_factory(mut self, factory: RebornProviderFactory) -> Self {
        self.provider_factory = Some(factory);
        self
    }

    /// Attach the LLM-trace-recording decorator when `IRONCLAW_RECORD_TRACE` is
    /// set in the environment; otherwise return the resolved LLM unchanged.
    pub fn with_env_trace_recording(self) -> Self {
        if !ironclaw_llm::RecordingLlm::env_recording_enabled() {
            return self;
        }
        let factory: RebornProviderFactory =
            Arc::new(
                |inner| match ironclaw_llm::RecordingLlm::from_env(Arc::clone(&inner)) {
                    Some(recorder) => recorder as Arc<dyn ironclaw_llm::LlmProvider>,
                    None => inner,
                },
            );
        self.with_provider_factory(factory)
    }
}
