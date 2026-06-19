use std::sync::Arc;

use chrono::Utc;
use ironclaw_extensions::ExtensionInstallationId;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use ironclaw_product_adapters::{AdapterInstallationId, ExternalActorRef};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::slack_host_beta::{SlackHostBetaChannelRoute, SlackHostBetaConfig};
use crate::slack_serve::SlackTeamId;

const SLACK_EXTENSION_SETTINGS_ROOT: &str = "/tenant-shared/slack-extension-installations";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackExtensionInstallationSettings {
    pub(crate) tenant_id: TenantId,
    pub(crate) user_id: UserId,
    pub(crate) agent_id: AgentId,
    pub(crate) project_id: Option<ProjectId>,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) team_id: SlackTeamId,
    pub(crate) api_app_id: String,
    pub(crate) slack_user_id: Option<String>,
    pub(crate) shared_subject_user_id: Option<UserId>,
    pub(crate) channel_routes: Vec<SlackHostBetaChannelRoute>,
}

impl SlackExtensionInstallationSettings {
    pub(crate) fn from_host_beta_config(config: &SlackHostBetaConfig) -> Result<Self, Error> {
        let api_app_id = match &config.installation_selector {
            crate::slack_serve::SlackInstallationSelector::AppTeam { api_app_id, .. } => {
                api_app_id.as_str().to_string()
            }
            _ => {
                return Err(Error::Invalid {
                    reason: "Slack extension settings require an app/team selector".to_string(),
                });
            }
        };
        Ok(Self {
            tenant_id: config.tenant_id.clone(),
            user_id: config.user_id.clone(),
            agent_id: config.agent_id.clone(),
            project_id: config.project_id.clone(),
            adapter_installation_id: config.installation_id.clone(),
            team_id: config.team_id.clone(),
            api_app_id,
            slack_user_id: config
                .slack_actor
                .as_ref()
                .map(|actor| actor.id().to_string()),
            shared_subject_user_id: config.shared_subject_user_id.clone(),
            channel_routes: config.channel_routes.clone(),
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

    pub(crate) fn slack_actor(&self) -> Result<Option<ExternalActorRef>, Error> {
        self.slack_user_id
            .as_ref()
            .map(|slack_user_id| {
                ExternalActorRef::new(
                    ironclaw_slack_v2_adapter::SLACK_USER_ACTOR_KIND,
                    slack_user_id.clone(),
                    None::<String>,
                )
                .map_err(|reason| Error::Invalid {
                    reason: format!("stored Slack user id is invalid: {reason}"),
                })
            })
            .transpose()
    }
}

pub(crate) struct FilesystemSlackExtensionSettingsStore<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemSlackExtensionSettingsStore<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    pub(crate) async fn upsert(
        &self,
        installation_id: &ExtensionInstallationId,
        settings: &SlackExtensionInstallationSettings,
    ) -> Result<(), Error> {
        let path = settings_path(installation_id)?;
        let body =
            serde_json::to_vec_pretty(&StoredSlackExtensionInstallationSettings::from(settings))
                .map_err(|error| Error::Invalid {
                    reason: format!("Slack extension settings could not be serialized: {error}"),
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
    ) -> Result<Option<SlackExtensionInstallationSettings>, Error> {
        let path = settings_path(installation_id)?;
        let Some(versioned) = self
            .filesystem
            .get(&ResourceScope::system(), &path)
            .await
            .map_err(map_fs_error)?
        else {
            return Ok(None);
        };
        let stored: StoredSlackExtensionInstallationSettings =
            serde_json::from_slice(&versioned.entry.body).map_err(|error| Error::Invalid {
                reason: format!("stored Slack extension settings are invalid JSON: {error}"),
            })?;
        stored.into_settings().map(Some)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSlackExtensionInstallationSettings {
    tenant_id: String,
    user_id: String,
    agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    adapter_installation_id: String,
    team_id: String,
    api_app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slack_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shared_subject_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    channel_routes: Vec<StoredSlackChannelRoute>,
    updated_at: chrono::DateTime<Utc>,
}

impl From<&SlackExtensionInstallationSettings> for StoredSlackExtensionInstallationSettings {
    fn from(settings: &SlackExtensionInstallationSettings) -> Self {
        Self {
            tenant_id: settings.tenant_id.as_str().to_string(),
            user_id: settings.user_id.as_str().to_string(),
            agent_id: settings.agent_id.as_str().to_string(),
            project_id: settings
                .project_id
                .as_ref()
                .map(|project_id| project_id.as_str().to_string()),
            adapter_installation_id: settings.adapter_installation_id.as_str().to_string(),
            team_id: settings.team_id.as_str().to_string(),
            api_app_id: settings.api_app_id.clone(),
            slack_user_id: settings.slack_user_id.clone(),
            shared_subject_user_id: settings
                .shared_subject_user_id
                .as_ref()
                .map(|user_id| user_id.as_str().to_string()),
            channel_routes: settings
                .channel_routes
                .iter()
                .map(StoredSlackChannelRoute::from)
                .collect(),
            updated_at: Utc::now(),
        }
    }
}

impl StoredSlackExtensionInstallationSettings {
    fn into_settings(self) -> Result<SlackExtensionInstallationSettings, Error> {
        Ok(SlackExtensionInstallationSettings {
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
            team_id: SlackTeamId::new(self.team_id),
            api_app_id: nonempty("api_app_id", self.api_app_id)?,
            slack_user_id: self
                .slack_user_id
                .map(|value| nonempty("slack_user_id", value))
                .transpose()?,
            shared_subject_user_id: self
                .shared_subject_user_id
                .map(UserId::new)
                .transpose()
                .map_err(invalid_id("shared_subject_user_id"))?,
            channel_routes: self
                .channel_routes
                .into_iter()
                .map(StoredSlackChannelRoute::into_route)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSlackChannelRoute {
    channel_id: String,
    subject_user_id: String,
}

impl From<&SlackHostBetaChannelRoute> for StoredSlackChannelRoute {
    fn from(route: &SlackHostBetaChannelRoute) -> Self {
        Self {
            channel_id: route.channel_id.clone(),
            subject_user_id: route.subject_user_id.as_str().to_string(),
        }
    }
}

impl StoredSlackChannelRoute {
    fn into_route(self) -> Result<SlackHostBetaChannelRoute, Error> {
        Ok(SlackHostBetaChannelRoute::new(
            nonempty("channel_id", self.channel_id)?,
            UserId::new(self.subject_user_id).map_err(invalid_id("subject_user_id"))?,
        ))
    }
}

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("invalid Slack extension settings: {reason}")]
    Invalid { reason: String },
    #[error("Slack extension settings store is unavailable: {reason}")]
    StoreUnavailable { reason: String },
}

fn settings_path(installation_id: &ExtensionInstallationId) -> Result<ScopedPath, Error> {
    ScopedPath::new(format!(
        "{}/{}.json",
        SLACK_EXTENSION_SETTINGS_ROOT,
        path_segment(installation_id.as_str())
    ))
    .map_err(|error| Error::Invalid {
        reason: format!("Slack extension settings path is invalid: {error}"),
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
