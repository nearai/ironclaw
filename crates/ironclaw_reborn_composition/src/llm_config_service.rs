//! Composition-side implementation of the WebChat v2 LLM-config port.
//!
//! Ties together the read/set-active surface ([`RebornProviderAdmin`]), the
//! custom-provider overlay writer ([`ProviderRepo`]), the operator-scoped key
//! store ([`LlmKeyStore`]), and the live provider-reload seam
//! ([`LlmReloadTrigger`]). Everything the webui2 Inference tab needs lands here;
//! the product facade stays a thin, sanitized pass-through.
//!
//! Persistence is operator-wide and split across three surfaces, mirroring how
//! reborn already resolves an LLM at boot:
//! - custom provider definitions  → `$IRONCLAW_REBORN_HOME/providers.json`
//! - active provider + model      → `config.toml [llm.default]`
//! - API-key **values**           → scoped secret store (never the file)
//!
//! After a successful write the running provider's inner backend is hot-swapped
//! via the reload trigger. The on-disk files are the source of truth: if reload
//! fails the change is still persisted and applies on the next restart, so the
//! operator is never left with a silently-dropped edit (the failure is logged).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_llm::registry::{ProviderDefinition, ProviderProtocol, ProviderRegistry};
use ironclaw_product_workflow::{
    LlmActiveSelection, LlmConfigService, LlmConfigServiceError, LlmConfigSnapshot,
    LlmModelsResult, LlmProbeRequest, LlmProbeResult, LlmProviderView, SetActiveLlmRequest,
    UpsertLlmProviderRequest, WebUiAuthenticatedCaller,
};
use ironclaw_reborn_config::{LlmSlotSelection, RebornBootConfig, RebornConfigFile};
use secrecy::{ExposeSecret as _, SecretString};

use crate::llm_catalog::{
    apply_stored_api_key, resolve_against_registry, resolve_reborn_runtime_llm,
};
use crate::{LlmKeyStore, ProviderRepo, RebornProviderAdmin};

/// Live-reload seam. The runtime supplies an impl that re-resolves the LLM
/// config (including any stored key) and atomically swaps the running
/// provider's inner backend; tests / unwired runtimes leave it absent.
#[async_trait]
pub trait LlmReloadTrigger: Send + Sync {
    /// Re-resolve and hot-swap the active provider. The error string is for
    /// logging only and must stay free of secrets / backend internals.
    async fn reload(&self) -> Result<(), String>;
}

/// Operator-wide LLM configuration service backing the webui2 settings surface.
pub struct RebornLlmConfigService {
    boot: RebornBootConfig,
    repo: ProviderRepo,
    keys: LlmKeyStore,
    reload: Option<Arc<dyn LlmReloadTrigger>>,
}

impl RebornLlmConfigService {
    pub fn new(boot: RebornBootConfig, keys: LlmKeyStore) -> Self {
        let repo = ProviderRepo::new(boot.home().providers_file_path());
        Self {
            boot,
            repo,
            keys,
            reload: None,
        }
    }

    /// Attach the live-reload trigger (from the runtime).
    pub fn with_reload_trigger(mut self, reload: Arc<dyn LlmReloadTrigger>) -> Self {
        self.reload = Some(reload);
        self
    }

    fn admin(&self) -> RebornProviderAdmin {
        RebornProviderAdmin::new(self.boot.clone())
    }

    /// Persist-then-reload: the file write already happened; refresh the
    /// running provider. A reload failure is logged, not fatal — the on-disk
    /// config is authoritative and applies on next restart.
    ///
    /// The reload swaps the live provider's *inner* backend. It does NOT yet
    /// update the model gateway's pinned model-profile route or cost table
    /// (those are built once at boot), so changing the active *model* fully
    /// applies on restart; for providers that honor per-request model overrides
    /// the gateway still pins the boot model until then. A swappable model
    /// gateway (and live reload from a no-LLM cold boot, where no reload handle
    /// exists at all) is owned by the first-run provider work.
    async fn refresh_running_provider(&self) {
        let Some(reload) = self.reload.as_ref() else {
            // Cold boot: no LLM was configured at startup, so there is no live
            // provider to swap into. Don't fail silently — tell the operator the
            // saved config needs a restart to take effect.
            tracing::warn!(
                "LLM configuration saved, but no live LLM provider was configured at startup \
                 (no config.toml or provider env creds), so it cannot be applied to the running \
                 process. Restart the server to use the new configuration."
            );
            return;
        };
        if let Err(reason) = reload.reload().await {
            tracing::warn!(
                reason = %reason,
                "LLM config persisted but live provider reload failed; change applies on restart"
            );
        }
    }

    async fn build_snapshot(&self) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        let list = self.admin().list(None, true).map_err(map_admin_error)?;
        let overlay_ids = self
            .repo
            .load()
            .map_err(|_| LlmConfigServiceError::Unavailable)?
            .into_iter()
            .map(|definition| definition.id.to_lowercase())
            .collect::<Vec<_>>();

        let mut providers = Vec::with_capacity(list.providers.len());
        let mut active = None;
        for info in list.providers {
            let api_key_set = self
                .keys
                .exists(&info.id)
                .await
                .map_err(|_| LlmConfigServiceError::Unavailable)?;
            let builtin = !overlay_ids.contains(&info.id.to_lowercase());
            let metadata = info.metadata;
            if info.active && active.is_none() {
                active = Some(LlmActiveSelection {
                    provider_id: info.id.clone(),
                    model: info.active_model.clone(),
                });
            }
            providers.push(LlmProviderView {
                id: info.id,
                description: info.description,
                adapter: metadata
                    .as_ref()
                    .map(|meta| meta.protocol.clone())
                    .unwrap_or_default(),
                default_model: info.default_model,
                base_url: metadata.as_ref().and_then(|meta| meta.base_url.clone()),
                builtin,
                active: info.active,
                active_model: info.active_model,
                api_key_required: metadata
                    .as_ref()
                    .map(|meta| meta.api_key_required)
                    .unwrap_or(false),
                api_key_set,
                can_list_models: metadata
                    .as_ref()
                    .map(|meta| meta.can_list_models)
                    .unwrap_or(false),
            });
        }

        Ok(LlmConfigSnapshot { providers, active })
    }

    /// Build a transient provider from a probe request and run a closure
    /// against it. Reused by `test_connection` and `list_models`.
    async fn probe_provider(
        &self,
        request: &LlmProbeRequest,
    ) -> Result<Arc<dyn ironclaw_llm::LlmProvider>, LlmConfigServiceError> {
        let protocol = parse_adapter(&request.adapter).ok_or_else(|| {
            LlmConfigServiceError::InvalidRequest {
                field: Some("adapter".to_string()),
                reason: format!("unknown adapter `{}`", request.adapter),
            }
        })?;
        let base_url = request
            .base_url
            .clone()
            .filter(|url| !url.trim().is_empty());
        let model = request
            .model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_default();

        let definition = custom_definition(&request.provider_id, protocol, base_url.clone(), model);
        let registry = ProviderRegistry::new(vec![definition]);
        let selection = LlmSlotSelection {
            provider_id: Some(request.provider_id.clone()),
            model: request
                .model
                .clone()
                .filter(|model| !model.trim().is_empty()),
            api_key_env: None,
            base_url,
        };
        let mut config = resolve_against_registry(&selection, &registry).map_err(|error| {
            LlmConfigServiceError::InvalidRequest {
                field: None,
                reason: error.to_string(),
            }
        })?;

        // Prefer the request's inline key; fall back to a stored one.
        if let Some(key) = request.api_key.as_ref() {
            apply_stored_api_key(&mut config, key.clone());
        } else if let Some(stored) = self
            .keys
            .read(&request.provider_id)
            .await
            .map_err(|_| LlmConfigServiceError::Unavailable)?
        {
            apply_stored_api_key(&mut config, stored);
        }

        let session = ironclaw_llm::create_session_manager(config.session.clone()).await;
        ironclaw_llm::build_static_provider_chain(&config, session)
            .await
            .map_err(|_| LlmConfigServiceError::Unavailable)
    }
}

#[async_trait]
impl LlmConfigService for RebornLlmConfigService {
    async fn snapshot(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        self.build_snapshot().await
    }

    async fn upsert_provider(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: UpsertLlmProviderRequest,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        let id = validate_provider_id(&request.id)?;

        let base_url = request
            .base_url
            .clone()
            .filter(|url| !url.trim().is_empty());
        let model = request
            .default_model
            .clone()
            .filter(|model| !model.trim().is_empty());
        let has_new_key = request
            .api_key
            .as_ref()
            .is_some_and(|key| !is_masked_sentinel(key));
        let key_present = has_new_key
            || self
                .keys
                .exists(&id)
                .await
                .map_err(|_| LlmConfigServiceError::Unavailable)?;

        // Editing a built-in must PRESERVE its compiled-in definition (protocol,
        // setup hints, env-var names) and overlay only what the operator
        // changed. Writing a fresh generic definition would strip OAuth/setup
        // from providers like openai_codex, gemini_oauth, nearai, and bedrock.
        let builtin_registry = ironclaw_llm::ProviderRegistry::try_load_from_path(None)
            .map_err(|_| LlmConfigServiceError::Unavailable)?;
        let definition = build_overlay_definition(
            &id,
            builtin_registry.find(&id),
            &request.adapter,
            base_url,
            model,
            key_present,
            request.name.as_deref(),
        )?;

        self.repo
            .upsert(definition)
            .map_err(|_| LlmConfigServiceError::Unavailable)?;

        // Store the key value only when a real (non-sentinel) one was supplied.
        if has_new_key && let Some(key) = request.api_key.as_ref() {
            self.keys
                .put(&id, key.clone())
                .await
                .map_err(|_| LlmConfigServiceError::Unavailable)?;
        }

        if request.set_active {
            self.admin()
                .set_provider(&id, request.model.as_deref())
                .map_err(map_admin_error)?;
        }

        self.refresh_running_provider().await;
        self.snapshot(caller).await
    }

    async fn delete_provider(
        &self,
        caller: WebUiAuthenticatedCaller,
        provider_id: String,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        let id = validate_provider_id(&provider_id)?;
        let removed = self
            .repo
            .delete(&id)
            .map_err(|_| LlmConfigServiceError::Unavailable)?;
        if !removed {
            return Err(LlmConfigServiceError::NotFound);
        }
        // Best-effort: drop any stored key for the deleted provider.
        let _ = self.keys.delete(&id).await;

        self.refresh_running_provider().await;
        self.snapshot(caller).await
    }

    async fn set_active(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: SetActiveLlmRequest,
    ) -> Result<LlmConfigSnapshot, LlmConfigServiceError> {
        let id = validate_provider_id(&request.provider_id)?;
        self.admin()
            .set_provider(&id, request.model.as_deref())
            .map_err(map_admin_error)?;
        self.refresh_running_provider().await;
        self.snapshot(caller).await
    }

    async fn test_connection(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmProbeResult, LlmConfigServiceError> {
        let provider = self.probe_provider(&request).await?;
        match provider.list_models().await {
            Ok(models) if !models.is_empty() => Ok(LlmProbeResult {
                ok: true,
                message: format!("connection ok — {} models available", models.len()),
            }),
            Ok(_) => Ok(LlmProbeResult {
                ok: true,
                message: "provider configured; this adapter does not expose a model list to verify"
                    .to_string(),
            }),
            Err(_) => Ok(LlmProbeResult {
                ok: false,
                message: "could not reach the provider with these settings".to_string(),
            }),
        }
    }

    async fn list_models(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: LlmProbeRequest,
    ) -> Result<LlmModelsResult, LlmConfigServiceError> {
        let provider = self.probe_provider(&request).await?;
        match provider.list_models().await {
            Ok(models) => Ok(LlmModelsResult {
                ok: true,
                models,
                message: String::new(),
            }),
            Err(_) => Ok(LlmModelsResult {
                ok: false,
                models: Vec::new(),
                message: "could not list models for this provider".to_string(),
            }),
        }
    }
}

/// Parse a wire adapter name (e.g. `open_ai_completions`) into a protocol.
fn parse_adapter(adapter: &str) -> Option<ProviderProtocol> {
    serde_json::from_value(serde_json::Value::String(adapter.to_string())).ok()
}

/// Resolve the overlay `ProviderDefinition` to write for an upsert.
///
/// When `builtin` is `Some` the id names a compiled-in provider: clone its
/// definition (preserving protocol, setup hints, env-var names) and overlay
/// only the operator's `base_url`/`model`, relaxing `api_key_required` when a
/// key is stored (so resolution doesn't demand the env var; the stored value is
/// injected at provider-build time). When `builtin` is `None` it's a brand-new
/// custom provider, which needs a valid `adapter`.
fn build_overlay_definition(
    id: &str,
    builtin: Option<&ProviderDefinition>,
    adapter: &str,
    base_url: Option<String>,
    model: Option<String>,
    key_present: bool,
    name: Option<&str>,
) -> Result<ProviderDefinition, LlmConfigServiceError> {
    if let Some(builtin) = builtin {
        let mut def = builtin.clone();
        if let Some(base_url) = base_url {
            def.default_base_url = Some(base_url);
        }
        if let Some(model) = model {
            def.default_model = model;
        }
        if key_present {
            def.api_key_required = false;
        }
        return Ok(def);
    }

    let protocol = parse_adapter(adapter).ok_or_else(|| LlmConfigServiceError::InvalidRequest {
        field: Some("adapter".to_string()),
        reason: format!("unknown adapter `{adapter}`"),
    })?;
    let mut def = custom_definition(id, protocol, base_url, model.unwrap_or_default());
    def.description = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.to_string());
    Ok(def)
}

/// Build a custom (operator-defined) provider definition. The API key is never
/// stored in the catalog — `api_key_required = false` so resolution succeeds
/// without an env var, and the stored value is injected at provider-build time.
fn custom_definition(
    id: &str,
    protocol: ProviderProtocol,
    base_url: Option<String>,
    default_model: String,
) -> ProviderDefinition {
    ProviderDefinition {
        id: id.to_string(),
        aliases: Vec::new(),
        protocol,
        default_base_url: base_url,
        base_url_env: None,
        base_url_required: false,
        api_key_env: None,
        api_key_required: false,
        model_env: synthetic_model_env(id),
        default_model,
        description: id.to_string(),
        extra_headers_env: None,
        unsupported_params: Vec::new(),
        setup: None,
    }
}

fn synthetic_model_env(id: &str) -> String {
    let upper: String = id
        .chars()
        .map(|c| {
            if c == '-' {
                '_'
            } else {
                c.to_ascii_uppercase()
            }
        })
        .collect();
    format!("LLM_CUSTOM_{upper}_MODEL")
}

/// The masked sentinel the UI sends for "key unchanged".
fn is_masked_sentinel(value: &SecretString) -> bool {
    value.expose_secret().chars().all(|c| c == '\u{2022}')
}

fn validate_provider_id(id: &str) -> Result<String, LlmConfigServiceError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(LlmConfigServiceError::InvalidRequest {
            field: Some("id".to_string()),
            reason: "provider id cannot be empty".to_string(),
        });
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(LlmConfigServiceError::InvalidRequest {
            field: Some("id".to_string()),
            reason: "provider id may only contain lowercase letters, digits, '_' or '-'"
                .to_string(),
        });
    }
    Ok(trimmed.to_string())
}

fn map_admin_error(error: crate::RebornProviderAdminError) -> LlmConfigServiceError {
    use crate::RebornProviderAdminError as E;
    match error {
        E::UnknownProvider { .. } => LlmConfigServiceError::NotFound,
        E::InvalidRequest { reason } => LlmConfigServiceError::InvalidRequest {
            field: None,
            reason,
        },
        E::LoadRegistry { .. } | E::LoadConfig { .. } | E::UpdateConfig { .. } => {
            LlmConfigServiceError::Unavailable
        }
    }
}

/// Live-reload adapter wired by the runtime. Re-resolves the LLM config from
/// `config.toml` + `providers.json` + the stored key, then hot-swaps the
/// running provider's inner backend via the `ironclaw_llm` reload handle.
pub(crate) struct RebornLlmReloadAdapter {
    boot: RebornBootConfig,
    reload_handle: Arc<ironclaw_llm::LlmReloadHandle>,
    session: Arc<ironclaw_llm::SessionManager>,
    keys: LlmKeyStore,
}

impl RebornLlmReloadAdapter {
    pub(crate) fn new(
        boot: RebornBootConfig,
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
}

#[async_trait]
impl LlmReloadTrigger for RebornLlmReloadAdapter {
    async fn reload(&self) -> Result<(), String> {
        let config_file = RebornConfigFile::load(&self.boot.home().config_file_path())
            .map_err(|error| error.to_string())?;
        let Some(resolved) = resolve_reborn_runtime_llm(&self.boot, config_file.as_ref())
            .map_err(|error| error.to_string())?
        else {
            // No provider selected yet — nothing to swap.
            return Ok(());
        };
        let provider_id = resolved.provider_id().to_string();
        let mut config = resolved.config;
        if let Some(stored) = self
            .keys
            .read(&provider_id)
            .await
            .map_err(|error| error.to_string())?
        {
            apply_stored_api_key(&mut config, stored);
        }
        self.reload_handle
            .reload(&config, Arc::clone(&self.session))
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_adapters() {
        assert_eq!(
            parse_adapter("open_ai_completions"),
            Some(ProviderProtocol::OpenAiCompletions)
        );
        assert_eq!(
            parse_adapter("anthropic"),
            Some(ProviderProtocol::Anthropic)
        );
        assert_eq!(parse_adapter("ollama"), Some(ProviderProtocol::Ollama));
        assert_eq!(parse_adapter("nearai"), Some(ProviderProtocol::NearAi));
        assert_eq!(parse_adapter("near_ai"), Some(ProviderProtocol::NearAi));
        assert_eq!(parse_adapter("not_a_real_adapter"), None);
    }

    #[test]
    fn custom_definition_never_requires_or_names_a_key() {
        let def = custom_definition(
            "acme",
            ProviderProtocol::OpenAiCompletions,
            Some("https://api.acme.test/v1".to_string()),
            "acme-large".to_string(),
        );
        assert!(!def.api_key_required);
        assert!(def.api_key_env.is_none());
        assert_eq!(def.model_env, "LLM_CUSTOM_ACME_MODEL");
        assert_eq!(def.default_model, "acme-large");
    }

    #[test]
    fn masked_sentinel_detected() {
        assert!(is_masked_sentinel(&SecretString::from(
            "\u{2022}\u{2022}\u{2022}"
        )));
        assert!(!is_masked_sentinel(&SecretString::from("sk-real-key")));
    }

    #[test]
    fn provider_id_validation_rejects_bad_input() {
        assert!(validate_provider_id("acme_1").is_ok());
        assert!(validate_provider_id("Acme").is_err());
        assert!(validate_provider_id("has space").is_err());
        assert!(validate_provider_id("  ").is_err());
    }

    #[test]
    fn editing_a_builtin_preserves_protocol_and_setup() {
        // openai_codex is a built-in with a dedicated protocol + OAuth setup.
        let registry = ironclaw_llm::ProviderRegistry::try_load_from_path(None).expect("registry");
        let builtin = registry.find("openai_codex").expect("openai_codex builtin");
        assert_eq!(builtin.protocol, ProviderProtocol::OpenAiCodex);
        let had_setup = builtin.setup.is_some();

        let def = build_overlay_definition(
            "openai_codex",
            Some(builtin),
            "ignored_adapter",
            None,
            Some("gpt-5.3-codex".to_string()),
            false,
            None,
        )
        .expect("overlay def");

        // Protocol + setup preserved; only the model changed.
        assert_eq!(def.protocol, ProviderProtocol::OpenAiCodex);
        assert_eq!(def.setup.is_some(), had_setup);
        assert_eq!(def.default_model, "gpt-5.3-codex");
        assert_eq!(def.id, "openai_codex");
    }

    #[test]
    fn editing_a_builtin_relaxes_key_requirement_when_key_stored() {
        let registry = ironclaw_llm::ProviderRegistry::try_load_from_path(None).expect("registry");
        let openai = registry.find("openai").expect("openai builtin");
        assert!(openai.api_key_required, "openai requires a key by default");

        let def = build_overlay_definition(
            "openai",
            Some(openai),
            "open_ai_completions",
            None,
            None,
            true, // a key is stored
            None,
        )
        .expect("overlay def");
        assert!(
            !def.api_key_required,
            "stored key means resolution must not demand the env var"
        );
        assert_eq!(def.protocol, ProviderProtocol::OpenAiCompletions);
    }

    #[test]
    fn brand_new_custom_provider_uses_the_request_adapter() {
        let def = build_overlay_definition(
            "acme",
            None,
            "anthropic",
            Some("https://acme.test/v1".to_string()),
            Some("acme-1".to_string()),
            false,
            Some("Acme"),
        )
        .expect("overlay def");
        assert_eq!(def.protocol, ProviderProtocol::Anthropic);
        assert_eq!(def.description, "Acme");
        assert!(!def.api_key_required);
    }

    #[test]
    fn brand_new_custom_provider_rejects_unknown_adapter() {
        let err = build_overlay_definition("acme", None, "nonsense", None, None, false, None)
            .expect_err("unknown adapter must fail");
        assert!(matches!(err, LlmConfigServiceError::InvalidRequest { .. }));
    }
}
