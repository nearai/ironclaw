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

use crate::{AdapterInstallationId, NormalizedInboundMessage, ProductTriggerReason};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthFlowId, AuthProductScope, AuthProviderId,
    AuthSurface,
};
use ironclaw_conversations::{
    AdapterKind, ConversationActorPairingService, ExpectedExternalActorOwner,
    ExternalActorBindingEpoch, ExternalActorRef,
};
use ironclaw_filesystem::{
    CasApply, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    AgentId, ExtensionId, HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions,
    MountView, ProjectId, ResourceScope, ScopedPath, TenantId, UserId, VirtualPath,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CHANNEL_PAIRING_CODE_ALPHABET as PAIRING_CODE_ALPHABET,
    CHANNEL_PAIRING_CODE_LEN as PAIRING_CODE_LEN, ChannelConnectionNoticePolicy,
    ChannelConnectionRequirement, ChannelPairingCode, ChannelPairingIssue,
    ExtensionAccountSetupDescriptor, ProductActorUserResolutionRequest, ProductActorUserResolver,
    ProductAuthContinuationDispatcher, ProductWorkflowError, ResolvedProductActorUser,
};

const PAIRING_TTL_MINUTES: i64 = 15;

/// Bounds completion ownership after a process loss. This lease protects only
/// one outbox dispatch attempt; it does not track or watch the resumed run.
const PAIRING_COMPLETION_LEASE_SECONDS: i64 = 30;
const PAIRING_COMPLETION_RENEWAL_SECONDS: u64 = 10;

/// Per-collection record bound for one extension snapshot. Replaceable pairing
/// codes are evicted oldest-first; live completion, actor, and cleanup records
/// reject new distinct admission rather than being silently discarded.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    binding_epoch: Option<ExternalActorBindingEpoch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lease: Option<PairingCompletionLease>,
}

/// Durable, cross-instance ownership of one pairing workflow commit.
/// Completion dispatch and final unpair identity deletion both use the claim
/// token to fence exact release and settlement when an old owner resumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PairingCompletionLease {
    owner_id: uuid::Uuid,
    claim_id: uuid::Uuid,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ClaimedPairingCompletion {
    completion: PendingPairingCompletion,
    lease: PairingCompletionLease,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    binding_epoch: Option<ExternalActorBindingEpoch>,
}

/// One durable user-scoped disconnect transaction.
///
/// Actor records stay attached until identity deletion succeeds. That makes
/// every earlier cleanup retryable and keeps the transaction visible to
/// issue/consume, while the lease fences concurrent workers at the final
/// identity commit point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingUnpairTransaction {
    transaction_id: uuid::Uuid,
    user_id: UserId,
    actors: Vec<PairedActorRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lease: Option<PairingCompletionLease>,
}

enum PendingUnpairClaim {
    Claimed(PairingCompletionLease),
    Busy,
    Settled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct PairingSnapshot {
    pairings: Vec<ChannelPairingRecord>,
    completions: Vec<PendingPairingCompletion>,
    paired_actors: Vec<PairedActorRecord>,
    /// Rollback-compatible legacy actor cleanup intents. New writes migrate
    /// these into `pending_unpair_transactions`.
    #[serde(default)]
    pending_unpairs: Vec<PairedActorRecord>,
    #[serde(default)]
    pending_unpair_transactions: Vec<PendingUnpairTransaction>,
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
    // Unlike replaceable pairing codes, these collections carry live work or
    // cleanup authority and must never be evicted here. Their admission paths
    // reject new records at the cap.
}

/// The generic pairing service for one `WebGeneratedCode` channel extension.
///
/// Provider identity for bindings and continuation fan-out is the extension
/// id itself (the same provider the parked `BlockedAuth` requirement names in
/// the extension's account-setup descriptor).
pub struct ChannelPairingService {
    completion_owner_id: uuid::Uuid,
    #[cfg(any(test, feature = "test-support"))]
    completion_lease_duration: Duration,
    #[cfg(any(test, feature = "test-support"))]
    completion_renewal_interval: std::time::Duration,
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

/// Runtime ports used by one manifest-declared channel pairing service.
/// Manifest data stays in the canonical [`ExtensionAccountSetupDescriptor`]
/// instead of being mirrored into another constructor DTO.
pub struct ChannelPairingServiceDependencies {
    pub store: Arc<FilesystemChannelPairingStore>,
    pub installation: Arc<dyn ChannelPairingInstallationSource>,
    pub template_values: Arc<dyn ChannelPairingTemplateValues>,
    pub identity: Arc<dyn ChannelPairingIdentityStore>,
    pub continuation: Arc<dyn ProductAuthContinuationDispatcher>,
    pub conversation_actor_pairings: Arc<dyn ConversationActorPairingService>,
    pub direct_targets: Arc<dyn ChannelPairingDirectTargetStore>,
}

mod service;

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

    /// Reconcile all registered completion outboxes without holding the
    /// synchronous registry lock across async persistence or dispatch.
    pub async fn reconcile_pending_completions(&self) -> Result<(), ChannelPairingError> {
        let services: Vec<_> = {
            let services = self
                .services
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            services.values().cloned().collect()
        };
        for service in services {
            service.reconcile_pending_completions().await?;
        }
        Ok(())
    }
}

/// Pairing-strategy channels resolve verified actors through the same durable
/// record that owns unpair fencing. Returning its generation ensures the
/// canonical conversation pairing cannot silently discard the exact-owner
/// epoch on the actor's next ordinary inbound message.
#[async_trait]
impl ProductActorUserResolver for ChannelPairingService {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<ResolvedProductActorUser>, ProductWorkflowError> {
        if request.adapter_id.as_str() != self.extension_id.as_str() {
            return Ok(None);
        }
        let user_id = self
            .identity
            .resolve_user(
                &self.extension_id,
                &request.installation_id,
                request.external_actor_ref.id(),
            )
            .await
            .map_err(|error| ProductWorkflowError::BindingResolutionFailed { reason: error })?;
        let Some(user_id) = user_id else {
            return Ok(None);
        };
        let snapshot = self.store.read_snapshot().await.map_err(|error| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: error.to_string(),
            }
        })?;
        let binding_epoch = snapshot
            .paired_actors
            .iter()
            .find(|actor| {
                actor.installation_id == request.installation_id
                    && actor.actor_kind == request.external_actor_ref.kind()
                    && actor.external_actor_id == request.external_actor_ref.id()
                    && actor.user_id == user_id
            })
            .and_then(|actor| actor.binding_epoch.clone());
        Ok(Some(match binding_epoch {
            Some(binding_epoch) => {
                ResolvedProductActorUser::with_binding_epoch(user_id, binding_epoch)
            }
            None => ResolvedProductActorUser::new(user_id),
        }))
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
