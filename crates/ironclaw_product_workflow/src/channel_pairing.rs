//! Product-workflow-owned generic web-generated-code channel pairing
//! (extension-runtime §5.5's
//! second connect strategy, `WebGeneratedCode`).
//!
//! Direction is web→channel: IronClaw mints a short-lived single-use code the
//! WebUI presents (optionally as a vendor deep link); the channel's verified
//! webhook consumes it from a direct conversation and binds the sending
//! external actor to the code's Reborn user through the generic
//! installation-scoped identity bindings. Codes expire; gates don't — the
//! parked `BlockedAuth` run is provider-keyed (the extension id), so pairing
//! with the n-th rotated code still resumes it via the standard
//! auth-continuation fan-out.
//!
//! Everything here is vendor-blind: the extension declares the strategy and
//! optional deep-link template through its [`crate::ExtensionAccountSetupDescriptor`]
//! (assembled by the binary, never named by composition), template values
//! come from the extension's non-secret administrator fields, and the
//! consume half is invoked by the generic inbound sink for any unbound actor
//! on a `WebGeneratedCode` channel.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthFlowId, AuthProductScope, AuthProviderId,
    AuthSurface,
};
use ironclaw_conversations::{
    AdapterKind, ConversationActorPairingService, ExpectedExternalActorOwner, ExternalActorRef,
};
use ironclaw_filesystem::{
    CasApply, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    AgentId, ExtensionId, HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions,
    MountView, ProjectId, ResourceScope, ScopedPath, TenantId, UserId, VirtualPath,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, NormalizedInboundMessage, ProductTriggerReason,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CHANNEL_PAIRING_CODE_ALPHABET as PAIRING_CODE_ALPHABET,
    CHANNEL_PAIRING_CODE_LEN as PAIRING_CODE_LEN, ChannelConnectionNoticePolicy,
    ChannelConnectionRequirement, ChannelPairingCode, ChannelPairingIssue,
    ProductAuthContinuationDispatcher,
};

const PAIRING_TTL_MINUTES: i64 = 15;

/// Pairing snapshots keep at most this many total records per extension
/// (expired/consumed records beyond the bound are evicted oldest-first).
const PAIRING_SNAPSHOT_CAP: usize = 4096;

const PAIRING_ALIAS: &str = "/tenant-shared/channel-pairing";

fn mint_pairing_code() -> Result<ChannelPairingCode, ChannelPairingError> {
    use rand::RngExt;
    let mut rng = rand::rng();
    let code: String = (0..PAIRING_CODE_LEN)
        .map(|_| {
            let index = rng.random_range(0..PAIRING_CODE_ALPHABET.len());
            PAIRING_CODE_ALPHABET[index] as char
        })
        .collect();
    ChannelPairingCode::new(code).map_err(|_| ChannelPairingError::StoreUnavailable {
        reason: "generated pairing code failed canonical validation".to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ChannelPairingRecord {
    code: ChannelPairingCode,
    user_id: UserId,
    installation_id: AdapterInstallationId,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

impl ChannelPairingRecord {
    fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.consumed_at.is_none() && self.expires_at > now
    }
}

impl ChannelPairingConsumeOutcome {
    fn paired_user(&self) -> Option<&UserId> {
        match self {
            Self::Paired { user_id } | Self::AlreadyPairedSameUser { user_id } => Some(user_id),
            Self::AlreadyBoundToOtherUser | Self::ExpiredOrUnknown => None,
        }
    }
}

/// Durable pairing-completion intent for `(installation, user)`. Provider
/// ingress dispatches it synchronously through the generic lifecycle/fan-out
/// path; a transient failure leaves the intent intact and requests provider
/// redelivery with the same event identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingPairingCompletion {
    #[serde(default)]
    dispatch_id: AuthFlowId,
    installation_id: AdapterInstallationId,
    user_id: UserId,
    conversation_space_id: Option<String>,
    conversation_id: String,
    actor_kind: String,
    external_actor_id: String,
    #[serde(default = "Utc::now")]
    emitted_at: DateTime<Utc>,
}

/// Durable metadata for one identity binding written by pairing consume —
/// retained so user-scoped unpair can reconstruct the external actor for
/// conversation-actor cleanup without re-deriving vendor shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PairedActorRecord {
    provider_user_id: String,
    installation_id: AdapterInstallationId,
    actor_kind: String,
    external_actor_id: String,
    user_id: UserId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct PairingSnapshot {
    pairings: Vec<ChannelPairingRecord>,
    completions: Vec<PendingPairingCompletion>,
    paired_actors: Vec<PairedActorRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelPairingStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<ChannelPairingIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelPairingConsumeOutcome {
    Paired { user_id: UserId },
    AlreadyPairedSameUser { user_id: UserId },
    AlreadyBoundToOtherUser,
    ExpiredOrUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChannelPairingError {
    #[error("channel pairing store unavailable: {reason}")]
    StoreUnavailable { reason: String },
    #[error("this channel is not configured by an administrator yet")]
    NotConfigured,
    #[error("pairing continuation dispatch failed: {reason}")]
    ContinuationDispatch { reason: String },
}

fn store_unavailable(reason: impl std::fmt::Display) -> ChannelPairingError {
    ChannelPairingError::StoreUnavailable {
        reason: reason.to_string(),
    }
}

/// Resolves the extension installation visible to the pairing caller.
/// `Ok(None)` means the extension is not installed for that caller, so pairing
/// fails closed without minting a code.
#[async_trait]
pub trait ChannelPairingInstallationSource: Send + Sync {
    async fn current_installation(
        &self,
        caller: &UserId,
    ) -> Result<Option<AdapterInstallationId>, String>;
}

/// Resolves the non-secret template values a deep-link template may
/// reference (`{bot_username}` etc.) from administrator configuration.
/// `{code}` is always supplied by the service itself.
#[async_trait]
pub trait ChannelPairingTemplateValues: Send + Sync {
    async fn template_value(&self, handle: &str) -> Result<Option<String>, String>;
}

/// Result of binding an installation-scoped external actor to an IronClaw
/// user. The workflow owns first-writer-wins semantics; concrete identity
/// persistence remains an injected host adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelPairingIdentityBindOutcome {
    Bound,
    AlreadyBoundToOtherUser,
}

/// Provider-neutral identity binding operations needed by pairing.
#[async_trait]
pub trait ChannelPairingIdentityStore: Send + Sync {
    /// Stable host identity key retained in the existing pairing snapshot
    /// schema for rollback-compatible cleanup and idempotent replacement.
    fn binding_key(
        &self,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> String;

    async fn resolve_user(
        &self,
        extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> Result<Option<UserId>, String>;

    async fn bind_user(
        &self,
        extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
        user_id: UserId,
    ) -> Result<ChannelPairingIdentityBindOutcome, String>;

    async fn delete_user_bindings(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<(), String>;
}

/// Provider-neutral direct-target operations needed by pairing.
#[async_trait]
pub trait ChannelPairingDirectTargetStore: Send + Sync {
    async fn is_connected(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<bool, String>;

    async fn upsert(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
        external_actor_id: &str,
        conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<(), String>;

    async fn delete(&self, extension_id: &ExtensionId, user_id: &UserId) -> Result<(), String>;
}

/// Pairing pre-admission decision. `Consumed` means pairing and its durable
/// lifecycle continuation were accepted before the provider acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelPairingInterception {
    NotHandled,
    Consumed(ChannelPairingConsumeOutcome),
    RetryableFailure,
}

/// Product-owned pairing interception seam used by generic channel ingress.
#[async_trait]
pub trait ChannelPairingInterceptor: Send + Sync {
    async fn intercept(
        &self,
        installation_id: &AdapterInstallationId,
        message: &NormalizedInboundMessage,
    ) -> ChannelPairingInterception;
}

fn path_segment(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn tenant_path_segment(value: &str) -> &str {
    if value == ironclaw_host_api::SYSTEM_RESERVED_ID {
        "__system__"
    } else {
        value
    }
}

/// The `{handle}` placeholders in a deep-link template, minus the
/// service-supplied `{code}`.
fn template_handles(template: &str) -> Vec<String> {
    let mut handles = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let handle = &rest[start + 1..start + end];
        if handle != "code" && !handle.is_empty() && !handles.iter().any(|seen| seen == handle) {
            handles.push(handle.to_string());
        }
        rest = &rest[start + end + 1..];
    }
    handles
}

/// Filesystem-backed pairing state: one CAS-updated snapshot per extension
/// under `/tenant-shared/channel-pairing/{extension}.json`. In-memory test
/// composition rides the same store over `InMemoryBackend`
/// (arch-simplification §4.3: in-memory is a backend, not a store).
pub struct FilesystemChannelPairingStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
    extension_id: ExtensionId,
}

impl std::fmt::Debug for FilesystemChannelPairingStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemChannelPairingStore")
            .field("extension_id", &self.extension_id)
            .finish_non_exhaustive()
    }
}

fn pairing_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = tenant_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(PAIRING_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/channel-pairing"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

impl FilesystemChannelPairingStore {
    pub fn new(
        filesystem: Arc<dyn RootFilesystem>,
        tenant_id: TenantId,
        operator_user_id: UserId,
        extension_id: ExtensionId,
    ) -> Self {
        let scoped = Arc::new(ScopedFilesystem::new(filesystem, pairing_mount_view));
        Self {
            filesystem: scoped,
            scope: ResourceScope {
                tenant_id,
                user_id: operator_user_id,
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            extension_id,
        }
    }

    fn snapshot_path(&self) -> Result<ScopedPath, ChannelPairingError> {
        ScopedPath::new(format!(
            "{PAIRING_ALIAS}/{}.json",
            path_segment(self.extension_id.as_str())
        ))
        .map_err(store_unavailable)
    }

    async fn read_snapshot(&self) -> Result<PairingSnapshot, ChannelPairingError> {
        let path = self.snapshot_path()?;
        let versioned = match self.filesystem.get(&self.scope, &path).await {
            Ok(versioned) => versioned,
            Err(FilesystemError::NotFound { .. }) => return Ok(PairingSnapshot::default()),
            Err(error) => return Err(store_unavailable(error)),
        };
        let Some(versioned) = versioned else {
            return Ok(PairingSnapshot::default());
        };
        serde_json::from_slice(&versioned.entry.body).map_err(store_unavailable)
    }

    /// One bounded CAS read-modify-write over the extension's snapshot. The
    /// `apply` closure returns `(snapshot, output)`; retries re-read.
    async fn update_snapshot<T, F>(&self, apply: F) -> Result<T, ChannelPairingError>
    where
        T: Send + 'static,
        F: Fn(PairingSnapshot) -> (PairingSnapshot, T) + Send + Sync + Clone + 'static,
    {
        let path = self.snapshot_path()?;
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            |bytes| {
                serde_json::from_slice::<PairingSnapshot>(bytes).map_err(|error| error.to_string())
            },
            |snapshot| {
                serde_json::to_vec(snapshot)
                    .map(|body| Entry::bytes(body).with_content_type(ContentType::json()))
                    .map_err(|error| error.to_string())
            },
            move |current: Option<PairingSnapshot>| {
                let apply = apply.clone();
                async move {
                    let (mut snapshot, output) = apply(current.unwrap_or_default());
                    bound_snapshot(&mut snapshot);
                    Ok::<_, String>(CasApply::new(snapshot, output))
                }
            },
        )
        .await
        .map_err(store_unavailable)
    }
}

fn bound_snapshot(snapshot: &mut PairingSnapshot) {
    if snapshot.pairings.len() > PAIRING_SNAPSHOT_CAP {
        let excess = snapshot.pairings.len() - PAIRING_SNAPSHOT_CAP;
        snapshot.pairings.drain(0..excess);
    }
}

/// The generic pairing service for one `WebGeneratedCode` channel extension.
///
/// Provider identity for bindings and continuation fan-out is the extension
/// id itself (the same provider the parked `BlockedAuth` requirement names in
/// the extension's account-setup descriptor).
pub struct ChannelPairingService {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    extension_id: ExtensionId,
    connection_notices: ChannelConnectionNoticePolicy,
    connection_requirement: ChannelConnectionRequirement,
    deep_link_template: Option<String>,
    inbound_code_prefixes: Vec<String>,
    store: Arc<FilesystemChannelPairingStore>,
    installation: Arc<dyn ChannelPairingInstallationSource>,
    template_values: Arc<dyn ChannelPairingTemplateValues>,
    identity: Arc<dyn ChannelPairingIdentityStore>,
    continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    /// Conversation-actor pairing cleanup on unpair (disconnect parity
    /// with the OAuth channel lane): without it a re-paired chat resurrects
    /// its old thread and any run parked there.
    conversation_actor_pairings: Arc<dyn ConversationActorPairingService>,
    /// The canonical DM-target store outbound delivery reads: pairing
    /// completion records the direct conversation here.
    direct_targets: Arc<dyn ChannelPairingDirectTargetStore>,
}

impl std::fmt::Debug for ChannelPairingService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelPairingService")
            .field("extension_id", &self.extension_id)
            .finish_non_exhaustive()
    }
}

pub struct ChannelPairingServiceParts {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub extension_id: ExtensionId,
    pub connection_notices: ChannelConnectionNoticePolicy,
    pub connection_requirement: ChannelConnectionRequirement,
    pub deep_link_template: Option<String>,
    pub inbound_code_prefixes: Vec<String>,
    pub store: Arc<FilesystemChannelPairingStore>,
    pub installation: Arc<dyn ChannelPairingInstallationSource>,
    pub template_values: Arc<dyn ChannelPairingTemplateValues>,
    pub identity: Arc<dyn ChannelPairingIdentityStore>,
    pub continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    pub conversation_actor_pairings: Arc<dyn ConversationActorPairingService>,
    pub direct_targets: Arc<dyn ChannelPairingDirectTargetStore>,
}

impl ChannelPairingService {
    pub fn new(parts: ChannelPairingServiceParts) -> Self {
        Self {
            tenant_id: parts.tenant_id,
            agent_id: parts.agent_id,
            project_id: parts.project_id,
            extension_id: parts.extension_id,
            connection_notices: parts.connection_notices,
            connection_requirement: parts.connection_requirement,
            deep_link_template: parts.deep_link_template,
            inbound_code_prefixes: parts.inbound_code_prefixes,
            store: parts.store,
            installation: parts.installation,
            template_values: parts.template_values,
            identity: parts.identity,
            continuation: parts.continuation,
            conversation_actor_pairings: parts.conversation_actor_pairings,
            direct_targets: parts.direct_targets,
        }
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    pub fn connection_notices(&self) -> &ChannelConnectionNoticePolicy {
        &self.connection_notices
    }

    pub fn connection_requirement(&self) -> &ChannelConnectionRequirement {
        &self.connection_requirement
    }

    async fn resolve_deep_link(
        &self,
        code: &ChannelPairingCode,
    ) -> Result<Option<String>, ChannelPairingError> {
        let Some(template) = &self.deep_link_template else {
            return Ok(None);
        };
        let mut link = template.clone();
        link = link.replace("{code}", code.as_str());
        for handle in template_handles(template) {
            let Some(value) = self
                .template_values
                .template_value(&handle)
                .await
                .map_err(store_unavailable)?
            else {
                return Ok(None);
            };
            link = link.replace(&format!("{{{handle}}}"), &value);
        }
        // A template placeholder without a configured value means setup is
        // incomplete — presenting a broken link would strand the user, so the
        // issue falls back to code-only presentation.
        if link.contains('{') {
            return Ok(None);
        }
        Ok(Some(link))
    }

    async fn issue_for_record(
        &self,
        record: &ChannelPairingRecord,
    ) -> Result<ChannelPairingIssue, ChannelPairingError> {
        Ok(ChannelPairingIssue {
            code: record.code.clone(),
            deep_link: self.resolve_deep_link(&record.code).await?,
            expires_at: record.expires_at,
        })
    }

    /// Mint (or rotate) the caller's pairing code. Fails closed when the
    /// channel is not installed for the caller — no code is ever minted first.
    pub async fn issue_or_rotate(
        &self,
        caller: &UserId,
    ) -> Result<ChannelPairingIssue, ChannelPairingError> {
        let installation_id = self
            .installation
            .current_installation(caller)
            .await
            .map_err(store_unavailable)?
            .ok_or(ChannelPairingError::NotConfigured)?;
        let now = Utc::now();
        let record = ChannelPairingRecord {
            code: mint_pairing_code()?,
            user_id: caller.clone(),
            installation_id,
            created_at: now,
            expires_at: now + Duration::minutes(PAIRING_TTL_MINUTES),
            consumed_at: None,
        };
        let stored = record.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                // Rotation: at most one live code per user.
                snapshot.pairings.retain(|existing| {
                    existing.user_id != stored.user_id || existing.consumed_at.is_some()
                });
                snapshot.pairings.push(stored.clone());
                (snapshot, ())
            })
            .await?;
        self.issue_for_record(&record).await
    }

    pub async fn status_for(
        &self,
        caller: &UserId,
    ) -> Result<ChannelPairingStatus, ChannelPairingError> {
        let installation_id = self
            .installation
            .current_installation(caller)
            .await
            .map_err(store_unavailable)?;
        let connected = match &installation_id {
            Some(_installation_id) => self
                .direct_targets
                .is_connected(&self.extension_id, caller)
                .await
                .map_err(store_unavailable)?,
            None => false,
        };
        let pending = match (&installation_id, connected) {
            (Some(installation_id), false) => {
                let snapshot = self.store.read_snapshot().await?;
                let record = snapshot
                    .pairings
                    .iter()
                    .find(|record| {
                        &record.user_id == caller
                            && record.is_live(Utc::now())
                            && &record.installation_id == installation_id
                    })
                    .cloned();
                match record {
                    Some(record) => Some(self.issue_for_record(&record).await?),
                    None => None,
                }
            }
            _ => None,
        };
        Ok(ChannelPairingStatus { connected, pending })
    }

    /// Materialize the current pairing challenge without rotating a still-live
    /// code. Projection and channel-delivery replays therefore observe the
    /// same durable challenge as the WebUI pairing panel.
    pub async fn pending_or_issue(
        &self,
        caller: &UserId,
    ) -> Result<Option<ChannelPairingIssue>, ChannelPairingError> {
        let status = self.status_for(caller).await?;
        if status.connected {
            return Ok(None);
        }
        match status.pending {
            Some(issue) => Ok(Some(issue)),
            None => self.issue_or_rotate(caller).await.map(Some),
        }
    }

    /// Consume a code arriving over the verified webhook from a direct
    /// conversation.
    ///
    /// Ordering is claim-first: the code is atomically consumed (single
    /// winner) BEFORE any identity/target side effect, so two concurrent
    /// consumers of one code can never both bind. Completion (peer target +
    /// continuation dispatch) is idempotently repairable: a sender already
    /// bound to the code's user re-runs the completion effects — including on
    /// an already-consumed code — so a consume that failed after the claim is
    /// recovered by re-sending a code instead of stranding the blocked run.
    pub async fn consume(
        &self,
        authenticated_installation_id: &AdapterInstallationId,
        raw_code: &str,
        actor_kind: &str,
        external_actor_id: &str,
        conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<ChannelPairingConsumeOutcome, ChannelPairingError> {
        let Ok(code) = ChannelPairingCode::new(raw_code) else {
            return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
        };
        let snapshot = self.store.read_snapshot().await?;
        let Some(record) = snapshot
            .pairings
            .iter()
            .find(|record| record.code == code)
            .cloned()
        else {
            return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
        };
        if &record.installation_id != authenticated_installation_id {
            return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
        }
        match self
            .bound_user_for(&record.installation_id, external_actor_id)
            .await?
        {
            Some(existing) if existing == record.user_id => {
                // Repair path: burn the code if it is still live (whoever
                // wins — the sender is already bound), then re-run completion.
                let _already_burned = self.claim(&code).await?;
                self.complete_pairing(
                    &record,
                    actor_kind,
                    external_actor_id,
                    conversation_space_id,
                    conversation_id,
                )
                .await?;
                return Ok(ChannelPairingConsumeOutcome::AlreadyPairedSameUser {
                    user_id: existing,
                });
            }
            Some(_other) => {
                if !record.is_live(Utc::now()) {
                    return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
                }
                // Refusal keeps the live code intact for its owner.
                return Ok(ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser);
            }
            None => {}
        }
        // Single-consumer claim BEFORE identity/target writes: exactly one
        // concurrent consumer of a live code proceeds past this point.
        let Some(record) = self.claim(&code).await? else {
            return Ok(ChannelPairingConsumeOutcome::ExpiredOrUnknown);
        };
        match self
            .identity
            .bind_user(
                &self.extension_id,
                &record.installation_id,
                external_actor_id,
                record.user_id.clone(),
            )
            .await
            .map_err(store_unavailable)?
        {
            ChannelPairingIdentityBindOutcome::Bound => {}
            ChannelPairingIdentityBindOutcome::AlreadyBoundToOtherUser => {
                return Ok(ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser);
            }
        }
        self.complete_pairing(
            &record,
            actor_kind,
            external_actor_id,
            conversation_space_id,
            conversation_id,
        )
        .await?;
        Ok(ChannelPairingConsumeOutcome::Paired {
            user_id: record.user_id,
        })
    }

    /// Commit the idempotent completion intent shared by first-time pairing
    /// and the repair path, then publish the DM target. The product-owned
    /// interceptor dispatches and settles this durable intent before the
    /// provider ingress acknowledgement is returned.
    async fn complete_pairing(
        &self,
        record: &ChannelPairingRecord,
        actor_kind: &str,
        external_actor_id: &str,
        conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<(), ChannelPairingError> {
        let candidate = PendingPairingCompletion {
            dispatch_id: AuthFlowId::new(),
            installation_id: record.installation_id.clone(),
            user_id: record.user_id.clone(),
            conversation_space_id: conversation_space_id.map(str::to_string),
            conversation_id: conversation_id.to_string(),
            actor_kind: actor_kind.to_string(),
            external_actor_id: external_actor_id.to_string(),
            emitted_at: Utc::now(),
        };
        let provider_user_id = self
            .identity
            .binding_key(&record.installation_id, external_actor_id);
        let paired_actor = PairedActorRecord {
            provider_user_id,
            installation_id: record.installation_id.clone(),
            actor_kind: actor_kind.to_string(),
            external_actor_id: external_actor_id.to_string(),
            user_id: record.user_id.clone(),
        };
        let completion = self
            .store
            .update_snapshot(move |mut snapshot| {
                let completion = match snapshot.completions.iter_mut().find(|existing| {
                    existing.installation_id == candidate.installation_id
                        && existing.user_id == candidate.user_id
                }) {
                    Some(existing) => {
                        existing.conversation_space_id = candidate.conversation_space_id.clone();
                        existing.conversation_id = candidate.conversation_id.clone();
                        existing.actor_kind = candidate.actor_kind.clone();
                        existing.external_actor_id = candidate.external_actor_id.clone();
                        existing.clone()
                    }
                    None => {
                        snapshot.completions.push(candidate.clone());
                        candidate.clone()
                    }
                };
                snapshot
                    .paired_actors
                    .retain(|existing| existing.provider_user_id != paired_actor.provider_user_id);
                snapshot.paired_actors.push(paired_actor.clone());
                (snapshot, completion)
            })
            .await?;
        self.persist_dm_target(&completion).await
    }

    async fn finish_pending_for_user(&self, user_id: &UserId) -> Result<(), ChannelPairingError> {
        let snapshot = self.store.read_snapshot().await?;
        let pending: Vec<_> = snapshot
            .completions
            .iter()
            .filter(|completion| &completion.user_id == user_id)
            .cloned()
            .collect();
        for completion in pending {
            self.finish_pending_completion(completion).await?;
        }
        Ok(())
    }

    async fn finish_pending_completion(
        &self,
        completion: PendingPairingCompletion,
    ) -> Result<(), ChannelPairingError> {
        self.finish_pending_completion_with(
            completion,
            self.tenant_id.clone(),
            Arc::clone(&self.continuation),
        )
        .await
    }

    async fn finish_pending_completion_with(
        &self,
        completion: PendingPairingCompletion,
        tenant_id: TenantId,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        // Boxed: the continuation fan-out resumes parked runs through the
        // turn coordinator — a deep async subtree relative to this caller.
        Box::pin(self.dispatch_pairing_completion_with(&completion, tenant_id, continuation))
            .await?;
        let settled_dispatch_id = completion.dispatch_id;
        let settled_installation_id = completion.installation_id.clone();
        let settled_user_id = completion.user_id.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                snapshot.completions.retain(|existing| {
                    existing.dispatch_id != settled_dispatch_id
                        || existing.installation_id != settled_installation_id
                        || existing.user_id != settled_user_id
                });
                (snapshot, ())
            })
            .await
    }

    async fn persist_dm_target(
        &self,
        completion: &PendingPairingCompletion,
    ) -> Result<(), ChannelPairingError> {
        self.direct_targets
            .upsert(
                &self.extension_id,
                &completion.user_id,
                &completion.external_actor_id,
                completion.conversation_space_id.as_deref(),
                &completion.conversation_id,
            )
            .await
            .map_err(store_unavailable)
    }

    /// Unpair the caller: bindings and peer targets removed, pending code
    /// invalidated. Only this user is affected; history is retained.
    ///
    /// Deliberately independent of the current installation: an admin
    /// clearing the deployment must not orphan a user's durable bindings —
    /// those would silently resurrect the connection when the same channel is
    /// reconfigured even though the user disconnected.
    pub async fn unpair(&self, caller: &UserId) -> Result<(), ChannelPairingError> {
        self.identity
            .delete_user_bindings(&self.extension_id, caller)
            .await
            .map_err(store_unavailable)?;
        let adapter_kind =
            AdapterKind::new(self.extension_id.as_str()).map_err(store_unavailable)?;
        self.direct_targets
            .delete(&self.extension_id, caller)
            .await
            .map_err(store_unavailable)?;
        let caller_owned = caller.clone();
        let cleanup: Vec<PairedActorRecord> = self
            .store
            .update_snapshot(move |mut snapshot| {
                snapshot.pairings.retain(|record| {
                    record.user_id != caller_owned || record.consumed_at.is_some()
                });
                snapshot
                    .completions
                    .retain(|completion| completion.user_id != caller_owned);
                let (removed_actors, kept): (Vec<_>, Vec<_>) = snapshot
                    .paired_actors
                    .drain(..)
                    .partition(|actor| actor.user_id == caller_owned);
                snapshot.paired_actors = kept;
                (snapshot, removed_actors)
            })
            .await?;
        // Conversation-actor pairing cleanup (disconnect parity with the
        // OAuth channel lane): the workflow paired this external actor to the caller at
        // inbound; leaving that pairing behind re-attaches a re-paired user
        // to their old thread — and any run parked on it. The generic
        // identity store carries no binding epoch, so ownership is checked by
        // user id alone (accepted delta from the epoch-guarded host-state
        // shape this generalizes).
        for actor in cleanup {
            let actor_ref = ExternalActorRef::new(&actor.actor_kind, &actor.external_actor_id)
                .map_err(store_unavailable)?;
            let installation_id =
                ironclaw_conversations::AdapterInstallationId::new(actor.installation_id.as_str())
                    .map_err(store_unavailable)?;
            self.conversation_actor_pairings
                .unpair_external_actor_if_owned_by(
                    &self.tenant_id,
                    &adapter_kind,
                    &installation_id,
                    &actor_ref,
                    &ExpectedExternalActorOwner {
                        user_id: caller.clone(),
                        binding_epoch: None,
                    },
                )
                .await
                .map_err(store_unavailable)?;
        }
        Ok(())
    }

    /// Atomically consume the code (single winner): the CAS snapshot update
    /// marks it consumed and returns the pre-claim record exactly once.
    async fn claim(
        &self,
        code: &ChannelPairingCode,
    ) -> Result<Option<ChannelPairingRecord>, ChannelPairingError> {
        let code = code.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                let now = Utc::now();
                let mut claimed = None;
                for record in snapshot.pairings.iter_mut() {
                    if record.code == code && record.is_live(now) {
                        let pre_claim = record.clone();
                        record.consumed_at = Some(now);
                        claimed = Some(pre_claim);
                        break;
                    }
                }
                (snapshot, claimed)
            })
            .await
    }

    async fn bound_user_for(
        &self,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> Result<Option<UserId>, ChannelPairingError> {
        self.identity
            .resolve_user(&self.extension_id, installation_id, external_actor_id)
            .await
            .map_err(store_unavailable)
    }

    /// Emit the standard lifecycle continuation. Pairing is the final
    /// manifest-declared setup step, so activation is completed server-side
    /// before blocked runs resume; no browser or model-issued second action is
    /// part of the product state machine.
    async fn dispatch_pairing_completion_with(
        &self,
        completion: &PendingPairingCompletion,
        tenant_id: TenantId,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        let provider = AuthProviderId::new(self.extension_id.as_str()).map_err(|error| {
            ChannelPairingError::ContinuationDispatch {
                reason: error.to_string(),
            }
        })?;
        let event = AuthContinuationEvent {
            flow_id: completion.dispatch_id,
            scope: AuthProductScope::new(
                ResourceScope {
                    tenant_id,
                    user_id: completion.user_id.clone(),
                    agent_id: Some(self.agent_id.clone()),
                    project_id: self.project_id.clone(),
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
                AuthSurface::Callback,
            ),
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new(self.extension_id.as_str())
                    .map_err(|error| ChannelPairingError::ContinuationDispatch {
                        reason: error.to_string(),
                    })?,
            },
            provider,
            credential_account_id: None,
            emitted_at: completion.emitted_at,
        };
        continuation
            .dispatch_auth_continuation(event)
            .await
            .map_err(|error| ChannelPairingError::ContinuationDispatch {
                reason: error.to_string(),
            })
    }

    /// Re-dispatch pairing completion through the caller's real turn world.
    /// Integration groups execute runs in a shared turn store created after
    /// this composed service, unlike production where both use one store.
    /// Test-only: zero production bytes.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn finish_pending_for_user_with_for_test(
        &self,
        user_id: &UserId,
        tenant_id: TenantId,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        let snapshot = self.store.read_snapshot().await?;
        let pending: Vec<_> = snapshot
            .completions
            .iter()
            .filter(|completion| &completion.user_id == user_id)
            .cloned()
            .collect();
        for completion in pending {
            self.finish_pending_completion_with(
                completion,
                tenant_id.clone(),
                Arc::clone(&continuation),
            )
            .await?;
        }
        Ok(())
    }

    /// Settle pending completions through the service's configured dispatcher.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn finish_pending_for_user_for_test(
        &self,
        user_id: &UserId,
    ) -> Result<(), ChannelPairingError> {
        self.finish_pending_for_user(user_id).await
    }

    /// Inspect durable completion identities without exposing the private
    /// persistence snapshot shape to composition tests.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn pending_completion_dispatch_ids_for_test(
        &self,
    ) -> Result<Vec<AuthFlowId>, ChannelPairingError> {
        Ok(self
            .store
            .read_snapshot()
            .await?
            .completions
            .into_iter()
            .map(|completion| completion.dispatch_id)
            .collect())
    }

    /// Replace the continuation dispatcher in composition-level fault tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn replace_continuation_for_test(
        &mut self,
        continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    ) {
        self.continuation = continuation;
    }
}

/// Composition-built registry of pairing services keyed by extension id.
/// The factory populates it for every account-setup descriptor declaring the
/// `WebGeneratedCode` strategy; the channel-host assembly consults it when
/// wiring inbound sinks and actor resolution.
#[derive(Default)]
pub struct ChannelPairingRegistry {
    services: std::sync::RwLock<BTreeMap<String, Arc<ChannelPairingService>>>,
}

impl std::fmt::Debug for ChannelPairingRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ChannelPairingRegistry").finish()
    }
}

impl ChannelPairingRegistry {
    pub fn register(&self, service: Arc<ChannelPairingService>) {
        let mut services = self
            .services
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        services.insert(service.extension_id().as_str().to_string(), service);
    }

    pub fn get(&self, extension_id: &str) -> Option<Arc<ChannelPairingService>> {
        let services = self
            .services
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        services.get(extension_id).cloned()
    }
}

/// Extract a candidate pairing code from direct-message text. Bare codes are
/// always accepted; command-shaped syntax is granted only by the manifest.
fn candidate_code(text: &str, declared_prefixes: &[String]) -> Option<ChannelPairingCode> {
    let trimmed = text.trim();
    let candidate = declared_prefixes
        .iter()
        .find_map(|prefix| {
            let rest = trimmed.strip_prefix(prefix)?;
            rest.chars()
                .next()
                .is_some_and(char::is_whitespace)
                .then(|| rest.trim())
        })
        .unwrap_or(trimmed);
    ChannelPairingCode::new(candidate).ok()
}

#[async_trait]
impl ChannelPairingInterceptor for ChannelPairingService {
    async fn intercept(
        &self,
        installation_id: &AdapterInstallationId,
        message: &NormalizedInboundMessage,
    ) -> ChannelPairingInterception {
        if message.trigger != ProductTriggerReason::DirectChat {
            return ChannelPairingInterception::NotHandled;
        }
        let Some(code) = candidate_code(&message.text, &self.inbound_code_prefixes) else {
            return ChannelPairingInterception::NotHandled;
        };
        let outcome = Box::pin(self.consume(
            installation_id,
            code.as_str(),
            message.actor.kind(),
            message.actor.id(),
            message.conversation.space_id(),
            message.conversation.conversation_id(),
        ))
        .await;
        match outcome {
            Ok(outcome) => {
                if let Some(user_id) = outcome.paired_user()
                    && let Err(error) = self.finish_pending_for_user(user_id).await
                {
                    tracing::warn!(
                        target: "ironclaw::reborn::channel_pairing",
                        %error,
                        %user_id,
                        "pairing continuation was not accepted; requesting provider redelivery"
                    );
                    return ChannelPairingInterception::RetryableFailure;
                }
                tracing::debug!(
                    target: "ironclaw::reborn::channel_pairing",
                    outcome = ?outcome,
                    "pairing code consumed from channel inbound"
                );
                ChannelPairingInterception::Consumed(outcome)
            }
            Err(error) => {
                tracing::warn!(
                    target: "ironclaw::reborn::channel_pairing",
                    error = %error,
                    "pairing consume failed; provider redelivery requested"
                );
                ChannelPairingInterception::RetryableFailure
            }
        }
    }
}

/// The extension lifecycle's narrow connection-status probe: composition
/// connects the pairing service to the extension's declared account-setup
/// entry so activation can gate on the caller's pairing state without
/// holding the full pairing surface.
#[async_trait]
impl crate::AccountConnectionStatusSource for ChannelPairingService {
    async fn connected(
        &self,
        user_id: &UserId,
    ) -> Result<bool, crate::AccountConnectionStatusError> {
        let status = self.status_for(user_id).await.map_err(|error| {
            tracing::debug!(
                target: "ironclaw::reborn::channel_pairing",
                error = %error,
                "channel pairing status lookup failed"
            );
            crate::AccountConnectionStatusError::new("channel pairing status unavailable")
        })?;
        Ok(status.connected)
    }
}
