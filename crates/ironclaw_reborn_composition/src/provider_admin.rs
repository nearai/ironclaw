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
        let active = active_llm_selection(config.as_ref(), &registry);

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
        let active = active_llm_selection(config.as_ref(), &registry);
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
}

#[derive(Debug, Clone)]
struct ActiveLlmSelection {
    provider_id: Option<String>,
    canonical_provider_id: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    base_url: Option<String>,
}

fn active_llm_selection(
    config: Option<&RebornConfigFile>,
    registry: &ironclaw_llm::ProviderRegistry,
) -> Option<ActiveLlmSelection> {
    let selection = config.and_then(RebornConfigFile::default_llm_slot)?;
    Some(active_selection_from_slot(selection, registry))
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
