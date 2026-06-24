use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use ironclaw_extensions::ExtensionManifestRecord;
use ironclaw_host_api::ingress::IngressRouteDescriptor;
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use ironclaw_host_ingress_registry::{HostIngressRuntimeEntry, list_enabled_host_ingress_entries};
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ConnectableChannelsProductFacade, RebornChannelConnectAction, RebornChannelConnectStrategy,
    RebornConnectableChannelInfo, StaticConnectableChannelsProductFacade,
};
use serde::Deserialize;

use crate::webui::build_webui_services_with_connectable_channels;
use crate::webui_serve::{PublicRouteMount, WebuiServeConfig};
use crate::{RebornBuildError, RebornRuntime, RebornWebuiBundle};

#[cfg(feature = "slack-v2-host-beta")]
use crate::slack_channel_routes::{
    SlackChannelRouteAdminRouteConfig, slack_channel_route_admin_descriptors,
};
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack_host_beta::{
    SlackHostBetaBuildError, SlackHostBetaConfig,
    build_slack_events_host_ingress_mount_from_entries, build_slack_host_beta_mounts,
};
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack_host_ingress::SLACK_EVENTS_HOST_INGRESS_ROUTE_ID;
#[cfg(feature = "slack-v2-host-beta")]
use crate::slack_personal_binding_pairing_serve::{
    SlackPersonalBindingPairingRouteConfig, slack_personal_binding_pairing_route_descriptors,
};
#[cfg(feature = "telegram-v2-host-beta")]
use crate::telegram_host_beta::{
    TelegramHostBetaBuildError, build_telegram_updates_host_ingress_mount_from_entries,
};
#[cfg(feature = "telegram-v2-host-beta")]
use crate::telegram_host_ingress::TELEGRAM_UPDATES_HOST_INGRESS_ROUTE_ID;

const HOST_INGRESS_LOG_TARGET: &str = "ironclaw::reborn::host_ingress_serve_plan";

#[cfg(feature = "slack-v2-host-beta")]
const SLACK_CONFIG_IMPORT_MISSING_ROUTE: &str =
    "Slack config import did not produce an enabled Slack extension events route";
#[cfg(feature = "telegram-v2-host-beta")]
const TELEGRAM_CONFIG_IMPORT_MISSING_ROUTE: &str =
    "Telegram config import did not produce an enabled Telegram extension updates route";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfiguredHostIngressProjectionMode {
    Suppress,
    ValidateOnly,
    Serve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostIngressOperatorRouteVisibility {
    Hidden,
    Visible,
}

#[derive(Default)]
pub struct HostIngressServePlanInput {
    #[cfg(feature = "slack-v2-host-beta")]
    slack: Option<SlackHostIngressServeInput>,
    #[cfg(feature = "telegram-v2-host-beta")]
    telegram: Option<TelegramHostIngressServeInput>,
}

impl HostIngressServePlanInput {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "slack-v2-host-beta")]
    pub fn with_slack_host_beta(
        mut self,
        config: SlackHostBetaConfig,
        projection_mode: ConfiguredHostIngressProjectionMode,
        operator_route_visibility: HostIngressOperatorRouteVisibility,
    ) -> Self {
        self.slack = Some(SlackHostIngressServeInput {
            config,
            projection_mode,
            operator_route_visibility,
        });
        self
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    pub fn with_telegram_host_beta(
        mut self,
        projection_mode: ConfiguredHostIngressProjectionMode,
    ) -> Self {
        self.telegram = Some(TelegramHostIngressServeInput { projection_mode });
        self
    }
}

#[cfg(feature = "slack-v2-host-beta")]
struct SlackHostIngressServeInput {
    config: SlackHostBetaConfig,
    projection_mode: ConfiguredHostIngressProjectionMode,
    operator_route_visibility: HostIngressOperatorRouteVisibility,
}

#[cfg(feature = "telegram-v2-host-beta")]
struct TelegramHostIngressServeInput {
    projection_mode: ConfiguredHostIngressProjectionMode,
}

#[derive(Clone, Default)]
pub struct HostIngressServePlan {
    public_route_mounts: Vec<PublicRouteMount>,
    connectable_channels: Vec<RebornConnectableChannelInfo>,
    requires_outbound_delivery_target_provider: bool,
    #[cfg(feature = "slack-v2-host-beta")]
    slack_personal_binding_pairing: Option<SlackPersonalBindingPairingRouteConfig>,
    #[cfg(feature = "slack-v2-host-beta")]
    slack_channel_routes: Option<SlackChannelRouteAdminRouteConfig>,
}

impl HostIngressServePlan {
    pub fn public_route_mounts(&self) -> &[PublicRouteMount] {
        &self.public_route_mounts
    }

    pub fn connectable_channels(&self) -> &[RebornConnectableChannelInfo] {
        &self.connectable_channels
    }

    pub fn public_route_ids(&self) -> Vec<String> {
        let mut route_ids = Vec::new();
        for mount in &self.public_route_mounts {
            route_ids.extend(mount_route_ids(mount));
        }
        route_ids
    }

    pub fn apply_to_webui_serve_config(&self, mut config: WebuiServeConfig) -> WebuiServeConfig {
        for mount in &self.public_route_mounts {
            config = config.with_public_route_mount(mount.clone());
        }
        #[cfg(feature = "slack-v2-host-beta")]
        {
            if let Some(pairing) = &self.slack_personal_binding_pairing {
                config = config.with_slack_personal_binding_pairing(pairing.clone());
            }
            if let Some(channel_routes) = &self.slack_channel_routes {
                config = config.with_slack_channel_routes(channel_routes.clone());
            }
        }
        config
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostIngressServePlanError {
    #[error("host-ingress projection requires durable host state")]
    DurableHostStateUnavailable,
    #[error("host-ingress projection requires extension lifecycle state")]
    ExtensionLifecycleUnavailable,
    #[error(transparent)]
    HostIngressRegistry(#[from] ironclaw_host_ingress_registry::Error),
    #[cfg(feature = "slack-v2-host-beta")]
    #[error(transparent)]
    SlackHostBeta(#[from] SlackHostBetaBuildError),
    #[cfg(feature = "telegram-v2-host-beta")]
    #[error(transparent)]
    TelegramHostBeta(#[from] TelegramHostBetaBuildError),
    #[error("{reason}")]
    RequiredProjectionMissing { reason: &'static str },
    #[error("invalid connectable-channel metadata in extension `{extension_id}`: {reason}")]
    InvalidConnectableMetadata {
        extension_id: String,
        reason: String,
    },
}

pub async fn build_host_ingress_serve_plan(
    runtime: &RebornRuntime,
    input: HostIngressServePlanInput,
) -> Result<HostIngressServePlan, HostIngressServePlanError> {
    #[cfg(not(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta")))]
    let _ = input;
    let mut plan = HostIngressServePlan::default();
    let mut projection_policy = ProjectionPolicy::default();
    let mut active_route_ids = BTreeSet::new();

    #[cfg(feature = "slack-v2-host-beta")]
    if let Some(slack) = input.slack {
        let mounts = build_slack_host_beta_mounts(runtime, slack.config)?;
        plan.requires_outbound_delivery_target_provider = true;
        collect_descriptor_route_ids(
            slack_personal_binding_pairing_route_descriptors().iter(),
            &mut active_route_ids,
        );
        if slack.operator_route_visibility == HostIngressOperatorRouteVisibility::Visible {
            collect_descriptor_route_ids(
                slack_channel_route_admin_descriptors().iter(),
                &mut active_route_ids,
            );
        }
        plan.slack_personal_binding_pairing = Some(mounts.personal_binding_pairing);
        plan.slack_channel_routes = Some(mounts.channel_routes);
        match slack.projection_mode {
            ConfiguredHostIngressProjectionMode::Suppress => {
                projection_policy.set(
                    SLACK_EVENTS_HOST_INGRESS_ROUTE_ID,
                    ProjectionRouteMode::Suppress,
                    None,
                );
                record_mount_route_ids(&mounts.events, &mut active_route_ids);
                plan.public_route_mounts.push(mounts.events);
            }
            ConfiguredHostIngressProjectionMode::ValidateOnly => {
                projection_policy.set(
                    SLACK_EVENTS_HOST_INGRESS_ROUTE_ID,
                    ProjectionRouteMode::Suppress,
                    Some(SLACK_CONFIG_IMPORT_MISSING_ROUTE),
                );
                record_mount_route_ids(&mounts.events, &mut active_route_ids);
                plan.public_route_mounts.push(mounts.events);
            }
            ConfiguredHostIngressProjectionMode::Serve => {
                projection_policy.set(
                    SLACK_EVENTS_HOST_INGRESS_ROUTE_ID,
                    ProjectionRouteMode::Serve,
                    Some(SLACK_CONFIG_IMPORT_MISSING_ROUTE),
                );
            }
        }
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    if let Some(telegram) = input.telegram {
        let mode = match telegram.projection_mode {
            ConfiguredHostIngressProjectionMode::Suppress => ProjectionRouteMode::Suppress,
            ConfiguredHostIngressProjectionMode::ValidateOnly => ProjectionRouteMode::Suppress,
            ConfiguredHostIngressProjectionMode::Serve => ProjectionRouteMode::Serve,
        };
        let required = match telegram.projection_mode {
            ConfiguredHostIngressProjectionMode::Suppress => None,
            ConfiguredHostIngressProjectionMode::ValidateOnly
            | ConfiguredHostIngressProjectionMode::Serve => {
                Some(TELEGRAM_CONFIG_IMPORT_MISSING_ROUTE)
            }
        };
        projection_policy.set(TELEGRAM_UPDATES_HOST_INGRESS_ROUTE_ID, mode, required);
    }

    for mount in build_host_ingress_mounts_from_enabled_extensions(runtime).await? {
        projection_policy.record_seen(&mount);
        if projection_policy.should_serve(&mount) {
            record_mount_route_ids(&mount, &mut active_route_ids);
            plan.public_route_mounts.push(mount);
        }
    }
    projection_policy.validate_required()?;

    plan.connectable_channels =
        project_enabled_connectable_channels(runtime, &active_route_ids).await?;

    Ok(plan)
}

pub async fn build_host_ingress_mounts_from_enabled_extensions(
    runtime: &RebornRuntime,
) -> Result<Vec<PublicRouteMount>, HostIngressServePlanError> {
    #[cfg(not(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta")))]
    {
        let _ = runtime;
        Ok(Vec::new())
    }
    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
    {
        let entries = enabled_host_ingress_entries(runtime).await?;
        build_host_ingress_mounts_from_entries(runtime, &entries).await
    }
}

pub fn build_webui_services_with_host_ingress_plan(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
    plan: &HostIngressServePlan,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    if plan.requires_outbound_delivery_target_provider
        && runtime.outbound_delivery_target_provider().is_none()
    {
        return Err(RebornBuildError::InvalidConfig {
            reason: "outbound delivery target providers require local runtime services".to_string(),
        });
    }
    let connectable_channels = (!plan.connectable_channels.is_empty()).then(|| {
        Arc::new(StaticConnectableChannelsProductFacade::new(
            plan.connectable_channels.clone(),
        )) as Arc<dyn ConnectableChannelsProductFacade>
    });
    build_webui_services_with_connectable_channels(
        runtime,
        event_stream,
        connectable_channels,
        Vec::new(),
    )
}

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
async fn enabled_host_ingress_entries(
    runtime: &RebornRuntime,
) -> Result<Vec<HostIngressRuntimeEntry>, HostIngressServePlanError> {
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(HostIngressServePlanError::DurableHostStateUnavailable)?;
    let extension_management = local_runtime
        .extension_management
        .as_ref()
        .ok_or(HostIngressServePlanError::ExtensionLifecycleUnavailable)?;
    let store = extension_management.installation_store();
    Ok(list_enabled_host_ingress_entries(store.as_ref()).await?)
}

#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
async fn build_host_ingress_mounts_from_entries(
    runtime: &RebornRuntime,
    entries: &[HostIngressRuntimeEntry],
) -> Result<Vec<PublicRouteMount>, HostIngressServePlanError> {
    let mut mounts = Vec::new();
    #[cfg(feature = "slack-v2-host-beta")]
    if let Some(mount) =
        build_slack_events_host_ingress_mount_from_entries(runtime, entries).await?
    {
        mounts.push(mount);
    }
    #[cfg(feature = "telegram-v2-host-beta")]
    if let Some(mount) =
        build_telegram_updates_host_ingress_mount_from_entries(runtime, entries).await?
    {
        mounts.push(mount);
    }
    Ok(mounts)
}

#[derive(Default)]
struct ProjectionPolicy {
    routes: BTreeMap<&'static str, ProjectionRoutePolicy>,
}

impl ProjectionPolicy {
    #[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
    fn set(
        &mut self,
        route_id: &'static str,
        mode: ProjectionRouteMode,
        required_error: Option<&'static str>,
    ) {
        self.routes.insert(
            route_id,
            ProjectionRoutePolicy {
                mode,
                required_error,
                seen: false,
            },
        );
    }

    fn record_seen(&mut self, mount: &PublicRouteMount) {
        for route_id in mount_route_ids(mount) {
            if let Some(policy) = self.routes.get_mut(route_id.as_str()) {
                policy.seen = true;
                if policy.required_error.is_some() && policy.mode == ProjectionRouteMode::Suppress {
                    tracing::debug!(
                        target = HOST_INGRESS_LOG_TARGET,
                        route_id,
                        "extension host-ingress route projection validated",
                    );
                }
            }
        }
    }

    fn should_serve(&self, mount: &PublicRouteMount) -> bool {
        let route_ids = mount_route_ids(mount);
        if route_ids.is_empty() {
            return false;
        }
        route_ids.iter().any(|route_id| {
            self.routes
                .get(route_id.as_str())
                .map(|policy| policy.mode == ProjectionRouteMode::Serve)
                .unwrap_or(true)
        })
    }

    fn validate_required(&self) -> Result<(), HostIngressServePlanError> {
        for policy in self.routes.values() {
            if let Some(reason) = policy.required_error
                && !policy.seen
            {
                return Err(HostIngressServePlanError::RequiredProjectionMissing { reason });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectionRouteMode {
    Suppress,
    Serve,
}

struct ProjectionRoutePolicy {
    mode: ProjectionRouteMode,
    required_error: Option<&'static str>,
    seen: bool,
}

async fn project_enabled_connectable_channels(
    runtime: &RebornRuntime,
    active_route_ids: &BTreeSet<String>,
) -> Result<Vec<RebornConnectableChannelInfo>, HostIngressServePlanError> {
    if active_route_ids.is_empty() {
        return Ok(Vec::new());
    }
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(HostIngressServePlanError::DurableHostStateUnavailable)?;
    let extension_management = local_runtime
        .extension_management
        .as_ref()
        .ok_or(HostIngressServePlanError::ExtensionLifecycleUnavailable)?;
    let store = extension_management.installation_store();
    let manifests = store
        .list_manifests()
        .await
        .map_err(ironclaw_host_ingress_registry::Error::from)?;
    let manifest_by_extension: BTreeMap<_, _> = manifests
        .iter()
        .map(|manifest| (manifest.extension_id().clone(), manifest))
        .collect();
    let mut seen = HashSet::new();
    let mut channels = Vec::new();

    for installation in store
        .list_enabled_installations()
        .await
        .map_err(ironclaw_host_ingress_registry::Error::from)?
    {
        let Some(manifest) = manifest_by_extension.get(installation.extension_id()) else {
            return Err(HostIngressServePlanError::HostIngressRegistry(
                ironclaw_host_ingress_registry::Error::UnknownManifest {
                    extension_id: installation.extension_id().to_string(),
                },
            ));
        };
        for declaration in connectable_channel_metadata(manifest)? {
            if !declaration
                .requires_route_ids
                .iter()
                .all(|route_id| active_route_ids.contains(route_id))
            {
                continue;
            }
            let strategy_key = declaration.strategy.as_str().to_string();
            let channel = declaration.into_channel_info(manifest)?;
            if seen.insert((channel.channel.clone(), strategy_key)) {
                channels.push(channel);
            }
        }
    }

    Ok(channels)
}

fn connectable_channel_metadata(
    manifest: &ExtensionManifestRecord,
) -> Result<Vec<ConnectableChannelMetadata>, HostIngressServePlanError> {
    let document: toml::Value = toml::from_str(manifest.raw_toml()).map_err(|error| {
        HostIngressServePlanError::InvalidConnectableMetadata {
            extension_id: manifest.extension_id().to_string(),
            reason: error.to_string(),
        }
    })?;
    let Some(channels) = document
        .get("metadata")
        .and_then(|value| value.get("connectable"))
        .and_then(|value| value.get("channels"))
    else {
        return Ok(Vec::new());
    };
    let Some(items) = channels.as_array() else {
        return Err(HostIngressServePlanError::InvalidConnectableMetadata {
            extension_id: manifest.extension_id().to_string(),
            reason: "metadata.connectable.channels must be an array".to_string(),
        });
    };
    items
        .iter()
        .map(|item| {
            item.clone().try_into().map_err(|error: toml::de::Error| {
                HostIngressServePlanError::InvalidConnectableMetadata {
                    extension_id: manifest.extension_id().to_string(),
                    reason: error.to_string(),
                }
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectableChannelMetadata {
    channel: String,
    #[serde(default)]
    display_name: Option<String>,
    strategy: ConnectableChannelStrategyMetadata,
    #[serde(default)]
    requires_route_ids: Vec<String>,
    title: String,
    instructions: String,
    input_placeholder: String,
    submit_label: String,
    success_message: String,
    error_message: String,
    #[serde(default)]
    command_aliases: Vec<String>,
}

impl ConnectableChannelMetadata {
    fn into_channel_info(
        self,
        manifest: &ExtensionManifestRecord,
    ) -> Result<RebornConnectableChannelInfo, HostIngressServePlanError> {
        validate_connectable_field(manifest, "channel", &self.channel)?;
        validate_connectable_field(manifest, "title", &self.title)?;
        validate_connectable_field(manifest, "instructions", &self.instructions)?;
        validate_connectable_field(manifest, "input_placeholder", &self.input_placeholder)?;
        validate_connectable_field(manifest, "submit_label", &self.submit_label)?;
        validate_connectable_field(manifest, "success_message", &self.success_message)?;
        validate_connectable_field(manifest, "error_message", &self.error_message)?;
        for route_id in &self.requires_route_ids {
            validate_connectable_field(manifest, "requires_route_ids", route_id)?;
        }
        let display_name = self
            .display_name
            .unwrap_or_else(|| manifest.manifest().name.clone());
        validate_connectable_field(manifest, "display_name", &display_name)?;
        Ok(RebornConnectableChannelInfo {
            channel: self.channel,
            display_name,
            strategy: self.strategy.into(),
            action: RebornChannelConnectAction {
                title: self.title,
                instructions: self.instructions,
                input_placeholder: self.input_placeholder,
                submit_label: self.submit_label,
                success_message: self.success_message,
                error_message: self.error_message,
            },
            command_aliases: self.command_aliases,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConnectableChannelStrategyMetadata {
    InboundProofCode,
    AdminManagedChannels,
}

impl ConnectableChannelStrategyMetadata {
    fn as_str(self) -> &'static str {
        match self {
            Self::InboundProofCode => "inbound_proof_code",
            Self::AdminManagedChannels => "admin_managed_channels",
        }
    }
}

impl From<ConnectableChannelStrategyMetadata> for RebornChannelConnectStrategy {
    fn from(value: ConnectableChannelStrategyMetadata) -> Self {
        match value {
            ConnectableChannelStrategyMetadata::InboundProofCode => Self::InboundProofCode,
            ConnectableChannelStrategyMetadata::AdminManagedChannels => Self::AdminManagedChannels,
        }
    }
}

fn validate_connectable_field(
    manifest: &ExtensionManifestRecord,
    field: &str,
    value: &str,
) -> Result<(), HostIngressServePlanError> {
    if value.trim().is_empty() {
        return Err(HostIngressServePlanError::InvalidConnectableMetadata {
            extension_id: manifest.extension_id().to_string(),
            reason: format!("{field} must not be empty"),
        });
    }
    Ok(())
}

fn record_mount_route_ids(mount: &PublicRouteMount, route_ids: &mut BTreeSet<String>) {
    collect_descriptor_route_ids(mount.descriptors.iter(), route_ids);
}

fn collect_descriptor_route_ids<'a>(
    descriptors: impl IntoIterator<Item = &'a IngressRouteDescriptor>,
    route_ids: &mut BTreeSet<String>,
) {
    for descriptor in descriptors {
        route_ids.insert(descriptor.route_id().as_str().to_string());
    }
}

fn mount_route_ids(mount: &PublicRouteMount) -> Vec<String> {
    mount
        .descriptors
        .iter()
        .map(|descriptor| descriptor.route_id().as_str().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_extensions::{
        ExtensionManifestRecord, HostApiContractRegistry, ManifestHash, ManifestSource,
    };
    use ironclaw_host_api::HostPortCatalog;
    use ironclaw_host_ingress_registry::HostIngressHostApiContract;

    use super::*;

    #[test]
    fn connectable_metadata_projects_channel_info() {
        let manifest = manifest_record(
            r#"
[[metadata.connectable.channels]]
channel = "demo"
strategy = "inbound_proof_code"
requires_route_ids = ["demo.events"]
title = "Demo account connection"
instructions = "Message the app, then enter the code here."
input_placeholder = "Enter code..."
submit_label = "Connect"
success_message = "Connected."
error_message = "Invalid code."
command_aliases = ["demo"]
"#,
        );

        let metadata = connectable_channel_metadata(&manifest).expect("metadata parses");
        assert_eq!(metadata.len(), 1);
        let channel = metadata
            .into_iter()
            .next()
            .expect("metadata includes one channel")
            .into_channel_info(&manifest)
            .expect("channel projects");

        assert_eq!(channel.channel, "demo");
        assert_eq!(channel.display_name, "Demo");
        assert_eq!(
            channel.strategy,
            RebornChannelConnectStrategy::InboundProofCode
        );
        assert_eq!(channel.command_aliases, vec!["demo"]);
    }

    #[test]
    fn connectable_metadata_rejects_empty_required_route_id() {
        let manifest = manifest_record(
            r#"
[[metadata.connectable.channels]]
channel = "demo"
strategy = "inbound_proof_code"
requires_route_ids = [""]
title = "Demo account connection"
instructions = "Message the app, then enter the code here."
input_placeholder = "Enter code..."
submit_label = "Connect"
success_message = "Connected."
error_message = "Invalid code."
"#,
        );

        let metadata = connectable_channel_metadata(&manifest).expect("metadata parses");
        let error = metadata
            .into_iter()
            .next()
            .expect("metadata includes one channel")
            .into_channel_info(&manifest)
            .expect_err("empty route id rejected");

        assert!(
            error
                .to_string()
                .contains("requires_route_ids must not be empty")
        );
    }

    fn manifest_record(metadata_toml: &str) -> ExtensionManifestRecord {
        let raw_toml = format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "demo"
name = "Demo"
version = "0.1.0"
description = "Demo channel."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "demo"

[[host_api]]
id = "ironclaw.host_ingress/v1"
section = "host_ingress.events"

[host_ingress.events]

[host_ingress.events.transport]
kind = "webhook"
route_id = "demo.events"
method = "post"
path = "/webhooks/demo/events"
ack = "immediate"
drain = "drain_before_runtime_shutdown"

[host_ingress.events.policy]
listener_class = "public_webhook"
scope_source = "host_resolved"
cors = "not_applicable"
websocket_origin = "not_applicable"
streaming = "none"
audit = "public_callback"

[host_ingress.events.policy.auth]
type = "required"
schemes = ["shared_secret_header"]

[host_ingress.events.policy.body_limit]
type = "limited"
max_bytes = 1024

[host_ingress.events.policy.rate_limit]
type = "limited"
scope = "global"
max_requests = 10
window_seconds = 60

[host_ingress.events.policy.effect_path]
type = "product_workflow"

[host_ingress.events.target]
type = "product_adapter_inbound"
capability_id = "demo.events"
product_adapter_section = "product_adapter.inbound"

[host_ingress.events.auth]
verifier = "shared_secret_header"
credential_handles = ["demo_secret"]

{metadata_toml}
"#
        );
        let mut contracts = HostApiContractRegistry::new();
        contracts
            .register(Arc::new(
                HostIngressHostApiContract::new().expect("host ingress contract"),
            ))
            .expect("register host ingress contract");
        ExtensionManifestRecord::from_toml_with_contracts(
            raw_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            Some(ManifestHash::new("test-hash").expect("hash")),
            &contracts,
        )
        .expect("manifest record")
    }
}
