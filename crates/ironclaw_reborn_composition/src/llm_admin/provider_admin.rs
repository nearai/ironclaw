//! Reborn provider-admin facade.
//!
//! This is the typed provider/model administration surface shared by the
//! standalone CLI and product command workflow. It deliberately edits only
//! Reborn `$IRONCLAW_REBORN_HOME/config.toml` and reads the shared provider
//! catalog through `ironclaw_llm`.

use std::{fmt, path::PathBuf};

use ironclaw_reborn_config::{
    DefaultLlmSlotUpdate, LlmSlotFieldUpdate, LlmSlotSelection, RebornBootConfig, RebornConfigFile,
    begin_default_llm_slot_update, update_default_llm_slot,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct RebornProviderAdmin {
    boot: RebornBootConfig,
}

impl RebornProviderAdmin {
    pub fn new(boot: RebornBootConfig) -> Self {
        Self { boot }
    }

    pub fn list(
        &self,
        provider: Option<&str>,
        verbose: bool,
    ) -> Result<RebornProviderList, RebornProviderAdminError> {
        let home = self.boot.home();
        let registry = self.load_registry()?;
        let config = RebornConfigFile::load(&home.config_file_path()).map_err(|source| {
            RebornProviderAdminError::LoadConfig {
                path: home.config_file_path(),
                source: Box::new(source),
            }
        })?;
        let active = active_llm_selection(
            config.as_ref(),
            &registry,
            Some(home.providers_file_path().as_path()),
        );

        let providers = if let Some(provider) = provider {
            let def = registry.find(provider).ok_or_else(|| {
                RebornProviderAdminError::UnknownProvider {
                    provider: provider.to_string(),
                    providers_file: home.providers_file_path(),
                    known: known_provider_ids(&registry),
                }
            })?;
            vec![provider_info(def, active.as_ref(), true)]
        } else {
            unique_provider_definitions(&registry)
                .into_iter()
                .map(|def| provider_info(def, active.as_ref(), verbose))
                .collect()
        };

        Ok(RebornProviderList {
            providers,
            config_file: home.config_file_path(),
            providers_file: home.providers_file_path(),
            v1_state: RebornV1State::NotUsed,
        })
    }

    pub fn status(&self) -> Result<RebornProviderStatus, RebornProviderAdminError> {
        let home = self.boot.home();
        let registry = self.load_registry()?;
        let config = RebornConfigFile::load(&home.config_file_path()).map_err(|source| {
            RebornProviderAdminError::LoadConfig {
                path: home.config_file_path(),
                source: Box::new(source),
            }
        })?;
        let active = active_llm_selection(
            config.as_ref(),
            &registry,
            Some(home.providers_file_path().as_path()),
        );
        Ok(RebornProviderStatus {
            routes: if active.is_some() {
                RebornModelRoutesState::Configured
            } else {
                RebornModelRoutesState::NotConfigured
            },
            default: active.map(|selection| RebornProviderSelection {
                provider_id: selection.provider_id,
                provider_known: selection.canonical_provider_id.is_some(),
                model: selection.model,
                api_key_env: selection.api_key_env,
                base_url: selection.base_url,
            }),
            config_file: home.config_file_path(),
            providers_file: home.providers_file_path(),
            v1_state: RebornV1State::NotUsed,
        })
    }

    pub fn set_model(
        &self,
        model: &str,
    ) -> Result<RebornProviderWriteOutcome, RebornProviderAdminError> {
        let model = model.trim();
        if model.is_empty() {
            return Err(RebornProviderAdminError::InvalidRequest {
                reason: "model name cannot be empty".to_string(),
            });
        }

        let home = self.boot.home();
        let config_path = home.config_file_path();
        let session = begin_default_llm_slot_update(&config_path).map_err(|source| {
            RebornProviderAdminError::UpdateConfig {
                path: config_path.clone(),
                source: Box::new(source),
            }
        })?;
        let provider_id = session
            .default_llm_slot()
            .map_err(|source| RebornProviderAdminError::UpdateConfig {
                path: config_path.clone(),
                source: Box::new(source),
            })?
            .as_ref()
            .and_then(|selection| selection.provider_id.as_deref())
            .ok_or_else(|| RebornProviderAdminError::InvalidRequest {
                reason: "no default Reborn provider is configured; set a provider first"
                    .to_string(),
            })?
            .to_string();

        let registry = self.load_registry()?;
        let provider_def = registry.find(&provider_id);
        let canonical_id = provider_def
            .map(|def| def.id.clone())
            .unwrap_or_else(|| provider_id.to_string());
        session
            .apply(&DefaultLlmSlotUpdate {
                provider_id: LlmSlotFieldUpdate::Set(canonical_id.clone()),
                model: LlmSlotFieldUpdate::Set(model.to_string()),
                ..Default::default()
            })
            .map_err(|source| RebornProviderAdminError::UpdateConfig {
                path: config_path.clone(),
                source: Box::new(source),
            })?;

        Ok(RebornProviderWriteOutcome {
            provider_id: canonical_id,
            model: model.to_string(),
            api_key_env: provider_def.and_then(|def| def.api_key_env.clone()),
            api_key_required: provider_def.is_some_and(|def| def.api_key_required),
            missing_api_key: provider_def.is_some_and(|def| {
                def.api_key_env.as_deref().is_some_and(|api_key_env| {
                    def.api_key_required && std::env::var_os(api_key_env).is_none()
                })
            }),
            config_file: config_path,
            v1_state: RebornV1State::NotUsed,
        })
    }

    /// Resolve `provider` (an id or alias) to its canonical registry id
    /// without writing anything.
    ///
    /// - For callers that must land a provider-keyed secret (e.g. an
    ///   onboarding API-key prompt) *before* committing the `config.toml`
    ///   selection via [`Self::set_provider`], so a store failure never
    ///   leaves `config.toml` pointing at a provider with no durable key.
    /// - Resolves the same canonical id `set_provider` would land in
    ///   `[llm.default]`, so the secret-store handle and the config
    ///   selection always agree.
    pub fn resolve_provider_id(&self, provider: &str) -> Result<String, RebornProviderAdminError> {
        let provider = provider.trim();
        if provider.is_empty() {
            return Err(RebornProviderAdminError::InvalidRequest {
                reason: "provider id cannot be empty".to_string(),
            });
        }
        let home = self.boot.home();
        let registry = self.load_registry()?;
        let def =
            registry
                .find(provider)
                .ok_or_else(|| RebornProviderAdminError::UnknownProvider {
                    provider: provider.to_string(),
                    providers_file: home.providers_file_path(),
                    known: known_provider_ids(&registry),
                })?;
        Ok(def.id.clone())
    }

    /// The MENU-LEVEL "requires an API key" value for `provider_id` — see
    /// [`effective_api_key_required`]'s doc for the menu-level override.
    ///
    /// - Not restricted to menu-eligible providers: `[llm.default]` may name
    ///   a provider excluded from the numbered menu (e.g. set via `models
    ///   set-provider`), and onboard's idempotent-rerun check
    ///   (`already_configured_outcome`) must answer "does the currently
    ///   configured provider need a key" for any provider id.
    /// - Returns `Ok(None)` when `provider_id` isn't in the registry at all
    ///   — the genuinely "can't tell" case, mirroring
    ///   [`Self::detect_env_llm`]'s read-only contract.
    pub fn effective_api_key_required(
        &self,
        provider_id: &str,
    ) -> Result<Option<bool>, RebornProviderAdminError> {
        let registry = self.load_registry()?;
        Ok(registry.find(provider_id).map(effective_api_key_required))
    }

    pub fn set_provider(
        &self,
        provider: &str,
        model: Option<&str>,
    ) -> Result<RebornProviderWriteOutcome, RebornProviderAdminError> {
        let provider = provider.trim();
        if provider.is_empty() {
            return Err(RebornProviderAdminError::InvalidRequest {
                reason: "provider id cannot be empty".to_string(),
            });
        }

        let home = self.boot.home();
        let config_path = home.config_file_path();
        let registry = self.load_registry()?;
        let def =
            registry
                .find(provider)
                .ok_or_else(|| RebornProviderAdminError::UnknownProvider {
                    provider: provider.to_string(),
                    providers_file: home.providers_file_path(),
                    known: known_provider_ids(&registry),
                })?;
        let model = model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or(&def.default_model);

        update_default_llm_slot(
            &config_path,
            &DefaultLlmSlotUpdate {
                provider_id: LlmSlotFieldUpdate::Set(def.id.clone()),
                model: LlmSlotFieldUpdate::Set(model.to_string()),
                api_key_env: def
                    .api_key_env
                    .clone()
                    .map(LlmSlotFieldUpdate::Set)
                    .unwrap_or(LlmSlotFieldUpdate::Remove),
                base_url: LlmSlotFieldUpdate::Remove,
            },
        )
        .map_err(|source| RebornProviderAdminError::UpdateConfig {
            path: config_path.clone(),
            source: Box::new(source),
        })?;

        Ok(RebornProviderWriteOutcome {
            provider_id: def.id.clone(),
            model: model.to_string(),
            api_key_env: def.api_key_env.clone(),
            api_key_required: def.api_key_required,
            missing_api_key: def.api_key_env.as_deref().is_some_and(|api_key_env| {
                def.api_key_required && std::env::var_os(api_key_env).is_none()
            }),
            config_file: config_path,
            v1_state: RebornV1State::NotUsed,
        })
    }

    /// Providers offered on the interactive `onboard` numbered menu, in
    /// `providers.json` order (`nearai` is entry 0, so always menu item 1).
    ///
    /// - Filters [`ironclaw_llm::ProviderRegistry::selectable`] to
    ///   `SetupHint` kind `ApiKey`/`SessionToken` only — excludes `ollama`,
    ///   `bedrock`, `gemini_oauth`, `openai_codex` by kind, and
    ///   `github_copilot` by id (it declares `kind: "api_key"` like a normal
    ///   provider). Onboarding-scope decision: these stay reachable via
    ///   `ironclaw-reborn config set` / `models set-provider`, just not on
    ///   the numbered menu.
    /// - Also excludes `OpenAiCompatible` kind (`openai_compatible`,
    ///   `cloudflare`) for a correctness reason, not scope: it requires a
    ///   base URL the menu never prompts for, so selecting it here would
    ///   "succeed" at onboard time and fail `serve` boot with
    ///   `LLM_BASE_URL` unset.
    /// - Also excludes [`EXAMPLE_OVERLAY_PROVIDER_ID`] — the tenant-pinned
    ///   OpenRouter example `config::init` seeds into a fresh
    ///   `providers.json` (`PROVIDERS_STUB`), meant to show the overlay
    ///   file's shape, not to be picked live. Matched by id (the only
    ///   stable marker available).
    /// - Returns the serializable [`ProviderMenuEntry`] DTO, not
    ///   `&ProviderDefinition`: `ironclaw_reborn_cli` must never see the
    ///   `ironclaw_llm` setup-hint taxonomy (pinned by
    ///   `reborn_dependency_boundaries`).
    /// - `api_key_required` is a MENU-LEVEL value — see
    ///   [`effective_api_key_required`]'s doc for the `nearai` override.
    pub fn menu_entries(&self) -> Result<Vec<ProviderMenuEntry>, RebornProviderAdminError> {
        let registry = self.load_registry()?;
        Ok(registry
            .selectable()
            .into_iter()
            .filter(|def| def.id != "github_copilot")
            .filter(|def| def.id != EXAMPLE_OVERLAY_PROVIDER_ID)
            .filter(|def| {
                matches!(
                    def.setup.as_ref(),
                    Some(ironclaw_llm::registry::SetupHint::ApiKey { .. })
                        | Some(ironclaw_llm::registry::SetupHint::SessionToken { .. })
                )
            })
            .map(|def| ProviderMenuEntry {
                id: def.id.clone(),
                display_name: def
                    .setup
                    .as_ref()
                    .map(|setup| setup.display_name().to_string())
                    .unwrap_or_else(|| def.id.clone()),
                api_key_required: effective_api_key_required(def),
                description: def.description.clone(),
                aliases: def.aliases.clone(),
            })
            .collect())
    }

    /// Detect an LLM provider configured purely through environment
    /// variables — the same env resolution `resolve_reborn_runtime_llm`'s
    /// fallback path and `run`/`serve`'s stub-gateway warning both use
    /// (`ironclaw_llm::resolve_provider_config_from_env`), wrapped here so
    /// `ironclaw_reborn_cli` (excluded from depending on `ironclaw_llm`
    /// directly, per `reborn_dependency_boundaries`) can offer onboard's
    /// env-detect-and-confirm/silent-seed step.
    ///
    /// Three outcomes, matching onboard's three branches:
    /// - `Ok(Some(detected))`: a complete provider configuration was found
    ///   in the environment (either `LLM_BACKEND` naming a known provider,
    ///   Codex CLI auth, or a provider whose own env vars — API key, base
    ///   URL, or model — are set).
    /// - `Ok(None)`: no LLM environment variables are set at all.
    /// - `Err(_)`: some LLM environment configuration was present (e.g. a
    ///   provider's `*_MODEL` env var set) but incomplete or invalid (e.g.
    ///   the same provider's required API key env var unset, or
    ///   `LLM_BACKEND` naming an unknown provider) — a "partial env" state
    ///   onboard must not silently seed from.
    ///
    /// Never writes anything — pure detection, mirroring
    /// [`Self::resolve_provider_id`]'s read-only contract.
    pub fn detect_env_llm(&self) -> Result<Option<DetectedEnvLlm>, RebornProviderAdminError> {
        let providers_path = self.boot.home().providers_file_path();
        ironclaw_llm::resolve_provider_config_from_env(Some(providers_path.as_path()))
            .map(|resolved| {
                resolved.map(|resolved| DetectedEnvLlm {
                    provider_id: resolved.provider_id().to_string(),
                    model: resolved.model().to_string(),
                })
            })
            .map_err(|source| RebornProviderAdminError::EnvDetection {
                source: Box::new(source),
            })
    }

    /// Re-resolve the API key for a provider previously reported by
    /// [`Self::detect_env_llm`] — a second, targeted env re-read rather than
    /// widening [`DetectedEnvLlm`] (which derives `Serialize` and must never
    /// carry a raw secret) with the key. Used by onboard's env-accept path
    /// (interactive confirm-yes and headless seed) to persist the key into
    /// the encrypted secret store: the installed service only inherits
    /// `IRONCLAW_REBORN_HOME`, not the operator's shell env, so a key left
    /// only in env is invisible to it at boot.
    ///
    /// `Ok(None)` covers both "nothing resolvable from env" and "env now
    /// resolves to a different provider than `provider_id`" (env changed
    /// between the two calls) — callers must not persist a key under the
    /// wrong provider id.
    pub fn resolve_env_api_key(
        &self,
        provider_id: &str,
    ) -> Result<Option<secrecy::SecretString>, RebornProviderAdminError> {
        let providers_path = self.boot.home().providers_file_path();
        let resolved =
            ironclaw_llm::resolve_provider_config_from_env(Some(providers_path.as_path()))
                .map_err(|source| RebornProviderAdminError::EnvDetection {
                    source: Box::new(source),
                })?;
        Ok(resolved
            .filter(|resolved| resolved.provider_id() == provider_id)
            .and_then(|resolved| resolved.api_key().cloned()))
    }

    /// Probe a candidate provider/key/model combination BEFORE it is
    /// persisted — onboard's `provision_via_menu` calls this before either
    /// durable write (secret store, then `[llm.default]`), so a rejected or
    /// unreachable key never lands in config. `api_key` is the caller's
    /// inline candidate (`None` for a keyless provider); `model` is the
    /// caller's override, or `None` to probe the catalog default.
    ///
    /// Reuses the webui2 "test connection"/"list models" probe machinery
    /// ([`crate::llm_admin::llm_config_service::probe_candidate_provider`]),
    /// minus its stored-key fallback (nothing is persisted yet here).
    ///
    /// Only errors when `provider_id` isn't in the registry — a
    /// network/auth/adapter failure during the probe itself reports inside
    /// `Ok(ProviderProbeOutcome { ok: false, .. })`, matching that shared
    /// helper's no-separate-error-channel contract.
    pub async fn probe_candidate(
        &self,
        provider_id: &str,
        api_key: Option<secrecy::SecretString>,
        model: Option<&str>,
    ) -> Result<ProviderProbeOutcome, RebornProviderAdminError> {
        let home = self.boot.home();
        let registry = self.load_registry()?;
        let definition = registry.find(provider_id).ok_or_else(|| {
            RebornProviderAdminError::UnknownProvider {
                provider: provider_id.to_string(),
                providers_file: home.providers_file_path(),
                known: known_provider_ids(&registry),
            }
        })?;
        let base_url = candidate_probe_base_url(definition);
        let request = ironclaw_product_workflow::LlmProbeRequest {
            adapter: provider_protocol_wire_name(definition.protocol),
            base_url,
            provider_id: provider_id.to_string(),
            model: model.map(str::to_string),
            api_key,
        };
        Ok(crate::llm_admin::llm_config_service::probe_candidate_provider(&request).await)
    }

    fn load_registry(&self) -> Result<ironclaw_llm::ProviderRegistry, RebornProviderAdminError> {
        let providers_path = self.boot.home().providers_file_path();
        ironclaw_llm::ProviderRegistry::try_load_from_path(Some(providers_path.as_path())).map_err(
            |error| RebornProviderAdminError::LoadRegistry {
                path: providers_path,
                reason: error.to_string(),
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderList {
    pub providers: Vec<RebornProviderInfo>,
    #[serde(skip_serializing)]
    pub config_file: PathBuf,
    #[serde(skip_serializing)]
    pub providers_file: PathBuf,
    pub v1_state: RebornV1State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderInfo {
    pub id: String,
    pub description: String,
    pub default_model: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<RebornProviderMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderMetadata {
    pub aliases: Vec<String>,
    pub protocol: String,
    pub model_env: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    pub api_key_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_kind: Option<&'static str>,
    pub accepts_api_key: bool,
    pub can_list_models: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderStatus {
    pub routes: RebornModelRoutesState,
    pub default: Option<RebornProviderSelection>,
    #[serde(skip_serializing)]
    pub config_file: PathBuf,
    #[serde(skip_serializing)]
    pub providers_file: PathBuf,
    pub v1_state: RebornV1State,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderSelection {
    pub provider_id: Option<String>,
    pub provider_known: bool,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RebornProviderWriteOutcome {
    pub provider_id: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub api_key_required: bool,
    pub missing_api_key: bool,
    #[serde(skip_serializing)]
    pub config_file: PathBuf,
    pub v1_state: RebornV1State,
}

/// An LLM provider fully resolvable from environment variables alone — see
/// [`RebornProviderAdmin::detect_env_llm`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedEnvLlm {
    pub provider_id: String,
    pub model: String,
}

/// Result of probing a not-yet-persisted provider/key/model combination —
/// see [`RebornProviderAdmin::probe_candidate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProbeOutcome {
    pub ok: bool,
    pub models: Vec<String>,
    pub message: String,
}

/// Id of the tenant-pinned OpenRouter example overlay entry
/// `ironclaw_reborn_cli::commands::config::init::PROVIDERS_STUB` seeds into
/// a fresh `providers.json` — see [`RebornProviderAdmin::menu_entries`]'s
/// doc for why this is filtered off the numbered menu. Named constant (not
/// an inline literal) because the two crates aren't type-linked (the stub
/// is a raw JSON string literal); `menu_entries_excludes_the_example_overlay_provider`
/// pins them staying in sync.
pub const EXAMPLE_OVERLAY_PROVIDER_ID: &str = "acme-openrouter";

/// One entry on the interactive `onboard` numbered provider menu — see
/// [`RebornProviderAdmin::menu_entries`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderMenuEntry {
    pub id: String,
    pub display_name: String,
    /// A MENU-LEVEL value — see [`effective_api_key_required`]'s doc for
    /// why this can differ from the raw `providers.json` `api_key_required`
    /// field for `session_token`-kind providers (`nearai`).
    pub api_key_required: bool,
    pub description: String,
    pub aliases: Vec<String>,
}

/// Whether `definition` should be treated as requiring an API key for
/// reborn onboarding purposes — the value [`RebornProviderAdmin::menu_entries`]
/// puts in [`ProviderMenuEntry::api_key_required`], and what
/// [`RebornProviderAdmin::effective_api_key_required`] returns for a single
/// provider id (used by onboard's idempotent-rerun check, which must agree
/// with the menu on what "requires a key" means — see that method's doc).
///
/// A `session_token`-kind definition (`nearai`) is overridden to `true`
/// here even though the raw catalog field is `false`: session-token auth is
/// not wired in reborn (no `SessionRenewer` attaches at `serve` boot), so
/// nearai requires an API key (cloud-api.near.ai) exactly like every other
/// menu-eligible provider. Every other kind (`api_key`) passes its raw
/// `api_key_required` value through unchanged.
fn effective_api_key_required(definition: &ironclaw_llm::registry::ProviderDefinition) -> bool {
    if matches!(
        definition.setup.as_ref(),
        Some(ironclaw_llm::registry::SetupHint::SessionToken { .. })
    ) {
        return true;
    }
    definition.api_key_required
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RebornV1State {
    #[serde(rename = "not-used")]
    NotUsed,
}

impl RebornV1State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotUsed => "not-used",
        }
    }
}

impl fmt::Display for RebornV1State {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RebornModelRoutesState {
    #[serde(rename = "configured")]
    Configured,
    #[serde(rename = "not-configured")]
    NotConfigured,
}

impl RebornModelRoutesState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::NotConfigured => "not-configured",
        }
    }
}

impl fmt::Display for RebornModelRoutesState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum RebornProviderAdminError {
    #[error("load Reborn provider catalog `{}`: {reason}", path.display())]
    LoadRegistry { path: PathBuf, reason: String },
    #[error("load Reborn config `{}`: {source}", path.display())]
    LoadConfig {
        path: PathBuf,
        source: Box<ironclaw_reborn_config::RebornConfigFileError>,
    },
    #[error("unknown Reborn LLM provider `{provider}` in {}; available providers: {}", providers_file.display(), known.join(", "))]
    UnknownProvider {
        provider: String,
        providers_file: PathBuf,
        known: Vec<String>,
    },
    #[error("{reason}")]
    InvalidRequest { reason: String },
    #[error("update Reborn config `{}`: {source}", path.display())]
    UpdateConfig {
        path: PathBuf,
        source: Box<ironclaw_reborn_config::RebornConfigFileUpdateError>,
    },
    /// [`RebornProviderAdmin::detect_env_llm`]'s "partial env" outcome: some
    /// LLM environment configuration was present but incomplete or invalid.
    #[error("environment LLM configuration is incomplete: {source}")]
    EnvDetection {
        #[source]
        source: Box<ironclaw_llm::LlmError>,
    },
}

#[derive(Debug, Clone)]
struct ActiveLlmSelection {
    provider_id: Option<String>,
    canonical_provider_id: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    base_url: Option<String>,
}

/// Resolve which provider is *actually* active, mirroring the runtime's
/// precedence in [`crate::llm_admin::llm_catalog::resolve_reborn_runtime_llm`]:
/// the persisted `config.toml [llm.default]` slot first, then the same
/// environment fallback the chat-serving provider chain is built from
/// (`LLM_BACKEND`, Codex CLI auth, or a provider whose env vars are set).
///
/// Without the env fallback the Settings UI reported "no active provider"
/// (defaulting the display to `nearai`) whenever the live provider came from
/// the environment rather than an explicit selection — the inconsistency in
/// issue #4697.
fn active_llm_selection(
    config: Option<&RebornConfigFile>,
    registry: &ironclaw_llm::ProviderRegistry,
    providers_path: Option<&std::path::Path>,
) -> Option<ActiveLlmSelection> {
    if let Some(selection) = config.and_then(RebornConfigFile::default_llm_slot) {
        return Some(active_selection_from_slot(selection, registry));
    }
    active_selection_from_env(registry, providers_path)
}

/// Build the active selection from the environment-resolved provider, when the
/// config file carries no explicit `[llm.default]` slot.
fn active_selection_from_env(
    registry: &ironclaw_llm::ProviderRegistry,
    providers_path: Option<&std::path::Path>,
) -> Option<ActiveLlmSelection> {
    let resolved = match ironclaw_llm::resolve_provider_config_from_env(providers_path) {
        Ok(resolved) => resolved?,
        Err(error) => {
            tracing::debug!(%error, "active provider env resolution failed; reporting none");
            return None;
        }
    };
    Some(active_selection_from_resolved(&resolved, registry))
}

/// Map an environment-resolved provider onto an [`ActiveLlmSelection`].
///
/// Pure mapping (no env / IO) so the field translation — canonical id lookup
/// and empty-string-to-`None` normalization — is unit-testable directly.
fn active_selection_from_resolved(
    resolved: &ironclaw_llm::ResolvedProviderConfig,
    registry: &ironclaw_llm::ProviderRegistry,
) -> ActiveLlmSelection {
    let provider_id = resolved.provider_id().to_string();
    let canonical_provider_id = registry.find(&provider_id).map(|def| def.id.clone());
    let model = resolved.model();
    let base_url = resolved.base_url();
    ActiveLlmSelection {
        provider_id: Some(provider_id),
        canonical_provider_id,
        model: (!model.is_empty()).then(|| model.to_string()),
        api_key_env: None,
        base_url: (!base_url.is_empty()).then(|| base_url.to_string()),
    }
}

fn active_selection_from_slot(
    selection: &LlmSlotSelection,
    registry: &ironclaw_llm::ProviderRegistry,
) -> ActiveLlmSelection {
    let canonical_provider_id = selection
        .provider_id
        .as_deref()
        .and_then(|provider_id| registry.find(provider_id))
        .map(|def| def.id.clone());
    ActiveLlmSelection {
        provider_id: selection.provider_id.clone(),
        canonical_provider_id,
        model: selection.model.clone(),
        api_key_env: selection.api_key_env.clone(),
        base_url: selection.base_url.clone(),
    }
}

fn unique_provider_definitions(
    registry: &ironclaw_llm::ProviderRegistry,
) -> Vec<&ironclaw_llm::registry::ProviderDefinition> {
    let mut emitted = std::collections::HashSet::new();
    registry
        .all()
        .iter()
        .filter_map(|candidate| {
            let final_def = registry.find(&candidate.id)?;
            if emitted.insert(final_def.id.as_str()) {
                Some(final_def)
            } else {
                None
            }
        })
        .collect()
}

fn known_provider_ids(registry: &ironclaw_llm::ProviderRegistry) -> Vec<String> {
    unique_provider_definitions(registry)
        .into_iter()
        .map(|def| def.id.clone())
        .collect()
}

fn provider_info(
    def: &ironclaw_llm::registry::ProviderDefinition,
    active: Option<&ActiveLlmSelection>,
    verbose: bool,
) -> RebornProviderInfo {
    let active_for_provider = active
        .and_then(|selection| selection.canonical_provider_id.as_deref())
        .is_some_and(|provider_id| provider_id.eq_ignore_ascii_case(&def.id));
    let active_model = active_for_provider.then(|| {
        active
            .and_then(|selection| selection.model.clone())
            .unwrap_or_else(|| def.default_model.clone())
    });
    RebornProviderInfo {
        id: def.id.clone(),
        description: def.description.clone(),
        default_model: def.default_model.clone(),
        active: active_for_provider,
        active_model,
        metadata: verbose.then(|| RebornProviderMetadata {
            aliases: def.aliases.clone(),
            protocol: provider_protocol_wire_name(def.protocol),
            model_env: def.model_env.clone(),
            api_key_env: def.api_key_env.clone(),
            api_key_required: def.api_key_required,
            base_url: def.default_base_url.clone(),
            credential_kind: def.setup.as_ref().map(|setup| setup.kind()),
            accepts_api_key: def.api_key_env.is_some()
                || def
                    .setup
                    .as_ref()
                    .is_some_and(ironclaw_llm::registry::SetupHint::accepts_api_key),
            can_list_models: def
                .setup
                .as_ref()
                .is_some_and(ironclaw_llm::registry::SetupHint::can_list_models),
        }),
    }
}

fn provider_protocol_wire_name(protocol: ironclaw_llm::registry::ProviderProtocol) -> String {
    serde_json::to_value(protocol)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Base URL to probe for a not-yet-persisted candidate provider.
///
/// `providers.json` leaves `default_base_url` unset for protocols whose
/// default lives in code rather than the catalog (today: only `nearai`,
/// which defaults to the cloud NEAR AI endpoint — see
/// [`ironclaw_llm::default_nearai_base_url`]). Passing `None` straight
/// through here would make the resolver derive an empty base URL for those
/// protocols. Every other protocol either carries its own
/// `default_base_url` in the catalog or (Bedrock, GeminiOauth,
/// OpenAiCodex) never consumes `base_url` at all, so they need no
/// fallback here.
fn candidate_probe_base_url(
    definition: &ironclaw_llm::registry::ProviderDefinition,
) -> Option<String> {
    if let Some(base_url) = definition.default_base_url.clone() {
        return Some(base_url);
    }
    if definition.protocol == ironclaw_llm::registry::ProviderProtocol::NearAi {
        return Some(ironclaw_llm::default_nearai_base_url(
            ironclaw_common::env_helpers::env_or_override("NEARAI_BASE_URL"),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_llm::{
        ProviderProtocol, ProviderRegistry, ResolvedDedicatedProviderConfig, ResolvedProviderConfig,
    };

    fn dedicated(provider_id: &str, model: &str, base_url: &str) -> ResolvedProviderConfig {
        ResolvedProviderConfig::Dedicated(ResolvedDedicatedProviderConfig {
            protocol: ProviderProtocol::OpenAiCompletions,
            provider_id: provider_id.to_string(),
            api_key: None,
            base_url: base_url.to_string(),
            model: model.to_string(),
        })
    }

    #[test]
    fn env_resolved_known_provider_maps_to_active_selection() {
        let registry = ProviderRegistry::try_load_from_path(None).expect("builtin registry");
        let resolved = dedicated("openai", "gpt-4o", "https://api.openai.com/v1");
        let active = active_selection_from_resolved(&resolved, &registry);
        assert_eq!(active.provider_id.as_deref(), Some("openai"));
        assert_eq!(active.canonical_provider_id.as_deref(), Some("openai"));
        assert_eq!(active.model.as_deref(), Some("gpt-4o"));
        assert_eq!(
            active.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
    }

    #[test]
    fn env_resolved_empty_model_and_base_url_become_none() {
        let registry = ProviderRegistry::try_load_from_path(None).expect("builtin registry");
        let resolved = dedicated("openai", "", "");
        let active = active_selection_from_resolved(&resolved, &registry);
        assert_eq!(active.provider_id.as_deref(), Some("openai"));
        assert!(active.model.is_none());
        assert!(active.base_url.is_none());
    }

    #[test]
    fn env_resolved_unknown_provider_has_no_canonical_id() {
        let registry = ProviderRegistry::try_load_from_path(None).expect("builtin registry");
        let resolved = dedicated("not-a-real-provider", "m", "");
        let active = active_selection_from_resolved(&resolved, &registry);
        assert_eq!(active.provider_id.as_deref(), Some("not-a-real-provider"));
        assert!(active.canonical_provider_id.is_none());
    }

    #[test]
    fn active_selection_prefers_config_slot_over_env() {
        use std::collections::BTreeMap;
        let registry = ProviderRegistry::try_load_from_path(None).expect("builtin registry");
        let config = RebornConfigFile {
            llm: Some(BTreeMap::from([(
                "default".to_string(),
                LlmSlotSelection {
                    provider_id: Some("anthropic".to_string()),
                    model: Some("claude-pinned-by-config".to_string()),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };
        // An explicit [llm.default] slot is authoritative: the selection comes
        // from the slot and never consults the environment. The distinctive
        // pinned model could not come from env resolution, so a regression
        // dropping the early return (falling through to env) would lose it.
        let active =
            active_llm_selection(Some(&config), &registry, None).expect("slot selection present");
        assert_eq!(active.provider_id.as_deref(), Some("anthropic"));
        assert_eq!(active.model.as_deref(), Some("claude-pinned-by-config"));
    }

    fn test_admin() -> RebornProviderAdmin {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = ironclaw_reborn_config::RebornHome::resolve_from_env_parts(
            Some(temp.path().join("reborn-home").as_os_str().to_os_string()),
            None,
            None,
        )
        .expect("valid reborn home");
        // Leak the tempdir so the on-disk stub outlives this function; only
        // needs to exist as a valid, empty root (nothing is ever written).
        std::mem::forget(temp);
        RebornProviderAdmin::new(RebornBootConfig::new(
            home,
            ironclaw_reborn_config::RebornProfile::LocalDev,
        ))
    }

    #[test]
    fn menu_entries_lists_nearai_first_and_requires_an_api_key() {
        let admin = test_admin();
        let entries = admin.menu_entries().expect("menu entries load");
        let first = entries.first().expect("at least one menu entry");
        assert_eq!(
            first.id, "nearai",
            "nearai must be menu item 1: {entries:?}"
        );
        // Menu-level override: no session-token auth wired in reborn, so
        // nearai requires a key here despite the raw catalog entry.
        assert!(
            first.api_key_required,
            "nearai must require an API key on the reborn onboard menu (no session-token auth \
             wired): {first:?}"
        );
    }

    /// `effective_api_key_required` must agree with `menu_entries`'s
    /// override for `nearai` (`true`, not raw catalog `false`), pass
    /// `openai`'s raw value through, and return `None` for an unknown id.
    #[test]
    fn effective_api_key_required_overrides_session_token_providers() {
        let admin = test_admin();
        assert_eq!(
            admin
                .effective_api_key_required("nearai")
                .expect("nearai known"),
            Some(true)
        );
        assert_eq!(
            admin
                .effective_api_key_required("openai")
                .expect("openai known"),
            Some(true)
        );
        assert_eq!(
            admin
                .effective_api_key_required("not-a-real-provider")
                .expect("lookup succeeds even when unknown"),
            None
        );
    }

    #[test]
    fn menu_entries_excludes_non_menu_setup_kinds() {
        let admin = test_admin();
        let entries = admin.menu_entries().expect("menu entries load");
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        for excluded in [
            "ollama",
            "bedrock",
            "gemini_oauth",
            "openai_codex",
            "github_copilot",
            "openai_compatible",
            "cloudflare",
        ] {
            assert!(
                !ids.contains(&excluded),
                "{excluded} must be excluded from the onboard menu: {ids:?}"
            );
        }
    }

    /// `openai_compatible` requires a base URL the numbered menu never
    /// prompts for; selecting it would "succeed" at onboard time and fail
    /// `serve` boot with `LLM_BASE_URL` unset. Pinned separately from the
    /// scope exclusions above — this is a correctness bug, not scope.
    #[test]
    fn menu_entries_excludes_openai_compatible_base_url_trap() {
        let admin = test_admin();
        let entries = admin.menu_entries().expect("menu entries load");
        assert!(
            entries.iter().all(|entry| entry.id != "openai_compatible"),
            "openai_compatible must never appear on the onboard menu: {entries:?}"
        );
    }

    #[test]
    fn menu_entries_populate_aliases() {
        let admin = test_admin();
        let entries = admin.menu_entries().expect("menu entries load");
        let github_copilot_absent = entries.iter().find(|entry| entry.id == "github_copilot");
        assert!(github_copilot_absent.is_none());
        let openai = entries
            .iter()
            .find(|entry| entry.id == "openai")
            .expect("openai present on menu");
        assert!(
            !openai.aliases.is_empty(),
            "openai should carry its registry aliases: {openai:?}"
        );
    }

    /// The tenant-pinned OpenRouter example overlay entry (same shape
    /// `PROVIDERS_STUB` writes: id `acme-openrouter`, kind `api_key`,
    /// otherwise indistinguishable from a real menu-eligible provider) must
    /// never appear on the numbered menu. Pins the id-equality filter
    /// against [`EXAMPLE_OVERLAY_PROVIDER_ID`].
    #[test]
    fn menu_entries_excludes_the_example_overlay_provider() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = ironclaw_reborn_config::RebornHome::resolve_from_env_parts(
            Some(temp.path().join("reborn-home").as_os_str().to_os_string()),
            None,
            None,
        )
        .expect("valid reborn home");
        std::fs::create_dir_all(home.path()).expect("create reborn home dir");

        // Built from the REAL `ironclaw_reborn_cli::commands::config::init::
        // PROVIDERS_STUB` JSON (not a hand-typed duplicate) so this test
        // catches drift between that stub's id and
        // `EXAMPLE_OVERLAY_PROVIDER_ID` instead of two disjoint fixtures
        // agreeing by coincidence.
        let stub_definitions: Vec<ironclaw_llm::registry::ProviderDefinition> =
            serde_json::from_str(providers_stub_json()).expect("PROVIDERS_STUB must parse as JSON");
        assert_eq!(
            stub_definitions.len(),
            1,
            "this test assumes PROVIDERS_STUB seeds exactly one overlay entry: {stub_definitions:?}"
        );
        let overlay_definition = stub_definitions.into_iter().next().expect("checked above");
        assert_eq!(
            overlay_definition.id, EXAMPLE_OVERLAY_PROVIDER_ID,
            "PROVIDERS_STUB's overlay id has drifted from EXAMPLE_OVERLAY_PROVIDER_ID"
        );
        crate::ProviderRepo::new(home.providers_file_path())
            .upsert(overlay_definition)
            .expect("write example overlay");

        let admin = RebornProviderAdmin::new(RebornBootConfig::new(
            home,
            ironclaw_reborn_config::RebornProfile::LocalDev,
        ));
        let entries = admin.menu_entries().expect("menu entries load");
        assert!(
            entries
                .iter()
                .all(|entry| entry.id != EXAMPLE_OVERLAY_PROVIDER_ID),
            "the tenant-pinned OpenRouter example overlay must never appear on the onboard \
             menu: {entries:?}"
        );
    }

    /// Extract the raw JSON text of `PROVIDERS_STUB` from
    /// `ironclaw_reborn_cli::commands::config::init`'s source, via
    /// `include_str!` — composition can't depend on `ironclaw_reborn_cli`
    /// (only the reverse), so this reads the file text directly rather than
    /// duplicating the JSON literal, keeping the fixture used above tied to
    /// the actual stub `config init`/`onboard` write.
    fn providers_stub_json() -> &'static str {
        const INIT_RS: &str =
            include_str!("../../../ironclaw_reborn_cli/src/commands/config/init.rs");
        const START_MARKER: &str = "const PROVIDERS_STUB: &str = r#\"";
        let start = INIT_RS.find(START_MARKER).unwrap_or_else(|| {
            panic!(
                "PROVIDERS_STUB definition not found in ironclaw_reborn_cli's init.rs — this \
                 test's extraction marker has drifted from the real source"
            )
        }) + START_MARKER.len();
        let end = INIT_RS[start..]
            .find("\"#;")
            .expect("PROVIDERS_STUB closing delimiter `\"#;` not found");
        &INIT_RS[start..start + end]
    }

    /// `detect_env_llm` must return `Ok(None)` with no LLM env vars set —
    /// the fresh-onboard case that must fall through to the full menu.
    /// The `Ok(Some(_))`/`Err(_)` branches can't be covered in-process
    /// (`forbid(unsafe_code)` blocks `set_var`); covered at the CLI smoke
    /// tier instead via `Command::env` on a real child process.
    #[test]
    fn detect_env_llm_is_none_with_no_llm_env_vars_set() {
        let admin = test_admin();
        let detected = admin
            .detect_env_llm()
            .expect("detection must not error with a clean environment");
        assert!(
            detected.is_none(),
            "detect_env_llm must report no detection with no LLM env vars set: {detected:?}"
        );
    }

    /// `nearai`'s catalog entry carries no `default_base_url` — its default
    /// lives in code and is now unconditionally the cloud endpoint (no more
    /// has-key branch to thread through the probe). A candidate probe must
    /// resolve to the cloud endpoint, not `None`/empty, or the resolver
    /// falls through to an empty base URL and every probe reports "could
    /// not reach the provider endpoint".
    #[test]
    fn candidate_probe_base_url_defaults_nearai_to_cloud() {
        let registry = ProviderRegistry::try_load_from_path(None).expect("builtin registry");
        let nearai = registry.find("nearai").expect("nearai in builtin registry");
        assert_eq!(
            nearai.default_base_url, None,
            "fixture assumption: nearai's catalog entry has no default_base_url"
        );

        let base_url = candidate_probe_base_url(nearai);

        assert_eq!(
            base_url.as_deref(),
            Some(ironclaw_llm::NEARAI_CLOUD_DEFAULT_BASE_URL),
            "a nearai probe must target the cloud endpoint, got {base_url:?}"
        );
    }

    /// Every other builtin provider either carries its own
    /// `default_base_url` (openai, anthropic, ollama, …) or never consumes
    /// `base_url` at all (bedrock, gemini_oauth, openai_codex) — none of
    /// them should gain a synthesized fallback here.
    #[test]
    fn candidate_probe_base_url_only_special_cases_nearai() {
        let registry = ProviderRegistry::try_load_from_path(None).expect("builtin registry");
        for definition in unique_provider_definitions(&registry) {
            if definition.protocol == ProviderProtocol::NearAi {
                continue;
            }
            assert_eq!(
                candidate_probe_base_url(definition),
                definition.default_base_url.clone(),
                "provider `{}` must not gain a synthesized probe base URL",
                definition.id
            );
        }
    }

    // `probe_candidate`'s live-stub tests (`spawn_models_stub`,
    // `write_stub_provider`, and the two probe tests) moved to
    // `tests/provider_admin_probe.rs`: the architecture boundary test
    // `reborn_product_api_crates_do_not_bind_http_ingress` greps every
    // `.rs` file under this crate's `src/` for a loopback-listener bind
    // call with no `#[cfg(test)]` awareness (by design — it's a text
    // scan, not a compile-aware check), so an in-module stub server
    // trips it even though it never runs in production. `tests/` sits
    // outside the scanned roots; `webui_v2_serve.rs` already binds a
    // loopback listener there for the same reason.
}
