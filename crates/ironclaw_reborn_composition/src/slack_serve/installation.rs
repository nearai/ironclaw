//! Slack installation resolution and post-auth installation-scoped ingress policy.

use std::collections::HashMap;
use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use futures::future::join_all;
use ironclaw_host_api::TenantId;
use ironclaw_product_adapters::{AdapterInstallationId, ProtocolAuthEvidence};
use ironclaw_slack_v2_adapter::SlackPayloadParseError;
use ironclaw_wasm_product_adapters::{ImmediateAckWorkflowObserver, RunnerError};
use serde::Deserialize;
use thiserror::Error;

use super::SlackEventsWebhookDispatcher;

const SLACK_INSTALLATION_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(120).unwrap(); // safety: 120 requests is a non-zero literal.
const SLACK_INSTALLATION_RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_SLACK_METADATA_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_SLACK_VERIFICATION_CANDIDATES: usize = 8;

macro_rules! slack_id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

slack_id_type!(SlackTeamId);
slack_id_type!(SlackEnterpriseId);
slack_id_type!(SlackApiAppId);
slack_id_type!(SlackUserId);
slack_id_type!(SlackChannelId);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackEnvelopeMetadata {
    pub team_id: Option<SlackTeamId>,
    pub enterprise_id: Option<SlackEnterpriseId>,
    pub api_app_id: Option<SlackApiAppId>,
    pub install_user_id: Option<SlackUserId>,
    pub event_user_id: Option<SlackUserId>,
    pub event_channel_id: Option<SlackChannelId>,
    install_contexts: Vec<SlackInstallContext>,
}

impl SlackEnvelopeMetadata {
    pub fn new(
        team_id: Option<SlackTeamId>,
        enterprise_id: Option<SlackEnterpriseId>,
        api_app_id: Option<SlackApiAppId>,
        install_user_id: Option<SlackUserId>,
        event_user_id: Option<SlackUserId>,
        event_channel_id: Option<SlackChannelId>,
    ) -> Self {
        Self {
            team_id: team_id.clone(),
            enterprise_id: enterprise_id.clone(),
            api_app_id,
            install_user_id: install_user_id.clone(),
            event_user_id,
            event_channel_id,
            install_contexts: vec![SlackInstallContext {
                team_id,
                enterprise_id,
                install_user_id,
            }],
        }
    }

    fn from_wrapper(wrapper: SlackEnvelopeMetadataWrapper) -> Self {
        let event = wrapper.event;
        let mut install_contexts: Vec<_> = wrapper
            .authorizations
            .into_iter()
            .map(SlackInstallContext::from_authorization)
            .collect();

        if install_contexts.is_empty() {
            install_contexts.push(SlackInstallContext {
                team_id: wrapper
                    .context_team_id
                    .or(wrapper.team_id)
                    .map(SlackTeamId::new),
                enterprise_id: wrapper
                    .context_enterprise_id
                    .or(wrapper.enterprise_id)
                    .map(SlackEnterpriseId::new),
                install_user_id: None,
            });
        }

        let primary_context = install_contexts.first().cloned().unwrap_or_default();
        Self {
            team_id: primary_context.team_id.clone(),
            enterprise_id: primary_context.enterprise_id.clone(),
            api_app_id: wrapper.api_app_id.map(SlackApiAppId::new),
            install_user_id: primary_context.install_user_id.clone(),
            event_user_id: event
                .as_ref()
                .and_then(|event| event.user.clone())
                .map(SlackUserId::new),
            event_channel_id: event
                .and_then(|event| event.channel)
                .map(SlackChannelId::new),
            install_contexts,
        }
    }

    fn install_contexts(&self) -> impl Iterator<Item = &SlackInstallContext> {
        self.install_contexts.iter()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SlackInstallContext {
    team_id: Option<SlackTeamId>,
    enterprise_id: Option<SlackEnterpriseId>,
    install_user_id: Option<SlackUserId>,
}

impl SlackInstallContext {
    fn from_authorization(authorization: SlackAuthorizationMetadata) -> Self {
        Self {
            team_id: authorization.team_id.map(SlackTeamId::new),
            enterprise_id: authorization.enterprise_id.map(SlackEnterpriseId::new),
            install_user_id: authorization.user_id.map(SlackUserId::new),
        }
    }
}

fn parse_slack_envelope(
    raw_payload: &[u8],
) -> Result<SlackEnvelopeMetadataWrapper, SlackPayloadParseError> {
    if raw_payload.len() > MAX_SLACK_METADATA_PAYLOAD_BYTES {
        return Err(SlackPayloadParseError::InvalidJson {
            reason: "payload exceeds size limit".into(),
        });
    }
    serde_json::from_slice(raw_payload).map_err(|err| SlackPayloadParseError::InvalidJson {
        reason: err.to_string(),
    })
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEnvelopeMetadataWrapper {
    #[serde(rename = "type")]
    kind: Option<String>,
    challenge: Option<String>,
    team_id: Option<String>,
    enterprise_id: Option<String>,
    context_team_id: Option<String>,
    context_enterprise_id: Option<String>,
    api_app_id: Option<String>,
    event: Option<SlackEnvelopeEventMetadata>,
    #[serde(default)]
    authorizations: Vec<SlackAuthorizationMetadata>,
}

impl SlackEnvelopeMetadataWrapper {
    fn is_url_verification(&self) -> bool {
        self.kind.as_deref() == Some("url_verification")
    }

    fn into_challenge(self) -> Result<String, SlackPayloadParseError> {
        self.challenge
            .ok_or_else(|| SlackPayloadParseError::InvalidJson {
                reason: "missing challenge".into(),
            })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEnvelopeEventMetadata {
    user: Option<String>,
    channel: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackAuthorizationMetadata {
    team_id: Option<String>,
    enterprise_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Clone)]
pub struct ResolvedSlackInstallation {
    tenant_id: TenantId,
    adapter_installation_id: AdapterInstallationId,
    evidence: ProtocolAuthEvidence,
    dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
    workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
}

impl ResolvedSlackInstallation {
    pub fn new(
        tenant_id: TenantId,
        adapter_installation_id: AdapterInstallationId,
        evidence: ProtocolAuthEvidence,
        dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
        workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Self {
        Self {
            tenant_id,
            adapter_installation_id,
            evidence,
            dispatcher,
            workflow_observer,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn adapter_installation_id(&self) -> &AdapterInstallationId {
        &self.adapter_installation_id
    }

    pub fn evidence(&self) -> &ProtocolAuthEvidence {
        &self.evidence
    }

    pub fn dispatcher(&self) -> Arc<dyn SlackEventsWebhookDispatcher> {
        Arc::clone(&self.dispatcher)
    }

    pub fn workflow_observer(&self) -> Option<Arc<dyn ImmediateAckWorkflowObserver>> {
        self.workflow_observer.clone()
    }
}

impl std::fmt::Debug for ResolvedSlackInstallation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedSlackInstallation")
            .field("tenant_id", &self.tenant_id)
            .field("adapter_installation_id", &self.adapter_installation_id)
            .field("dispatcher", &"Arc<dyn SlackEventsWebhookDispatcher>")
            .field("workflow_observer", &self.workflow_observer.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub enum ResolvedSlackIngress {
    UrlVerification {
        installation: ResolvedSlackInstallation,
        challenge: String,
    },
    Event {
        installation: ResolvedSlackInstallation,
        metadata: SlackEnvelopeMetadata,
    },
}

impl ResolvedSlackIngress {
    pub fn installation(&self) -> &ResolvedSlackInstallation {
        match self {
            Self::UrlVerification { installation, .. } | Self::Event { installation, .. } => {
                installation
            }
        }
    }
}

impl std::fmt::Debug for ResolvedSlackIngress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UrlVerification { installation, .. } => formatter
                .debug_struct("ResolvedSlackIngress::UrlVerification")
                .field("installation", installation)
                .finish(),
            Self::Event {
                installation,
                metadata,
            } => formatter
                .debug_struct("ResolvedSlackIngress::Event")
                .field("installation", installation)
                .field("metadata", metadata)
                .finish(),
        }
    }
}

/// A signature-verified Slack slash-command invocation, resolved to the
/// installation that owns it.
///
/// Produced only by [`SlackInstallationResolver::resolve_command_ingress`], and
/// only after the request signature verifies, so every field is trusted Slack
/// input bound to `installation`. `slack_user_id` is the user who *typed* the
/// command (the actor we pair), not the installing user.
#[derive(Clone)]
pub struct ResolvedSlackCommand {
    installation: ResolvedSlackInstallation,
    command: String,
    slack_user_id: SlackUserId,
}

impl ResolvedSlackCommand {
    pub fn new(
        installation: ResolvedSlackInstallation,
        command: String,
        slack_user_id: SlackUserId,
    ) -> Self {
        Self {
            installation,
            command,
            slack_user_id,
        }
    }

    pub fn installation(&self) -> &ResolvedSlackInstallation {
        &self.installation
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn slack_user_id(&self) -> &SlackUserId {
        &self.slack_user_id
    }
}

impl std::fmt::Debug for ResolvedSlackCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedSlackCommand")
            .field("installation", &self.installation)
            .field("command", &self.command)
            .field("slack_user_id", &self.slack_user_id)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlackIngressError {
    #[error(transparent)]
    Runner(#[from] RunnerError),
    #[error(transparent)]
    Envelope(#[from] SlackPayloadParseError),
    #[error("no verified Slack installation matched the signed envelope")]
    InstallationNotFound,
    #[error("multiple verified Slack installations matched the signed envelope")]
    AmbiguousInstallation,
    #[error(
        "Slack installation rate limit exceeded for tenant {tenant_id} installation {adapter_installation_id}"
    )]
    InstallationRateLimited {
        tenant_id: TenantId,
        adapter_installation_id: AdapterInstallationId,
    },
}

pub trait SlackInstallationResolver: Send + Sync {
    fn resolve_ingress<'a>(
        &'a self,
        headers: &'a HeaderMap,
        body: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedSlackIngress, SlackIngressError>> + Send + 'a>>;

    /// Resolve a Slack slash-command (`application/x-www-form-urlencoded`)
    /// request to its installation, verifying the request signature first.
    ///
    /// Slash commands carry no installing-user id, so routing matches on
    /// team / app / enterprise only (see [`SlackInstallationSelector::matches_command`]).
    /// This is a required method — every resolver participates in command
    /// routing, so there is deliberately no defaulted no-op that could leave a
    /// silent gap on the slash endpoint.
    fn resolve_command_ingress<'a>(
        &'a self,
        headers: &'a HeaderMap,
        body: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedSlackCommand, SlackIngressError>> + Send + 'a>>;

    fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

#[derive(Clone)]
pub struct SlackInstallationRecord {
    tenant_id: TenantId,
    adapter_installation_id: AdapterInstallationId,
    selector: SlackInstallationSelector,
    dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
    workflow_observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
}

impl SlackInstallationRecord {
    pub fn new(
        tenant_id: TenantId,
        adapter_installation_id: AdapterInstallationId,
        selector: SlackInstallationSelector,
        dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
    ) -> Self {
        Self {
            tenant_id,
            adapter_installation_id,
            selector,
            dispatcher,
            workflow_observer: None,
        }
    }

    pub fn with_workflow_observer(
        mut self,
        workflow_observer: Arc<dyn ImmediateAckWorkflowObserver>,
    ) -> Self {
        self.workflow_observer = Some(workflow_observer);
        self
    }
}

impl std::fmt::Debug for SlackInstallationRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackInstallationRecord")
            .field("tenant_id", &self.tenant_id)
            .field("adapter_installation_id", &self.adapter_installation_id)
            .field("selector", &self.selector)
            .field("dispatcher", &"Arc<dyn SlackEventsWebhookDispatcher>")
            .field("workflow_observer", &self.workflow_observer.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackInstallationSelector {
    Team {
        team_id: SlackTeamId,
    },
    AppTeam {
        api_app_id: SlackApiAppId,
        team_id: SlackTeamId,
    },
    EnterpriseTeam {
        enterprise_id: SlackEnterpriseId,
        team_id: SlackTeamId,
    },
    InstallUser {
        team_id: SlackTeamId,
        install_user_id: SlackUserId,
    },
    EnterpriseInstallUser {
        enterprise_id: SlackEnterpriseId,
        team_id: SlackTeamId,
        install_user_id: SlackUserId,
    },
    AppInstallUser {
        api_app_id: SlackApiAppId,
        team_id: SlackTeamId,
        install_user_id: SlackUserId,
    },
}

impl SlackInstallationSelector {
    pub fn team(team_id: impl Into<String>) -> Self {
        Self::Team {
            team_id: SlackTeamId::new(team_id),
        }
    }

    pub fn app_team(api_app_id: impl Into<String>, team_id: impl Into<String>) -> Self {
        Self::AppTeam {
            api_app_id: SlackApiAppId::new(api_app_id),
            team_id: SlackTeamId::new(team_id),
        }
    }

    pub fn enterprise_team(enterprise_id: impl Into<String>, team_id: impl Into<String>) -> Self {
        Self::EnterpriseTeam {
            enterprise_id: SlackEnterpriseId::new(enterprise_id),
            team_id: SlackTeamId::new(team_id),
        }
    }

    pub fn with_install_user_id(self, install_user_id: impl Into<String>) -> Self {
        match self {
            Self::Team { team_id } => Self::InstallUser {
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::AppTeam {
                api_app_id,
                team_id,
            } => Self::AppInstallUser {
                api_app_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::EnterpriseTeam {
                enterprise_id,
                team_id,
            } => Self::EnterpriseInstallUser {
                enterprise_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::InstallUser { team_id, .. } => Self::InstallUser {
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::EnterpriseInstallUser {
                enterprise_id,
                team_id,
                ..
            } => Self::EnterpriseInstallUser {
                enterprise_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
            Self::AppInstallUser {
                api_app_id,
                team_id,
                ..
            } => Self::AppInstallUser {
                api_app_id,
                team_id,
                install_user_id: SlackUserId::new(install_user_id),
            },
        }
    }

    fn matches(&self, metadata: &SlackEnvelopeMetadata) -> bool {
        match self {
            Self::Team { team_id } => metadata
                .install_contexts()
                .any(|context| context.team_id.as_ref() == Some(team_id)),
            Self::AppTeam {
                api_app_id,
                team_id,
            } => {
                metadata.api_app_id.as_ref() == Some(api_app_id)
                    && metadata
                        .install_contexts()
                        .any(|context| context.team_id.as_ref() == Some(team_id))
            }
            Self::EnterpriseTeam {
                enterprise_id,
                team_id,
            } => metadata.install_contexts().any(|context| {
                context.enterprise_id.as_ref() == Some(enterprise_id)
                    && context.team_id.as_ref() == Some(team_id)
            }),
            Self::InstallUser {
                team_id,
                install_user_id,
            } => metadata.install_contexts().any(|context| {
                context.team_id.as_ref() == Some(team_id)
                    && context.install_user_id.as_ref() == Some(install_user_id)
            }),
            Self::EnterpriseInstallUser {
                enterprise_id,
                team_id,
                install_user_id,
            } => metadata.install_contexts().any(|context| {
                context.enterprise_id.as_ref() == Some(enterprise_id)
                    && context.team_id.as_ref() == Some(team_id)
                    && context.install_user_id.as_ref() == Some(install_user_id)
            }),
            Self::AppInstallUser {
                api_app_id,
                team_id,
                install_user_id,
            } => {
                metadata.api_app_id.as_ref() == Some(api_app_id)
                    && metadata.install_contexts().any(|context| {
                        context.team_id.as_ref() == Some(team_id)
                            && context.install_user_id.as_ref() == Some(install_user_id)
                    })
            }
        }
    }

    /// Match this selector against a Slack slash-command's routing dimensions.
    ///
    /// Unlike [`Self::matches`], a slash command carries no installing-user id,
    /// so the per-install user dimension of the `*InstallUser` selectors is not
    /// applied here — a command routes on team (plus app / enterprise where the
    /// selector constrains them). If two installations share a team and differ
    /// only by install user, both match and `verify_candidates` resolves the
    /// ambiguity by signature (yielding `AmbiguousInstallation`), which the
    /// handler surfaces as a safe ephemeral error rather than guessing.
    fn matches_command(&self, context: &SlackCommandContext) -> bool {
        match self {
            Self::Team { team_id } => context.team_id.as_ref() == Some(team_id),
            Self::AppTeam {
                api_app_id,
                team_id,
            } => {
                context.api_app_id.as_ref() == Some(api_app_id)
                    && context.team_id.as_ref() == Some(team_id)
            }
            Self::EnterpriseTeam {
                enterprise_id,
                team_id,
            } => {
                context.enterprise_id.as_ref() == Some(enterprise_id)
                    && context.team_id.as_ref() == Some(team_id)
            }
            Self::InstallUser { team_id, .. } => context.team_id.as_ref() == Some(team_id),
            Self::EnterpriseInstallUser {
                enterprise_id,
                team_id,
                ..
            } => {
                context.enterprise_id.as_ref() == Some(enterprise_id)
                    && context.team_id.as_ref() == Some(team_id)
            }
            Self::AppInstallUser {
                api_app_id,
                team_id,
                ..
            } => {
                context.api_app_id.as_ref() == Some(api_app_id)
                    && context.team_id.as_ref() == Some(team_id)
            }
        }
    }
}

/// The installation-routing dimensions Slack includes on a slash command.
///
/// Built from the untrusted form body and used only to narrow which
/// installation's signing secret to check; trust is established by the
/// subsequent signature verification, not by these fields.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SlackCommandContext {
    team_id: Option<SlackTeamId>,
    api_app_id: Option<SlackApiAppId>,
    enterprise_id: Option<SlackEnterpriseId>,
}

impl SlackCommandContext {
    fn from_form(form: &SlackCommandForm) -> Self {
        Self {
            team_id: non_empty(form.team_id.as_deref()).map(SlackTeamId::new),
            api_app_id: non_empty(form.api_app_id.as_deref()).map(SlackApiAppId::new),
            enterprise_id: non_empty(form.enterprise_id.as_deref()).map(SlackEnterpriseId::new),
        }
    }
}

/// The slash-command form fields IronClaw routes on. All optional: a missing
/// field stays `None` so parsing never fails before signature verification —
/// a parse that could fail would leak a pre-auth oracle distinguishing a
/// malformed body from an unsigned one. Required fields are validated only
/// after the signature verifies (see `resolve_command_sync`).
#[derive(Debug, Default)]
struct SlackCommandForm {
    command: Option<String>,
    team_id: Option<String>,
    api_app_id: Option<String>,
    enterprise_id: Option<String>,
    user_id: Option<String>,
}

fn parse_slack_command_form(body: &[u8]) -> SlackCommandForm {
    let mut form = SlackCommandForm::default();
    for (key, value) in url::form_urlencoded::parse(body) {
        match key.as_ref() {
            "command" => form.command = Some(value.into_owned()),
            "team_id" => form.team_id = Some(value.into_owned()),
            "api_app_id" => form.api_app_id = Some(value.into_owned()),
            "enterprise_id" => form.enterprise_id = Some(value.into_owned()),
            "user_id" => form.user_id = Some(value.into_owned()),
            _ => {}
        }
    }
    form
}

/// Treat an empty form value as absent — Slack omits some fields by sending
/// them blank rather than dropping the key.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|candidate| !candidate.is_empty())
}

fn non_empty_owned(value: Option<String>) -> Option<String> {
    value.filter(|candidate| !candidate.is_empty())
}

fn command_field_missing() -> SlackIngressError {
    SlackIngressError::Envelope(SlackPayloadParseError::InvalidJson {
        reason: "Slack slash command form missing required field".into(),
    })
}

#[derive(Debug, Clone)]
pub struct StaticSlackInstallationResolver {
    installations: Vec<SlackInstallationRecord>,
}

impl StaticSlackInstallationResolver {
    pub fn new(installations: impl IntoIterator<Item = SlackInstallationRecord>) -> Self {
        Self {
            installations: installations.into_iter().collect(),
        }
    }

    fn resolve_sync(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ResolvedSlackIngress, SlackIngressError> {
        match parse_slack_envelope(body) {
            Ok(envelope) if envelope.is_url_verification() => {
                self.resolve_url_verification(headers, body, envelope)
            }
            Ok(envelope) => self.resolve_event(headers, body, envelope),
            Err(error) => self.resolve_unparseable(headers, body, error),
        }
    }

    fn resolve_url_verification(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        envelope: SlackEnvelopeMetadataWrapper,
    ) -> Result<ResolvedSlackIngress, SlackIngressError> {
        self.ensure_candidate_budget(self.installations.len())?;
        let mut verified = self.verify_candidates(self.installations.iter(), headers, body)?;
        if verified.len() > 1 {
            return Err(SlackIngressError::AmbiguousInstallation);
        }
        let (installation, evidence) = verified.remove(0);
        Ok(ResolvedSlackIngress::UrlVerification {
            installation: Self::resolved_installation(installation, evidence),
            challenge: envelope.into_challenge()?,
        })
    }

    fn resolve_event(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        envelope: SlackEnvelopeMetadataWrapper,
    ) -> Result<ResolvedSlackIngress, SlackIngressError> {
        let metadata = SlackEnvelopeMetadata::from_wrapper(envelope);
        let candidates: Vec<_> = self
            .installations
            .iter()
            .filter(|installation| installation.selector.matches(&metadata))
            .collect();
        if candidates.is_empty() {
            return Err(SlackIngressError::InstallationNotFound);
        }
        self.ensure_candidate_budget(candidates.len())?;

        let mut verified = self.verify_candidates(candidates, headers, body)?;
        if verified.len() > 1 {
            return Err(SlackIngressError::AmbiguousInstallation);
        }
        let (installation, evidence) = verified.remove(0);
        Ok(ResolvedSlackIngress::Event {
            installation: Self::resolved_installation(installation, evidence),
            metadata,
        })
    }

    fn resolve_command_sync(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ResolvedSlackCommand, SlackIngressError> {
        let form = parse_slack_command_form(body);
        let context = SlackCommandContext::from_form(&form);
        let candidates: Vec<_> = self
            .installations
            .iter()
            .filter(|installation| installation.selector.matches_command(&context))
            .collect();
        if candidates.is_empty() {
            self.ensure_candidate_budget(self.installations.len())?;
            self.verify_candidates(self.installations.iter(), headers, body)?;
            return Err(SlackIngressError::InstallationNotFound);
        }
        self.ensure_candidate_budget(candidates.len())?;

        let mut verified = self.verify_candidates(candidates, headers, body)?;
        if verified.len() > 1 {
            return Err(SlackIngressError::AmbiguousInstallation);
        }
        let (installation, evidence) = verified.remove(0);

        // Signature verified above, so the form is now trusted Slack input. A
        // real slash command always carries these fields; treat a
        // verified-but-empty one as a malformed payload rather than minting a
        // pairing code against a blank user.
        let command = non_empty_owned(form.command).ok_or_else(command_field_missing)?;
        let user_id = non_empty_owned(form.user_id).ok_or_else(command_field_missing)?;

        Ok(ResolvedSlackCommand::new(
            Self::resolved_installation(installation, evidence),
            command,
            SlackUserId::new(user_id),
        ))
    }

    fn resolve_unparseable(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        error: SlackPayloadParseError,
    ) -> Result<ResolvedSlackIngress, SlackIngressError> {
        let Some(installation) = self.installations.first() else {
            return Err(SlackIngressError::InstallationNotFound);
        };
        self.verify_candidates(std::iter::once(installation), headers, body)?;
        Err(error.into())
    }

    fn verify_candidates<'a>(
        &self,
        candidates: impl IntoIterator<Item = &'a SlackInstallationRecord>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Vec<(&'a SlackInstallationRecord, ProtocolAuthEvidence)>, SlackIngressError> {
        let mut auth_failure: Option<RunnerError> = None;
        let mut verified = Vec::new();
        for installation in candidates {
            match installation.dispatcher.verify_webhook_auth(headers, body) {
                Ok(evidence) => verified.push((installation, evidence)),
                Err(error) => {
                    auth_failure.get_or_insert(error);
                }
            };
        }

        if verified.is_empty() {
            return Err(auth_failure
                .unwrap_or(RunnerError::AuthenticationFailed {
                    failure: ironclaw_product_adapters::ProtocolAuthFailure::Missing,
                })
                .into());
        }
        Ok(verified)
    }

    fn ensure_candidate_budget(&self, candidate_count: usize) -> Result<(), SlackIngressError> {
        if candidate_count > MAX_SLACK_VERIFICATION_CANDIDATES {
            return Err(SlackIngressError::AmbiguousInstallation);
        }
        Ok(())
    }

    fn resolved_installation(
        installation: &SlackInstallationRecord,
        evidence: ProtocolAuthEvidence,
    ) -> ResolvedSlackInstallation {
        ResolvedSlackInstallation::new(
            installation.tenant_id.clone(),
            installation.adapter_installation_id.clone(),
            evidence,
            Arc::clone(&installation.dispatcher),
            installation.workflow_observer.clone(),
        )
    }
}

impl SlackInstallationResolver for StaticSlackInstallationResolver {
    fn resolve_ingress<'a>(
        &'a self,
        headers: &'a HeaderMap,
        body: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedSlackIngress, SlackIngressError>> + Send + 'a>>
    {
        Box::pin(async move { self.resolve_sync(headers, body) })
    }

    fn resolve_command_ingress<'a>(
        &'a self,
        headers: &'a HeaderMap,
        body: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedSlackCommand, SlackIngressError>> + Send + 'a>>
    {
        Box::pin(async move { self.resolve_command_sync(headers, body) })
    }

    fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let drains = self
                .installations
                .iter()
                .map(|installation| installation.dispatcher.drain_immediate_ack_tasks());
            join_all(drains).await;
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlackInstallationRateLimitConfig {
    pub max_requests: NonZeroU32,
    pub window: Duration,
}

impl SlackInstallationRateLimitConfig {
    pub fn new(max_requests: NonZeroU32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
        }
    }
}

impl Default for SlackInstallationRateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: SLACK_INSTALLATION_MAX_REQUESTS,
            window: SLACK_INSTALLATION_RATE_WINDOW,
        }
    }
}

#[derive(Clone)]
pub struct SlackInstallationRateLimiter {
    config: SlackInstallationRateLimitConfig,
    buckets: Arc<Mutex<HashMap<SlackInstallationRateLimitKey, SlackRateLimitBucket>>>,
}

impl SlackInstallationRateLimiter {
    pub fn new(config: SlackInstallationRateLimitConfig) -> Self {
        Self {
            config,
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn check(&self, installation: &ResolvedSlackInstallation) -> Result<(), SlackIngressError> {
        let now = Instant::now();
        let key = SlackInstallationRateLimitKey {
            tenant_id: installation.tenant_id.clone(),
            adapter_installation_id: installation.adapter_installation_id.clone(),
        };
        let mut buckets = match self.buckets.lock() {
            Ok(buckets) => buckets,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.prune_stale_buckets(&mut buckets, now);
        let bucket = buckets
            .entry(key)
            .or_insert_with(|| SlackRateLimitBucket::full(now, &self.config));
        bucket.refill(now, &self.config);
        if !bucket.try_consume() {
            return Err(SlackIngressError::InstallationRateLimited {
                tenant_id: installation.tenant_id.clone(),
                adapter_installation_id: installation.adapter_installation_id.clone(),
            });
        }
        Ok(())
    }

    fn prune_stale_buckets(
        &self,
        buckets: &mut HashMap<SlackInstallationRateLimitKey, SlackRateLimitBucket>,
        now: Instant,
    ) {
        let ttl = self.config.window.saturating_mul(2);
        let capacity = self.config.max_requests.get() as f64;
        buckets.retain(|_, bucket| {
            now.duration_since(bucket.last_refilled_at) < ttl || bucket.tokens < capacity
        });
    }
}

impl std::fmt::Debug for SlackInstallationRateLimiter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackInstallationRateLimiter")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SlackInstallationRateLimitKey {
    tenant_id: TenantId,
    adapter_installation_id: AdapterInstallationId,
}

#[derive(Debug, Clone)]
struct SlackRateLimitBucket {
    last_refilled_at: Instant,
    tokens: f64,
}

impl SlackRateLimitBucket {
    fn full(now: Instant, config: &SlackInstallationRateLimitConfig) -> Self {
        Self {
            last_refilled_at: now,
            tokens: config.max_requests.get() as f64,
        }
    }

    fn refill(&mut self, now: Instant, config: &SlackInstallationRateLimitConfig) {
        let elapsed = now.duration_since(self.last_refilled_at);
        if elapsed.is_zero() {
            return;
        }
        let capacity = config.max_requests.get() as f64;
        let refill_ratio = if config.window.is_zero() {
            1.0
        } else {
            elapsed.as_secs_f64() / config.window.as_secs_f64()
        };
        self.tokens = capacity.min(self.tokens + refill_ratio * capacity);
        self.last_refilled_at = now;
    }

    fn try_consume(&mut self) -> bool {
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::http::HeaderMap;
    use ironclaw_product_adapters::ProtocolAuthFailure;
    use ironclaw_product_adapters::auth::mark_request_signature_verified;
    use ironclaw_wasm_product_adapters::{ImmediateAckWorkflowObserver, WebhookProcessOutcome};

    use super::*;

    struct AlwaysVerifiedDispatcher {
        subject: &'static str,
    }

    struct CountingVerifiedDispatcher {
        subject: &'static str,
        calls: Arc<AtomicUsize>,
    }

    impl SlackEventsWebhookDispatcher for AlwaysVerifiedDispatcher {
        fn verify_webhook_auth(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<ProtocolAuthEvidence, RunnerError> {
            Ok(mark_request_signature_verified(
                "X-Slack-Signature",
                Some("X-Slack-Request-Timestamp".to_string()),
                self.subject,
            ))
        }

        fn process_verified_webhook_immediate_ack<'a>(
            &'a self,
            _body: &'a [u8],
            _evidence: &'a ProtocolAuthEvidence,
            _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
        ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>
        {
            Box::pin(async { Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch) })
        }

        fn drain_immediate_ack_tasks<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    impl SlackEventsWebhookDispatcher for CountingVerifiedDispatcher {
        fn verify_webhook_auth(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<ProtocolAuthEvidence, RunnerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(mark_request_signature_verified(
                "X-Slack-Signature",
                Some("X-Slack-Request-Timestamp".to_string()),
                self.subject,
            ))
        }

        fn process_verified_webhook_immediate_ack<'a>(
            &'a self,
            _body: &'a [u8],
            _evidence: &'a ProtocolAuthEvidence,
            _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
        ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>
        {
            Box::pin(async { Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch) })
        }

        fn drain_immediate_ack_tasks<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    fn tenant_id(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant")
    }

    fn installation_id(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).expect("valid installation")
    }

    fn dispatcher(subject: &'static str) -> Arc<dyn SlackEventsWebhookDispatcher> {
        Arc::new(AlwaysVerifiedDispatcher { subject })
    }

    fn counting_dispatcher(
        subject: &'static str,
        calls: Arc<AtomicUsize>,
    ) -> Arc<dyn SlackEventsWebhookDispatcher> {
        Arc::new(CountingVerifiedDispatcher { subject, calls })
    }

    #[test]
    fn envelope_metadata_prefers_authorization_context_for_slack_connect() {
        let metadata = SlackEnvelopeMetadata::from_wrapper(
            parse_slack_envelope(
                br#"{
                    "type": "event_callback",
                    "team_id": "T-external",
                    "context_team_id": "T-context",
                    "api_app_id": "A-slack",
                    "authorizations": [{
                        "team_id": "T-install",
                        "enterprise_id": "E-install",
                        "user_id": "U-install"
                    }],
                    "event": {"type": "message", "user": "U-external", "channel": "C-shared"}
                }"#,
            )
            .expect("envelope parses"),
        );

        assert_eq!(metadata.team_id.as_deref(), Some("T-install"));
        assert_eq!(metadata.enterprise_id.as_deref(), Some("E-install"));
        assert_eq!(metadata.install_user_id.as_deref(), Some("U-install"));
        assert_eq!(metadata.event_user_id.as_deref(), Some("U-external"));
    }

    #[test]
    fn envelope_metadata_matches_all_authorization_contexts() {
        let metadata = SlackEnvelopeMetadata::from_wrapper(
            parse_slack_envelope(
                br#"{
                    "type": "event_callback",
                    "api_app_id": "A-slack",
                    "authorizations": [
                        {"team_id": "T-shared", "user_id": "U-install-a"},
                        {"team_id": "T-shared", "user_id": "U-install-b"}
                    ],
                    "event": {"type": "message", "user": "U-event", "channel": "D123"}
                }"#,
            )
            .expect("envelope parses"),
        );

        assert!(
            SlackInstallationSelector::team("T-shared")
                .with_install_user_id("U-install-b")
                .matches(&metadata)
        );
    }

    #[tokio::test]
    async fn static_resolver_allows_url_verification_before_selector_matching() -> Result<(), String>
    {
        let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
            tenant_id("tenant-a"),
            installation_id("install-a"),
            SlackInstallationSelector::team("T-A"),
            dispatcher("install-a"),
        )]);

        let ingress = resolver
            .resolve_ingress(
                &HeaderMap::new(),
                br#"{"type":"url_verification","challenge":"challenge-token"}"#,
            )
            .await
            .expect("url verification resolves before selector matching");

        let ResolvedSlackIngress::UrlVerification {
            installation,
            challenge,
        } = ingress
        else {
            return Err("expected url verification".to_string());
        };
        assert_eq!(installation.tenant_id().as_str(), "tenant-a");
        assert_eq!(installation.adapter_installation_id().as_str(), "install-a");
        assert_eq!(challenge, "challenge-token");
        Ok(())
    }

    #[tokio::test]
    async fn static_resolver_disambiguates_same_workspace_by_authorization_user()
    -> Result<(), String> {
        let resolver = StaticSlackInstallationResolver::new(vec![
            SlackInstallationRecord::new(
                tenant_id("tenant-a"),
                installation_id("install-a"),
                SlackInstallationSelector::team("T-shared").with_install_user_id("U-install-a"),
                dispatcher("install-a"),
            ),
            SlackInstallationRecord::new(
                tenant_id("tenant-b"),
                installation_id("install-b"),
                SlackInstallationSelector::team("T-shared").with_install_user_id("U-install-b"),
                dispatcher("install-b"),
            ),
        ]);

        let ingress = resolver
            .resolve_ingress(
                &HeaderMap::new(),
                br#"{
                    "type":"event_callback",
                    "team_id":"T-shared",
                    "api_app_id":"A-slack",
                    "authorizations":[{"team_id":"T-shared","user_id":"U-install-b"}],
                    "event":{"type":"message","user":"U-event","channel":"D123"}
                }"#,
            )
            .await
            .expect("authorization user disambiguates install");

        let ResolvedSlackIngress::Event {
            installation,
            metadata,
        } = ingress
        else {
            return Err("expected event".to_string());
        };
        assert_eq!(installation.tenant_id().as_str(), "tenant-b");
        assert_eq!(installation.adapter_installation_id().as_str(), "install-b");
        assert_eq!(metadata.install_user_id.as_deref(), Some("U-install-b"));
        Ok(())
    }

    #[tokio::test]
    async fn static_resolver_verifies_one_candidate_for_unparseable_payload() {
        let calls = Arc::new(AtomicUsize::new(0));
        let resolver = StaticSlackInstallationResolver::new(vec![
            SlackInstallationRecord::new(
                tenant_id("tenant-a"),
                installation_id("install-a"),
                SlackInstallationSelector::team("T-A"),
                counting_dispatcher("install-a", calls.clone()),
            ),
            SlackInstallationRecord::new(
                tenant_id("tenant-b"),
                installation_id("install-b"),
                SlackInstallationSelector::team("T-B"),
                counting_dispatcher("install-b", calls.clone()),
            ),
        ]);

        let error = resolver
            .resolve_ingress(&HeaderMap::new(), br#"{"type":"event_callback""#)
            .await
            .expect_err("malformed JSON should stay a parse error after auth");

        assert!(
            matches!(error, SlackIngressError::Envelope(_)),
            "error: {error}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "malformed payloads should not HMAC every configured installation"
        );
    }

    /// A dispatcher whose signature verification always fails, used to prove the
    /// slash-command path rejects an unsigned (or wrongly signed) request.
    struct FailingDispatcher;

    impl SlackEventsWebhookDispatcher for FailingDispatcher {
        fn verify_webhook_auth(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<ProtocolAuthEvidence, RunnerError> {
            Err(RunnerError::AuthenticationFailed {
                failure: ProtocolAuthFailure::SignatureMismatch,
            })
        }

        fn process_verified_webhook_immediate_ack<'a>(
            &'a self,
            _body: &'a [u8],
            _evidence: &'a ProtocolAuthEvidence,
            _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
        ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>
        {
            Box::pin(async {
                Err(RunnerError::AuthenticationFailed {
                    failure: ProtocolAuthFailure::SignatureMismatch,
                })
            })
        }

        fn drain_immediate_ack_tasks<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    fn failing_dispatcher() -> Arc<dyn SlackEventsWebhookDispatcher> {
        Arc::new(FailingDispatcher)
    }

    /// Build a realistic Slack slash-command body (`application/x-www-form-urlencoded`).
    fn slack_command_body(
        team_id: &str,
        user_id: &str,
        command: &str,
        response_url: &str,
    ) -> Vec<u8> {
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("token", "verification-token")
            .append_pair("team_id", team_id)
            .append_pair("api_app_id", "A-slack")
            .append_pair("channel_id", "D123")
            .append_pair("user_id", user_id)
            .append_pair("command", command)
            .append_pair("text", "")
            .append_pair("response_url", response_url)
            .append_pair("trigger_id", "123.456.abc")
            .finish()
            .into_bytes()
    }

    #[tokio::test]
    async fn resolve_command_ingress_verifies_and_extracts_user() -> Result<(), String> {
        let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
            tenant_id("tenant-a"),
            installation_id("install-a"),
            SlackInstallationSelector::team("T-A"),
            dispatcher("install-a"),
        )]);

        let body = slack_command_body(
            "T-A",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-A/123/abc",
        );

        let command = resolver
            .resolve_command_ingress(&HeaderMap::new(), &body)
            .await
            .map_err(|error| format!("verified /pair command should resolve: {error}"))?;

        assert_eq!(command.installation().tenant_id().as_str(), "tenant-a");
        assert_eq!(
            command.installation().adapter_installation_id().as_str(),
            "install-a"
        );
        assert_eq!(command.command(), "/pair");
        assert_eq!(command.slack_user_id().as_str(), "U123");
        Ok(())
    }

    #[tokio::test]
    async fn resolve_command_ingress_rejects_bad_signature() {
        let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
            tenant_id("tenant-a"),
            installation_id("install-a"),
            SlackInstallationSelector::team("T-A"),
            failing_dispatcher(),
        )]);

        let body = slack_command_body(
            "T-A",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-A/123/abc",
        );

        let error = resolver
            .resolve_command_ingress(&HeaderMap::new(), &body)
            .await
            .expect_err("an unsigned /pair command must be rejected");

        assert!(
            matches!(
                error,
                SlackIngressError::Runner(RunnerError::AuthenticationFailed { .. })
            ),
            "error: {error}"
        );
    }

    #[tokio::test]
    async fn resolve_command_ingress_rejects_unknown_team() {
        let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
            tenant_id("tenant-a"),
            installation_id("install-a"),
            SlackInstallationSelector::team("T-A"),
            dispatcher("install-a"),
        )]);

        let body = slack_command_body(
            "T-UNKNOWN",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-UNKNOWN/123/abc",
        );

        let error = resolver
            .resolve_command_ingress(&HeaderMap::new(), &body)
            .await
            .expect_err("a command from an unconfigured team must not resolve");

        assert!(
            matches!(error, SlackIngressError::InstallationNotFound),
            "error: {error}"
        );
    }

    #[tokio::test]
    async fn resolve_command_ingress_authenticates_before_unknown_team_rejection() {
        let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
            tenant_id("tenant-a"),
            installation_id("install-a"),
            SlackInstallationSelector::team("T-A"),
            failing_dispatcher(),
        )]);

        let body = slack_command_body(
            "T-UNKNOWN",
            "U123",
            "/pair",
            "https://hooks.slack.com/commands/T-UNKNOWN/123/abc",
        );

        let error = resolver
            .resolve_command_ingress(&HeaderMap::new(), &body)
            .await
            .expect_err("an unsigned unknown-team command must fail authentication first");

        assert!(
            matches!(
                error,
                SlackIngressError::Runner(RunnerError::AuthenticationFailed { .. })
            ),
            "error: {error}"
        );
    }
}
