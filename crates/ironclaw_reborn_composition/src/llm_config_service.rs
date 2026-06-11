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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ironclaw_llm::registry::{ProviderDefinition, ProviderProtocol, ProviderRegistry};
use ironclaw_llm::{NearWalletSignedMessage, OpenAiCodexConfig, OpenAiCodexSessionManager};
use ironclaw_product_workflow::{
    CodexLoginStart, LlmActiveSelection, LlmConfigService, LlmConfigServiceError,
    LlmConfigSnapshot, LlmModelsResult, LlmProbeRequest, LlmProbeResult, LlmProviderView,
    NearAiLoginRequest, NearAiLoginStart, NearAiWalletLoginRequest, NearAiWalletLoginResult,
    SetActiveLlmRequest, UpsertLlmProviderRequest, WebUiAuthenticatedCaller,
};
use ironclaw_reborn_config::{LlmSlotSelection, RebornBootConfig};
use secrecy::{ExposeSecret as _, SecretString};

use crate::llm_catalog::{apply_stored_api_key, resolve_against_registry};
use crate::{LlmKeyStore, ProviderRepo, RebornProviderAdmin};

const NEARAI_LOGIN_STATE_TTL: Duration = Duration::from_secs(15 * 60);
const CODEX_LOGIN_ATTEMPT_TTL: Duration = Duration::from_secs(15 * 60);

/// In-memory CSRF state for NEAR AI browser redirects. The login start endpoint
/// issues a state token, and the public callback must consume it before any
/// operator-wide credential write happens.
#[derive(Debug, Default)]
pub(crate) struct NearAiLoginStateStore {
    states: tokio::sync::Mutex<HashMap<String, Instant>>,
}

impl NearAiLoginStateStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn issue(&self) -> String {
        let state = uuid::Uuid::new_v4().to_string();
        let mut states = self.states.lock().await;
        prune_expired(&mut states, Instant::now());
        states.insert(state.clone(), Instant::now() + NEARAI_LOGIN_STATE_TTL);
        state
    }

    #[cfg(any(test, feature = "webui-v2-beta"))]
    #[allow(dead_code)]
    pub(crate) async fn consume(&self, state: &str) -> bool {
        let mut states = self.states.lock().await;
        let now = Instant::now();
        prune_expired(&mut states, now);
        states
            .remove(state)
            .is_some_and(|expires_at| expires_at > now)
    }
}

#[derive(Debug, Clone)]
struct CodexLoginAttempt {
    id: uuid::Uuid,
    user_code: String,
    verification_uri: String,
    expires_at: Instant,
}

fn prune_expired(states: &mut HashMap<String, Instant>, now: Instant) {
    states.retain(|_, expires_at| *expires_at > now);
}

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
    /// The runtime's NEAR AI session manager — the same instance the live
    /// provider reads its token from, so a completed login takes effect on
    /// reload. Absent when the runtime has no LLM seam wired.
    nearai_session: Option<Arc<ironclaw_llm::SessionManager>>,
    nearai_login_states: Arc<NearAiLoginStateStore>,
    codex_login_attempts: Arc<tokio::sync::Mutex<HashMap<String, CodexLoginAttempt>>>,
}

impl RebornLlmConfigService {
    pub fn new(boot: RebornBootConfig, keys: LlmKeyStore) -> Self {
        let repo = ProviderRepo::new(boot.home().providers_file_path());
        Self {
            boot,
            repo,
            keys,
            reload: None,
            nearai_session: None,
            nearai_login_states: Arc::new(NearAiLoginStateStore::new()),
            codex_login_attempts: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Attach the live-reload trigger (from the runtime).
    pub fn with_reload_trigger(mut self, reload: Arc<dyn LlmReloadTrigger>) -> Self {
        self.reload = Some(reload);
        self
    }

    /// Attach the runtime's NEAR AI session manager (enables NEAR AI login).
    pub fn with_nearai_session(mut self, session: Arc<ironclaw_llm::SessionManager>) -> Self {
        self.nearai_session = Some(session);
        self
    }

    /// Attach the runtime's NEAR AI login-state store. The start endpoint and
    /// public callback must share the same store.
    pub(crate) fn with_nearai_login_states(mut self, states: Arc<NearAiLoginStateStore>) -> Self {
        self.nearai_login_states = states;
        self
    }

    fn admin(&self) -> RebornProviderAdmin {
        RebornProviderAdmin::new(self.boot.clone())
    }

    /// Persist-then-reload: the file write already happened; refresh the
    /// running provider. A reload failure is logged, not fatal — the on-disk
    /// config is authoritative and applies on next restart.
    ///
    /// The reload swaps the live provider's *inner* backend. The gateway's
    /// model profile is intentionally unpinned so requests use the reloaded
    /// provider's active model instead of the model selected at boot.
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
        let list = self.admin_list_async().await.map_err(map_admin_error)?;
        let builtin_registry = ironclaw_llm::ProviderRegistry::try_load_from_path(None)
            .map_err(|_| LlmConfigServiceError::Unavailable)?;

        let mut providers = Vec::with_capacity(list.providers.len());
        let mut active = None;
        for info in list.providers {
            let stored_key_set = self.stored_key_set_for_snapshot(&info.id).await;
            let builtin = builtin_registry.find(&info.id).is_some();
            let metadata = info.metadata;
            let env_key_set = metadata.as_ref().is_some_and(metadata_env_key_set);
            let api_key_set = stored_key_set || env_key_set;
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
                accepts_api_key: metadata
                    .as_ref()
                    .map(|meta| meta.accepts_api_key)
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

    async fn stored_key_set_for_snapshot(&self, provider_id: &str) -> bool {
        match self.keys.exists(provider_id).await {
            Ok(stored_key_set) => stored_key_set,
            Err(error) => {
                tracing::warn!(
                    provider_id,
                    error = %error,
                    "LLM provider snapshot could not read stored key metadata; reporting api_key_set=false"
                );
                false
            }
        }
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
        let stored_key_allowed = self.probe_matches_persisted_provider(request).await?;
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

        // Prefer the request's inline key. Stored operator credentials are only
        // safe when the probe targets the persisted provider endpoint; otherwise
        // a caller-controlled base_url could exfiltrate that key.
        if let Some(key) = request.api_key.as_ref() {
            apply_stored_api_key(&mut config, key.clone());
        } else if stored_key_allowed {
            if let Some(stored) = self
                .keys
                .read(&request.provider_id)
                .await
                .map_err(|_| LlmConfigServiceError::Unavailable)?
            {
                apply_stored_api_key(&mut config, stored);
            }
        } else if self
            .keys
            .exists(&request.provider_id)
            .await
            .map_err(|_| LlmConfigServiceError::Unavailable)?
        {
            // A stored key exists but this probe targets an overridden endpoint:
            // refuse to inject it (a caller-controlled base_url could exfiltrate
            // it). The caller must supply an inline key to probe elsewhere.
            return Err(LlmConfigServiceError::InvalidRequest {
                field: Some("api_key".to_string()),
                reason: "inline api_key is required when probing an overridden provider endpoint"
                    .to_string(),
            });
        }
        // Otherwise there is neither an inline key nor a stored key to protect,
        // so probe keyless — local OpenAI-compatible endpoints (e.g. Ollama)
        // need no auth and must not be blocked behind a phantom key requirement.

        let session = ironclaw_llm::create_session_manager(config.session.clone()).await;
        ironclaw_llm::build_static_provider_chain(&config, session)
            .await
            .map_err(|_| LlmConfigServiceError::Unavailable)
    }

    async fn probe_matches_persisted_provider(
        &self,
        request: &LlmProbeRequest,
    ) -> Result<bool, LlmConfigServiceError> {
        let providers_path = self.boot.home().providers_file_path();
        let provider_id = request.provider_id.clone();
        let registry = tokio::task::spawn_blocking(move || {
            ironclaw_llm::ProviderRegistry::try_load_from_path(Some(providers_path.as_path()))
        })
        .await
        .map_err(|_| LlmConfigServiceError::Unavailable)?
        .map_err(|_| LlmConfigServiceError::Unavailable)?;
        let Some(definition) = registry.find(&provider_id) else {
            return Ok(false);
        };
        let Some(protocol) = parse_adapter(&request.adapter) else {
            return Ok(false);
        };
        Ok(protocol == definition.protocol
            && normalized_endpoint(request.base_url.as_deref())
                == normalized_endpoint(definition.default_base_url.as_deref()))
    }

    async fn admin_list_async(
        &self,
    ) -> Result<crate::RebornProviderList, crate::RebornProviderAdminError> {
        let admin = self.admin();
        tokio::task::spawn_blocking(move || admin.list(None, true))
            .await
            .map_err(|error| crate::RebornProviderAdminError::InvalidRequest {
                reason: format!("provider-admin task failed: {error}"),
            })?
    }

    async fn set_provider_async(
        &self,
        id: String,
        model: Option<String>,
    ) -> Result<(), crate::RebornProviderAdminError> {
        let admin = self.admin();
        tokio::task::spawn_blocking(move || admin.set_provider(&id, model.as_deref()).map(|_| ()))
            .await
            .map_err(|error| crate::RebornProviderAdminError::InvalidRequest {
                reason: format!("provider-admin task failed: {error}"),
            })?
    }

    async fn rollback_provider_definition(
        &self,
        id: &str,
        previous_definition: Option<ProviderDefinition>,
    ) {
        let overlay_result = if let Some(previous_definition) = previous_definition {
            self.repo
                .upsert_async(previous_definition)
                .await
                .map(|_| ())
        } else {
            self.repo.delete_async(id).await.map(|_| ())
        };
        if let Err(error) = overlay_result {
            tracing::warn!(
                provider_id = %id,
                error = %error,
                "failed to roll back LLM provider overlay after active-selection failure",
            );
        }
    }

    async fn rollback_provider_key(&self, id: &str, previous_key: Option<SecretString>) {
        let key_result = if let Some(previous_key) = previous_key {
            self.keys.put(id, previous_key).await.map(|_| ())
        } else {
            self.keys.delete(id).await.map(|_| ())
        };
        if let Err(error) = key_result {
            tracing::warn!(
                provider_id = %id,
                error = %error,
                "failed to roll back LLM provider key after active-selection failure",
            );
        }
    }

    async fn rollback_upsert(
        &self,
        id: &str,
        previous_definition: Option<ProviderDefinition>,
        previous_key: Option<SecretString>,
        key_was_updated: bool,
    ) {
        self.rollback_provider_definition(id, previous_definition)
            .await;
        if key_was_updated {
            self.rollback_provider_key(id, previous_key).await;
        }
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
        let previous_key = if has_new_key {
            self.keys.read(&id).await.map_err(|error| {
                tracing::error!(provider_id = %id, %error, "LLM provider save: reading existing stored key failed");
                LlmConfigServiceError::Unavailable
            })?
        } else {
            None
        };
        let stored_key_present = if has_new_key {
            previous_key.is_some()
        } else {
            self.keys.exists(&id).await.map_err(|error| {
                tracing::error!(provider_id = %id, %error, "LLM provider save: checking stored key existence failed");
                LlmConfigServiceError::Unavailable
            })?
        };
        let previous_overlay = self.repo.load_async().await.map_err(|error| {
            tracing::error!(provider_id = %id, %error, "LLM provider save: loading provider overlay failed");
            LlmConfigServiceError::Unavailable
        })?;
        let previous_definition = previous_overlay
            .iter()
            .find(|definition| definition.id.eq_ignore_ascii_case(&id))
            .cloned();

        // Editing a built-in must PRESERVE its compiled-in definition (protocol,
        // setup hints, env-var names) and overlay only what the operator
        // changed. Writing a fresh generic definition would strip OAuth/setup
        // from providers like openai_codex, gemini_oauth, nearai, and bedrock.
        let builtin_registry = ironclaw_llm::ProviderRegistry::try_load_from_path(None)
            .map_err(|_| LlmConfigServiceError::Unavailable)?;
        let builtin = builtin_registry.find(&id);
        let key_present =
            has_new_key || stored_key_present || builtin.is_some_and(definition_env_key_set);
        let definition = build_overlay_definition(
            &id,
            builtin,
            &request.adapter,
            base_url,
            model,
            key_present,
            request.name.as_deref(),
        )?;

        // Store the key value only when a real (non-sentinel) one was supplied.
        if has_new_key && let Some(key) = request.api_key.as_ref() {
            self.keys.put(&id, key.clone()).await.map_err(|error| {
                tracing::error!(provider_id = %id, %error, "LLM provider save: storing API key failed");
                LlmConfigServiceError::Unavailable
            })?;
        }

        if let Err(error) = self.repo.upsert_async(definition).await {
            tracing::error!(provider_id = %id, %error, "LLM provider save: writing provider overlay failed");
            if has_new_key {
                self.rollback_provider_key(&id, previous_key).await;
            }
            return Err(LlmConfigServiceError::Unavailable);
        }

        if request.set_active {
            let active_result = self
                .set_provider_async(id.clone(), request.model.clone())
                .await;
            if let Err(error) = active_result {
                tracing::error!(provider_id = %id, %error, "LLM provider save: writing active selection failed");
                self.rollback_upsert(&id, previous_definition, previous_key, has_new_key)
                    .await;
                return Err(map_admin_error(error));
            }
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
            .delete_async(&id)
            .await
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
        self.set_provider_async(id, request.model)
            .await
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
            Err(error) => {
                tracing::debug!(
                    provider_id = %request.provider_id,
                    adapter = %request.adapter,
                    error = %error,
                    "list_models probe failed"
                );
                Ok(LlmModelsResult {
                    ok: false,
                    models: Vec::new(),
                    message: "could not list models for this provider".to_string(),
                })
            }
        }
    }

    async fn start_nearai_login(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: NearAiLoginRequest,
    ) -> Result<NearAiLoginStart, LlmConfigServiceError> {
        let session = self
            .nearai_session
            .as_ref()
            .ok_or(LlmConfigServiceError::Unavailable)?;

        // Point NEAR AI at the server's own public callback route (aligned with
        // the SSO PublicRouteMount pattern, not a second loopback listener).
        // NEAR AI redirects to `<frontend_callback>/auth/callback?token=...`, so
        // `frontend_callback` is this server's NEAR AI route prefix on the
        // browser's own origin (validated to a bare scheme://host[:port]).
        let origin = sanitize_origin(&request.origin).ok_or_else(|| {
            LlmConfigServiceError::InvalidRequest {
                field: Some("origin".to_string()),
                reason: "origin must be a bare http(s) origin".to_string(),
            }
        })?;
        let state = self.nearai_login_states.issue().await;
        let frontend_callback = format!("{origin}{NEARAI_LOGIN_PREFIX}/{state}");
        let mut auth_url = url::Url::parse(&format!(
            "{}/v1/auth/{}",
            session.auth_base_url(),
            request.provider.as_path()
        ))
        .map_err(|_| LlmConfigServiceError::Internal)?;
        auth_url
            .query_pairs_mut()
            .append_pair("frontend_callback", &frontend_callback);

        Ok(NearAiLoginStart {
            auth_url: auth_url.to_string(),
        })
    }

    async fn start_codex_login(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<CodexLoginStart, LlmConfigServiceError> {
        let attempt_key = codex_login_attempt_key(&caller);
        let now = Instant::now();
        {
            let mut attempts = self.codex_login_attempts.lock().await;
            attempts.retain(|_, attempt| attempt.expires_at > now);
            if let Some(attempt) = attempts.get(&attempt_key) {
                return Ok(CodexLoginStart {
                    user_code: attempt.user_code.clone(),
                    verification_uri: attempt.verification_uri.clone(),
                });
            }
        }

        // Point the login manager at the same session file the live openai_codex
        // provider reads on reload (mirror resolution.rs env precedence). The
        // model is irrelevant to the device-code flow, so leave it defaulted.
        let codex_config = OpenAiCodexConfig::build(
            None,
            nonempty_env("OPENAI_CODEX_AUTH_URL"),
            nonempty_env("OPENAI_CODEX_API_URL"),
            nonempty_env("OPENAI_CODEX_CLIENT_ID"),
            nonempty_env("OPENAI_CODEX_SESSION_PATH").map(std::path::PathBuf::from),
            None,
        );
        let manager = OpenAiCodexSessionManager::new(codex_config)
            .map_err(|_| LlmConfigServiceError::Internal)?;
        let start = manager
            .initiate_device_code()
            .await
            .map_err(|_| LlmConfigServiceError::Internal)?;

        let login = CodexLoginStart {
            user_code: start.user_code.clone(),
            verification_uri: start.verification_uri.clone(),
        };
        let attempt_id = uuid::Uuid::new_v4();
        {
            let mut attempts = self.codex_login_attempts.lock().await;
            attempts.insert(
                attempt_key.clone(),
                CodexLoginAttempt {
                    id: attempt_id,
                    user_code: login.user_code.clone(),
                    verification_uri: login.verification_uri.clone(),
                    expires_at: Instant::now() + CODEX_LOGIN_ATTEMPT_TTL,
                },
            );
        }

        // Poll for authorization off-thread: persist the tokens, make Codex the
        // active provider, and hot-swap the running provider. The frontend polls
        // the snapshot until openai_codex is active. The on-disk session file is
        // the source of truth, so a reload failure still applies on restart.
        let boot = self.boot.clone();
        let reload = self.reload.clone();
        let attempts = Arc::clone(&self.codex_login_attempts);
        tokio::spawn(async move {
            if let Err(error) = manager.complete_device_code(&start).await {
                tracing::debug!(%error, "codex device login did not complete");
                remove_codex_attempt_if_current(&attempts, &attempt_key, attempt_id).await;
                return;
            }
            if !remove_codex_attempt_if_current(&attempts, &attempt_key, attempt_id).await {
                tracing::debug!("codex login completed after a newer attempt superseded it");
                return;
            }
            if let Err(error) = RebornProviderAdmin::new(boot).set_provider("openai_codex", None) {
                tracing::debug!(%error, "codex login: could not set active provider");
                return;
            }
            if let Some(reload) = reload
                && let Err(error) = reload.reload().await
            {
                tracing::debug!(%error, "codex login: live reload failed; applies on restart");
            }
        });

        Ok(login)
    }

    async fn complete_nearai_wallet_login(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: NearAiWalletLoginRequest,
    ) -> Result<NearAiWalletLoginResult, LlmConfigServiceError> {
        let session = self
            .nearai_session
            .as_ref()
            .ok_or(LlmConfigServiceError::Unavailable)?;

        // Exchange the browser-signed NEP-413 message for a NEAR AI session
        // token. NEAR AI is the authority on the message/recipient/nonce
        // constraints, so a bad signature comes back as an error here; surface a
        // generic failure rather than leaking the provider's reason.
        let signed = NearWalletSignedMessage {
            account_id: request.account_id,
            public_key: request.public_key,
            signature: request.signature,
            message: request.message,
            recipient: request.recipient,
            nonce: request.nonce,
            callback_url: request.callback_url,
        };
        let token = session.near_wallet_login(&signed).await.map_err(|error| {
            tracing::debug!(%error, "NEAR AI wallet login exchange failed");
            LlmConfigServiceError::InvalidRequest {
                field: None,
                reason: "NEAR wallet sign-in failed".to_string(),
            }
        })?;

        // Apply the token the same way the SSO callback does: persist it, make
        // NEAR AI active, and hot-swap the running provider. Without a reload
        // seam the selection still persists and applies on restart.
        session
            .save_session_for_renewer(&token, Some("nearai"))
            .await
            .map_err(|error| {
                tracing::debug!(%error, "NEAR AI wallet login: token persist failed");
                LlmConfigServiceError::Internal
            })?;
        self.admin().set_provider("nearai", None).map_err(|error| {
            tracing::debug!(%error, "NEAR AI wallet login: set active failed");
            LlmConfigServiceError::Internal
        })?;
        let active = match &self.reload {
            Some(reload) => {
                reload.reload().await.map_err(|error| {
                    tracing::debug!(%error, "NEAR AI wallet login: live reload failed");
                    LlmConfigServiceError::Internal
                })?;
                true
            }
            None => false,
        };
        Ok(NearAiWalletLoginResult { active })
    }
}

/// Read an env var, treating empty/whitespace as absent. Mirrors the precedence
/// `ironclaw_llm::resolution` uses so the Codex login manager resolves the same
/// session path / client id / auth URL as the live provider.
fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn codex_login_attempt_key(caller: &WebUiAuthenticatedCaller) -> String {
    format!("{}:{}", caller.tenant_id.as_str(), caller.user_id.as_str())
}

async fn remove_codex_attempt_if_current(
    attempts: &tokio::sync::Mutex<HashMap<String, CodexLoginAttempt>>,
    key: &str,
    attempt_id: uuid::Uuid,
) -> bool {
    let mut attempts = attempts.lock().await;
    let Some(attempt) = attempts.get(key) else {
        return false;
    };
    if attempt.id != attempt_id {
        return false;
    }
    attempts.remove(key);
    true
}

/// Server route prefix handed to NEAR AI as `frontend_callback`, with an issued
/// state segment appended per login flow. NEAR AI appends
/// `/auth/callback?token=...`, so the public callback route is
/// `{NEARAI_LOGIN_PREFIX}/{state}/auth/callback`.
pub(crate) const NEARAI_LOGIN_PREFIX: &str = "/api/webchat/v2/llm/nearai";

/// The public callback path NEAR AI redirects to (token in the query). The
/// `{state}` segment must match an authenticated start request before the
/// callback can write the operator-wide session.
#[cfg(feature = "webui-v2-beta")]
pub(crate) const NEARAI_LOGIN_CALLBACK_PATH: &str =
    "/api/webchat/v2/llm/nearai/{state}/auth/callback";

/// Reduce a browser-supplied origin to a bare `scheme://host[:port]`, rejecting
/// anything with userinfo, a path, a query, or a fragment, plus non-http(s)
/// schemes. NEAR AI redirects the login token to this origin, so it must be a
/// clean origin with no smuggled structure — extra components are rejected
/// outright rather than silently normalized away.
fn sanitize_origin(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    // A bare origin has no credentials, only a root ("" or "/") path, and no
    // query or fragment. Anything else is not an origin we will trust to build
    // the token-bearing callback URL.
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
    {
        return None;
    }
    let host = parsed.host_str()?;
    let mut origin = format!("{}://{host}", parsed.scheme());
    if let Some(port) = parsed.port() {
        origin.push_str(&format!(":{port}"));
    }
    Some(origin)
}

/// Apply a completed NEAR AI login: store the session token on the live
/// session, make NEAR AI the active provider, and hot-swap the running
/// provider. Shared by the public callback route. Errors are log-only strings.
#[cfg(feature = "webui-v2-beta")]
pub(crate) async fn apply_nearai_login(
    session: &ironclaw_llm::SessionManager,
    boot: &RebornBootConfig,
    reload: &dyn LlmReloadTrigger,
    token: &str,
) -> Result<(), String> {
    session
        .save_session_for_renewer(token, Some("nearai"))
        .await
        .map_err(|error| error.to_string())?;
    RebornProviderAdmin::new(boot.clone())
        .set_provider("nearai", None)
        .map_err(|error| format!("set nearai active: {error}"))?;
    reload.reload().await
}

/// Parse a wire adapter name (e.g. `open_ai_completions`) into a protocol.
fn parse_adapter(adapter: &str) -> Option<ProviderProtocol> {
    serde_json::from_value(serde_json::Value::String(adapter.to_string())).ok()
}

fn metadata_env_key_set(metadata: &crate::RebornProviderMetadata) -> bool {
    metadata.api_key_env.as_deref().is_some_and(env_var_present)
}

fn definition_env_key_set(definition: &ProviderDefinition) -> bool {
    definition
        .api_key_env
        .as_deref()
        .is_some_and(env_var_present)
}

fn env_var_present(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

fn normalized_endpoint(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId};
    use ironclaw_reborn_config::{RebornHome, RebornProfile};
    use ironclaw_secrets::{
        InMemorySecretStore, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata,
        SecretStore, SecretStoreError,
    };

    fn boot_for_home(reborn_home: &std::path::Path) -> RebornBootConfig {
        let home = RebornHome::resolve_from_env_parts(
            Some(reborn_home.as_os_str().to_os_string()),
            None,
            None,
        )
        .expect("valid reborn home");
        RebornBootConfig::new(home, RebornProfile::LocalDev)
    }

    fn key_store() -> LlmKeyStore {
        LlmKeyStore::new(Arc::new(InMemorySecretStore::new()))
    }

    struct MetadataUnavailableSecretStore {
        inner: InMemorySecretStore,
    }

    impl MetadataUnavailableSecretStore {
        fn new() -> Self {
            Self {
                inner: InMemorySecretStore::new(),
            }
        }
    }

    #[async_trait]
    impl SecretStore for MetadataUnavailableSecretStore {
        async fn put(
            &self,
            scope: ResourceScope,
            handle: SecretHandle,
            material: SecretMaterial,
        ) -> Result<SecretMetadata, SecretStoreError> {
            self.inner.put(scope, handle, material).await
        }

        async fn metadata(
            &self,
            _scope: &ResourceScope,
            _handle: &SecretHandle,
        ) -> Result<Option<SecretMetadata>, SecretStoreError> {
            Err(SecretStoreError::StoreUnavailable {
                reason: "metadata index unavailable".to_string(),
            })
        }

        async fn delete(
            &self,
            scope: &ResourceScope,
            handle: &SecretHandle,
        ) -> Result<bool, SecretStoreError> {
            self.inner.delete(scope, handle).await
        }

        async fn lease_once(
            &self,
            scope: &ResourceScope,
            handle: &SecretHandle,
        ) -> Result<SecretLease, SecretStoreError> {
            self.inner.lease_once(scope, handle).await
        }

        async fn consume(
            &self,
            scope: &ResourceScope,
            lease_id: SecretLeaseId,
        ) -> Result<SecretMaterial, SecretStoreError> {
            self.inner.consume(scope, lease_id).await
        }

        async fn revoke(
            &self,
            scope: &ResourceScope,
            lease_id: SecretLeaseId,
        ) -> Result<SecretLease, SecretStoreError> {
            self.inner.revoke(scope, lease_id).await
        }

        async fn leases_for_scope(
            &self,
            scope: &ResourceScope,
        ) -> Result<Vec<SecretLease>, SecretStoreError> {
            self.inner.leases_for_scope(scope).await
        }
    }

    fn caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("user-alpha").expect("user"),
            Some(AgentId::new("agent-alpha").expect("agent")),
            Some(ProjectId::new("project-alpha").expect("project")),
        )
    }

    fn upsert_request(
        id: &str,
        api_key: Option<&str>,
        set_active: bool,
    ) -> UpsertLlmProviderRequest {
        UpsertLlmProviderRequest {
            id: id.to_string(),
            name: Some("Acme".to_string()),
            adapter: "open_ai_completions".to_string(),
            base_url: Some("https://api.acme.test/v1".to_string()),
            default_model: Some("acme-1".to_string()),
            api_key: api_key.map(SecretString::from),
            set_active,
            model: Some("acme-1".to_string()),
        }
    }

    fn probe_request(id: &str, base_url: &str, api_key: Option<&str>) -> LlmProbeRequest {
        LlmProbeRequest {
            provider_id: id.to_string(),
            adapter: "open_ai_completions".to_string(),
            base_url: Some(base_url.to_string()),
            model: Some("acme-1".to_string()),
            api_key: api_key.map(SecretString::from),
        }
    }

    #[cfg(feature = "webui-v2-beta")]
    #[tokio::test]
    async fn nearai_login_state_is_single_use() {
        let store = NearAiLoginStateStore::new();
        let state = store.issue().await;

        assert!(store.consume(&state).await);
        assert!(
            !store.consume(&state).await,
            "state must not be reusable after a successful callback"
        );
        assert!(!store.consume("missing-state").await);
    }

    #[test]
    fn sanitize_origin_accepts_bare_origins_only() {
        assert_eq!(
            sanitize_origin("https://app.example.com"),
            Some("https://app.example.com".to_string())
        );
        assert_eq!(
            sanitize_origin("http://localhost:3000"),
            Some("http://localhost:3000".to_string())
        );
        // A trailing root slash is still a bare origin.
        assert_eq!(
            sanitize_origin("https://app.example.com/"),
            Some("https://app.example.com".to_string())
        );
    }

    #[test]
    fn sanitize_origin_rejects_smuggled_structure_and_bad_schemes() {
        // Path, query, fragment, and userinfo must be rejected outright, not
        // silently dropped — they are not part of a trusted callback origin.
        assert_eq!(sanitize_origin("https://app.example.com/evil/path"), None);
        assert_eq!(sanitize_origin("https://app.example.com/?x=1"), None);
        assert_eq!(sanitize_origin("https://app.example.com/#frag"), None);
        assert_eq!(sanitize_origin("https://user:pass@app.example.com"), None);
        assert_eq!(sanitize_origin("ftp://app.example.com"), None);
        assert_eq!(sanitize_origin("javascript:alert(1)"), None);
        assert_eq!(sanitize_origin("not a url"), None);
    }

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

    #[tokio::test]
    async fn upsert_provider_persists_overlay_stores_key_and_preserves_existing_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        let boot = boot_for_home(&reborn_home);
        let keys = key_store();
        let service = RebornLlmConfigService::new(boot.clone(), keys.clone());

        let snapshot = service
            .upsert_provider(caller(), upsert_request("acme", Some("sk-original"), true))
            .await
            .expect("upsert with key");

        let acme = snapshot
            .providers
            .iter()
            .find(|provider| provider.id == "acme")
            .expect("acme provider in snapshot");
        assert!(!acme.builtin);
        assert!(acme.api_key_set);
        assert_eq!(snapshot.active.expect("active").provider_id, "acme");
        let overlay = ProviderRepo::new(boot.home().providers_file_path())
            .load()
            .expect("load overlay");
        assert_eq!(
            overlay
                .iter()
                .filter(|provider| provider.id == "acme")
                .count(),
            1
        );
        assert_eq!(
            keys.read("acme")
                .await
                .expect("read key")
                .expect("stored key")
                .expose_secret(),
            "sk-original"
        );

        service
            .upsert_provider(
                caller(),
                upsert_request("acme", Some("\u{2022}\u{2022}\u{2022}"), false),
            )
            .await
            .expect("masked-key upsert");

        assert_eq!(
            keys.read("acme")
                .await
                .expect("read key")
                .expect("stored key")
                .expose_secret(),
            "sk-original",
            "masked sentinel must preserve the existing stored key"
        );
    }

    #[tokio::test]
    async fn probe_override_requires_inline_key_before_using_stored_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        let boot = boot_for_home(&reborn_home);
        let keys = key_store();
        let service = RebornLlmConfigService::new(boot, keys);

        service
            .upsert_provider(
                caller(),
                upsert_request("acme", Some("sk-stored-secret"), false),
            )
            .await
            .expect("persist provider and stored key");

        let error = service
            .list_models(
                caller(),
                probe_request("acme", "https://attacker.example.test/v1", None),
            )
            .await
            .expect_err("overridden endpoint requires an inline key");

        assert!(
            matches!(
                error,
                LlmConfigServiceError::InvalidRequest {
                    field: Some(ref field),
                    ref reason,
                } if field == "api_key" && reason.contains("overridden provider endpoint")
            ),
            "stored operator keys must not be applied to caller-controlled probe endpoints"
        );
    }

    /// Reproduction for the "Fetch models" failure on a keyless local provider
    /// (e.g. Ollama at `http://localhost:11434`). With neither an inline key nor
    /// a stored key, the probe used to be rejected with an `invalid_request`
    /// validation error demanding an `api_key`. There is no stored key to
    /// exfiltrate, so the probe must be allowed to run keyless instead — it then
    /// just reports the endpoint as unreachable in the test environment.
    #[tokio::test]
    async fn probe_keyless_local_provider_is_not_blocked_on_missing_api_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        let boot = boot_for_home(&reborn_home);
        let service = RebornLlmConfigService::new(boot, key_store());

        // Brand-new custom provider, never persisted, no stored key.
        let result = service
            .list_models(
                caller(),
                probe_request("local-ollama", "http://127.0.0.1:1", None),
            )
            .await;

        assert!(
            result.is_ok(),
            "keyless local provider must pass the probe validation gate, got {result:?}"
        );
    }

    #[tokio::test]
    async fn upsert_builtin_remains_builtin_in_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        let boot = boot_for_home(&reborn_home);
        let service = RebornLlmConfigService::new(boot, key_store());

        let snapshot = service
            .upsert_provider(caller(), upsert_request("openai", Some("sk-openai"), false))
            .await
            .expect("upsert builtin");

        let openai = snapshot
            .providers
            .iter()
            .find(|provider| provider.id == "openai")
            .expect("openai provider in snapshot");
        assert!(
            openai.builtin,
            "overlay edits must not make built-ins custom"
        );
        assert!(openai.api_key_set);
    }

    #[tokio::test]
    async fn nearai_snapshot_exposes_api_key_as_supported_but_not_required() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        let boot = boot_for_home(&reborn_home);
        let service = RebornLlmConfigService::new(boot, key_store());

        let snapshot = service.snapshot(caller()).await.expect("snapshot");
        let nearai = snapshot
            .providers
            .iter()
            .find(|provider| provider.id == "nearai")
            .expect("nearai provider in snapshot");

        assert!(nearai.builtin);
        assert!(
            nearai.accepts_api_key,
            "NEAR AI supports API-key auth in addition to session-token login"
        );
        assert!(
            !nearai.api_key_required,
            "NEAR AI session-token login means API key is not the only setup path"
        );
    }

    #[tokio::test]
    async fn snapshot_survives_stored_key_metadata_unavailable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        let boot = boot_for_home(&reborn_home);
        let keys = LlmKeyStore::new(Arc::new(MetadataUnavailableSecretStore::new()));
        let service = RebornLlmConfigService::new(boot, keys);

        let snapshot = service
            .upsert_provider(caller(), upsert_request("acme", Some("sk-acme"), false))
            .await
            .expect("provider snapshot must remain available");
        let acme = snapshot
            .providers
            .iter()
            .find(|provider| provider.id == "acme")
            .expect("custom provider in snapshot");

        assert!(
            !acme.api_key_set,
            "unavailable stored-key metadata must degrade to api_key_set=false"
        );
    }

    /// Reproduction for issue #4673: saving the NEAR AI (builtin) provider
    /// returns `service_unavailable` even though Test connection succeeds. This
    /// wires the secret store EXACTLY as production `ironclaw-reborn serve` does
    /// — the dynamic `invocation_mount_view` scoped filesystem behind a real
    /// `FilesystemSecretStore` — instead of the in-memory store the other tests
    /// use, so a system-scope write/read regression in that path is caught.
    #[tokio::test]
    async fn upsert_builtin_nearai_with_production_secret_store_succeeds() {
        use ironclaw_secrets::{FilesystemSecretStore, SecretsCrypto};

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        let boot = boot_for_home(&reborn_home);

        let backend = Arc::new(ironclaw_filesystem::InMemoryBackend::default());
        let scoped = crate::wrap_scoped(backend);
        let crypto = Arc::new(
            SecretsCrypto::new(SecretMaterial::from(
                "0123456789abcdef0123456789abcdef".to_string(),
            ))
            .expect("valid master key"),
        );
        let keys = LlmKeyStore::new(Arc::new(FilesystemSecretStore::new(scoped, crypto)));

        let nearai_request = || UpsertLlmProviderRequest {
            id: "nearai".to_string(),
            name: Some("NEAR AI".to_string()),
            adapter: "near_ai".to_string(),
            base_url: Some("https://cloud-api.near.ai".to_string()),
            default_model: Some("deepseek-ai/DeepSeek-V4-Flash".to_string()),
            api_key: Some(SecretString::from("sk-near-test")),
            set_active: true,
            model: Some("deepseek-ai/DeepSeek-V4-Flash".to_string()),
        };

        let service = RebornLlmConfigService::new(boot.clone(), keys.clone());
        // First save persists the operator's NEAR AI key under the system scope.
        let snapshot = service
            .upsert_provider(caller(), nearai_request())
            .await
            .expect("saving the builtin NEAR AI provider must succeed");
        let active = snapshot.active.expect("an active provider after save");
        assert_eq!(active.provider_id, "nearai");
        assert_eq!(
            active.model.as_deref(),
            Some("deepseek-ai/DeepSeek-V4-Flash")
        );

        // The stored system-scoped key must read back (the #4673 regression: the
        // reserved system tenant id failed to deserialize, so any read-back of a
        // system-scoped secret errored — including a second save, which reads the
        // previous key first).
        assert_eq!(
            keys.read("nearai")
                .await
                .expect("system-scope key must read back")
                .expect("a stored key")
                .expose_secret(),
            "sk-near-test"
        );
        service
            .upsert_provider(caller(), nearai_request())
            .await
            .expect("re-saving an already-configured NEAR AI provider must succeed");
    }

    /// Integration coverage for the resolver path at the composition boundary
    /// (review on #4673): an explicit `config.toml` selection is honored
    /// end-to-end through the real `resolve_reborn_runtime_llm`. The env-vs-
    /// selection PRECEDENCE itself is unit-tested in
    /// `ironclaw_llm::resolution` (`explicit_selection_overrides_env_for_model_and_base_url`),
    /// where the env can be set — this crate is `#![forbid(unsafe_code)]` and the
    /// resolver reads raw `std::env::var`, so the env dimension cannot be driven
    /// here; this thin wrapper only adds the config.toml read it is exercised on.
    #[tokio::test]
    async fn reborn_runtime_llm_honors_explicit_config_selection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        let boot = boot_for_home(&reborn_home);

        crate::provider_admin::RebornProviderAdmin::new(boot.clone())
            .set_provider("nearai", Some("deepseek-ai/DeepSeek-V4-Flash"))
            .expect("persist active selection");
        let config_file =
            ironclaw_reborn_config::RebornConfigFile::load(&boot.home().config_file_path())
                .expect("load config file");

        let resolved = crate::llm_catalog::resolve_reborn_runtime_llm(&boot, config_file.as_ref())
            .expect("resolution succeeds")
            .expect("a provider is resolved from the selection");
        assert_eq!(resolved.provider_id(), "nearai");
        assert_eq!(resolved.model(), "deepseek-ai/DeepSeek-V4-Flash");
    }

    #[tokio::test]
    async fn upsert_active_failure_rolls_back_overlay_and_new_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(reborn_home.join("config.toml")).expect("mkdir config path");
        let boot = boot_for_home(&reborn_home);
        let keys = key_store();
        let service = RebornLlmConfigService::new(boot.clone(), keys.clone());

        let error = service
            .upsert_provider(caller(), upsert_request("acme", Some("sk-rollback"), true))
            .await
            .expect_err("config write must fail");

        assert!(matches!(error, LlmConfigServiceError::Unavailable));
        let overlay = ProviderRepo::new(boot.home().providers_file_path())
            .load()
            .expect("load overlay");
        assert!(
            overlay.is_empty(),
            "overlay must roll back when active selection fails"
        );
        assert!(
            !keys.exists("acme").await.expect("key exists check"),
            "new key must roll back when active selection fails"
        );
    }
}
