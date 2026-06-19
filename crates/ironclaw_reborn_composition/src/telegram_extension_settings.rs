use std::sync::Arc;

use chrono::Utc;
use ironclaw_extensions::ExtensionInstallationId;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use ironclaw_product_adapters::AdapterInstallationId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::telegram_host_beta::TelegramHostBetaConfig;

const TELEGRAM_EXTENSION_SETTINGS_ROOT: &str = "/tenant-shared/telegram-extension-installations";

/// Host-owned, non-secret Telegram installation settings persisted alongside the
/// enabled extension. Secret VALUES (bot token, webhook secret) never live here —
/// only the host-resolved identity needed to rebuild the runtime config; secrets
/// are stored in the secret store and referenced by `ExtensionCredentialBinding`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TelegramExtensionInstallationSettings {
    pub(crate) tenant_id: TenantId,
    pub(crate) user_id: UserId,
    pub(crate) agent_id: AgentId,
    pub(crate) project_id: Option<ProjectId>,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) shared_subject_user_id: Option<UserId>,
    pub(crate) bot_username: String,
    pub(crate) bot_user_id: i64,
    pub(crate) recognized_commands: Vec<String>,
    pub(crate) progress_push_enabled: bool,
}

impl TelegramExtensionInstallationSettings {
    pub(crate) fn from_host_beta_config(config: &TelegramHostBetaConfig) -> Result<Self, Error> {
        Ok(Self {
            tenant_id: config.tenant_id.clone(),
            user_id: config.user_id.clone(),
            agent_id: config.agent_id.clone(),
            project_id: config.project_id.clone(),
            adapter_installation_id: config.installation_id.clone(),
            shared_subject_user_id: config.shared_subject_user_id.clone(),
            bot_username: config.bot_username.clone(),
            bot_user_id: config.bot_user_id,
            recognized_commands: config.recognized_commands.clone(),
            progress_push_enabled: config.progress_push_enabled,
        })
    }

    pub(crate) fn secret_scope(&self) -> ResourceScope {
        ResourceScope {
            tenant_id: self.tenant_id.clone(),
            user_id: self.user_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            project_id: self.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}

pub(crate) struct FilesystemTelegramExtensionSettingsStore<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemTelegramExtensionSettingsStore<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    pub(crate) async fn upsert(
        &self,
        installation_id: &ExtensionInstallationId,
        settings: &TelegramExtensionInstallationSettings,
    ) -> Result<(), Error> {
        let path = settings_path(installation_id)?;
        let body =
            serde_json::to_vec_pretty(&StoredTelegramExtensionInstallationSettings::from(settings))
                .map_err(|error| Error::Invalid {
                    reason: format!("Telegram extension settings could not be serialized: {error}"),
                })?;
        self.filesystem
            .put(
                &ResourceScope::system(),
                &path,
                Entry::bytes(body).with_content_type(ContentType::json()),
                CasExpectation::Any,
            )
            .await
            .map_err(map_fs_error)?;
        Ok(())
    }

    pub(crate) async fn get(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<TelegramExtensionInstallationSettings>, Error> {
        let path = settings_path(installation_id)?;
        let Some(versioned) = self
            .filesystem
            .get(&ResourceScope::system(), &path)
            .await
            .map_err(map_fs_error)?
        else {
            return Ok(None);
        };
        let stored: StoredTelegramExtensionInstallationSettings =
            serde_json::from_slice(&versioned.entry.body).map_err(|error| Error::Invalid {
                reason: format!("stored Telegram extension settings are invalid JSON: {error}"),
            })?;
        stored.into_settings().map(Some)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTelegramExtensionInstallationSettings {
    tenant_id: String,
    user_id: String,
    agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    adapter_installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shared_subject_user_id: Option<String>,
    bot_username: String,
    // Telegram bot/user ids are i64; stored as a string for stable round-trip.
    bot_user_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    recognized_commands: Vec<String>,
    #[serde(default)]
    progress_push_enabled: bool,
    updated_at: chrono::DateTime<Utc>,
}

impl From<&TelegramExtensionInstallationSettings> for StoredTelegramExtensionInstallationSettings {
    fn from(settings: &TelegramExtensionInstallationSettings) -> Self {
        Self {
            tenant_id: settings.tenant_id.as_str().to_string(),
            user_id: settings.user_id.as_str().to_string(),
            agent_id: settings.agent_id.as_str().to_string(),
            project_id: settings
                .project_id
                .as_ref()
                .map(|project_id| project_id.as_str().to_string()),
            adapter_installation_id: settings.adapter_installation_id.as_str().to_string(),
            shared_subject_user_id: settings
                .shared_subject_user_id
                .as_ref()
                .map(|user_id| user_id.as_str().to_string()),
            bot_username: settings.bot_username.clone(),
            bot_user_id: settings.bot_user_id.to_string(),
            recognized_commands: settings.recognized_commands.clone(),
            progress_push_enabled: settings.progress_push_enabled,
            updated_at: Utc::now(),
        }
    }
}

impl StoredTelegramExtensionInstallationSettings {
    fn into_settings(self) -> Result<TelegramExtensionInstallationSettings, Error> {
        Ok(TelegramExtensionInstallationSettings {
            tenant_id: TenantId::new(self.tenant_id).map_err(invalid_id("tenant_id"))?,
            user_id: UserId::new(self.user_id).map_err(invalid_id("user_id"))?,
            agent_id: AgentId::new(self.agent_id).map_err(invalid_id("agent_id"))?,
            project_id: self
                .project_id
                .map(ProjectId::new)
                .transpose()
                .map_err(invalid_id("project_id"))?,
            adapter_installation_id: AdapterInstallationId::new(self.adapter_installation_id)
                .map_err(invalid_id("adapter_installation_id"))?,
            shared_subject_user_id: self
                .shared_subject_user_id
                .map(UserId::new)
                .transpose()
                .map_err(invalid_id("shared_subject_user_id"))?,
            bot_username: nonempty("bot_username", self.bot_username)?,
            bot_user_id: self
                .bot_user_id
                .parse::<i64>()
                .map_err(|error| Error::Invalid {
                    reason: format!("bot_user_id is invalid: {error}"),
                })?,
            recognized_commands: self.recognized_commands,
            progress_push_enabled: self.progress_push_enabled,
        })
    }
}

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("invalid Telegram extension settings: {reason}")]
    Invalid { reason: String },
    #[error("Telegram extension settings store is unavailable: {reason}")]
    StoreUnavailable { reason: String },
}

fn settings_path(installation_id: &ExtensionInstallationId) -> Result<ScopedPath, Error> {
    ScopedPath::new(format!(
        "{}/{}.json",
        TELEGRAM_EXTENSION_SETTINGS_ROOT,
        path_segment(installation_id.as_str())
    ))
    .map_err(|error| Error::Invalid {
        reason: format!("Telegram extension settings path is invalid: {error}"),
    })
}

fn path_segment(value: &str) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn nonempty(field: &'static str, value: String) -> Result<String, Error> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(Error::Invalid {
            reason: format!("{field} must be non-empty and trimmed"),
        });
    }
    Ok(value)
}

fn invalid_id<E: std::fmt::Display>(field: &'static str) -> impl FnOnce(E) -> Error {
    move |error| Error::Invalid {
        reason: format!("{field} is invalid: {error}"),
    }
}

fn map_fs_error(error: FilesystemError) -> Error {
    Error::StoreUnavailable {
        reason: match error {
            FilesystemError::BackendInfrastructure { reason, .. } => reason,
            other => other.to_string(),
        },
    }
}
