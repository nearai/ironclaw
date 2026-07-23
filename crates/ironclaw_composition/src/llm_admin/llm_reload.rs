use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_config::{IronClawBootConfig, IronClawConfigFile};

use crate::LlmKeyStore;
use crate::llm_admin::llm_catalog::{
    IronClawLlmCatalogError, apply_stored_api_key, resolve_ironclaw_runtime_llm,
    resolve_llm_selection_allow_missing_key,
};
use crate::llm_admin::llm_config_service::LlmReloadTrigger;
use crate::runtime_input::ResolvedIronClawLlm;

/// Live-reload adapter wired by the runtime. Re-resolves the LLM config from
/// `config.toml` + `providers.json` + the stored key, then hot-swaps the
/// running provider's inner backend via the `ironclaw_llm` reload handle.
pub(crate) struct IronClawLlmReloadAdapter {
    boot: IronClawBootConfig,
    reload_handle: Arc<ironclaw_llm::LlmReloadHandle>,
    session: Arc<ironclaw_llm::SessionManager>,
    keys: LlmKeyStore,
}

impl IronClawLlmReloadAdapter {
    pub(crate) fn new(
        boot: IronClawBootConfig,
        reload_handle: Arc<ironclaw_llm::LlmReloadHandle>,
        session: Arc<ironclaw_llm::SessionManager>,
        keys: LlmKeyStore,
    ) -> Self {
        Self {
            boot,
            reload_handle,
            session,
            keys,
        }
    }

    /// Resolve the effective LLM selection, tolerating a required-but-unset
    /// API key env var when a stored key already exists for that provider.
    ///
    /// Without this, a provider selected purely through `config.toml` with a
    /// key that only lives in the encrypted secret store (never written to
    /// an env var — the onboarding menu's stored-key path) would fail closed
    /// here with `ApiKeyEnvUnset` before this adapter ever reaches the
    /// stored-key lookup below, leaving the placeholder gateway wired
    /// forever. Mirrors `resolve_ironclaw_runtime_llm_with_stored_key_fallback`
    /// in the CLI's `serve` boot path — this adapter is the *other* caller of
    /// the same resolution (live settings-save reload and boot-time reload),
    /// so it needs the same tolerance. Only `ApiKeyEnvUnset` is treated
    /// specially, and only when a stored key genuinely exists; every other
    /// failure (including `ApiKeyEnvUnset` with nothing stored) surfaces
    /// unchanged.
    async fn resolve_effective_llm(
        &self,
        config_file: Option<&IronClawConfigFile>,
    ) -> Result<Option<ResolvedIronClawLlm>, String> {
        let error = match resolve_ironclaw_runtime_llm(&self.boot, config_file) {
            Ok(resolved) => return Ok(resolved),
            Err(error) => error,
        };
        let IronClawLlmCatalogError::ApiKeyEnvUnset { ref provider, .. } = error else {
            return Err(error.to_string());
        };
        let Some(selection) = config_file.and_then(|file| file.default_llm_slot()) else {
            return Err(error.to_string());
        };
        if !self
            .keys
            .exists(provider)
            .await
            .map_err(|store_error| store_error.to_string())?
        {
            return Err(error.to_string());
        }
        resolve_llm_selection_allow_missing_key(
            selection,
            Some(self.boot.home().providers_file_path().as_path()),
        )
        .map(ResolvedIronClawLlm::from_llm_config)
        .map(Some)
        .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl LlmReloadTrigger for IronClawLlmReloadAdapter {
    async fn reload(&self) -> Result<(), String> {
        let config_file = IronClawConfigFile::load(&self.boot.home().config_file_path())
            .map_err(|error| error.to_string())?;
        let Some(resolved) = self.resolve_effective_llm(config_file.as_ref()).await? else {
            // No provider selected yet, so there is nothing to swap.
            return Ok(());
        };
        let provider_id = resolved.provider_id().to_string();
        let mut config = resolved.config;
        let key_applied = match self
            .keys
            .read(&provider_id)
            .await
            .map_err(|error| error.to_string())?
        {
            Some(stored) => {
                apply_stored_api_key(&mut config, stored);
                true
            }
            None => false,
        };
        let result = self
            .reload_handle
            .reload(&config, Arc::clone(&self.session))
            .await
            .map_err(|error| error.to_string());
        // Never log key material — only provider id and whether a stored key
        // was applied.
        tracing::debug!(
            provider_id = %provider_id,
            key_applied,
            succeeded = result.is_ok(),
            "LLM reload applied to the live provider"
        );
        result
    }
}
