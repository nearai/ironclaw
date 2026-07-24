//! Generic web-generated-code channel pairing (extension-runtime §5.5's
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
//! optional deep-link template through its [`ExtensionAccountSetupDescriptor`]
//! (assembled by the binary, never named by composition), template values
//! come from the extension's non-secret `[channel.config]` fields, and the
//! consume half is invoked by the generic inbound sink for any unbound actor
//! on a `WebGeneratedCode` channel.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthFlowId, AuthProductScope, AuthProviderId,
    AuthSurface,
};
use ironclaw_conversations::{
    AdapterKind, ConversationActorPairingService, ExpectedExternalActorOwner, ExternalActorRef,
};
use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_filesystem::{
    CasApply, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    AgentId, ExtensionId, HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions,
    MountView, ProjectId, ResourceScope, ScopedPath, TenantId, UserId, VirtualPath,
};
use ironclaw_product::AdapterInstallationId;
use ironclaw_product::ChannelConnectionNoticePolicy;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::extension_host::channel_identity_store::path_segment;
use crate::product_auth::api::auth::RebornAuthContinuationDispatcher;
use crate::provider_identity::{
    RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
    RebornUserIdentityBindingDeleteStore, RebornUserIdentityBindingError,
    RebornUserIdentityBindingStore, RebornUserIdentityLookup, installation_scoped_provider_user_id,
};

/// Human-friendly unambiguous alphabet (no `0/O`, no `1/I`).
pub(crate) const PAIRING_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub(crate) const PAIRING_CODE_LEN: usize = 8;
pub(crate) const PAIRING_TTL_MINUTES: i64 = 15;

/// Pairing snapshots keep at most this many total records per extension
/// (expired/consumed records beyond the bound are evicted oldest-first).
const PAIRING_SNAPSHOT_CAP: usize = 4096;

const PAIRING_ALIAS: &str = "/tenant-shared/channel-pairing";

/// Canonical validated pairing-code value. External text is normalized
/// exactly once at parse (trim + uppercase).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct ChannelPairingCode(String);

impl ChannelPairingCode {
    pub(crate) fn parse(value: impl AsRef<str>) -> Result<Self, ChannelPairingCodeError> {
        let normalized = value.as_ref().trim().to_ascii_uppercase();
        if normalized.len() != PAIRING_CODE_LEN
            || !normalized
                .bytes()
                .all(|byte| PAIRING_CODE_ALPHABET.contains(&byte))
        {
            return Err(ChannelPairingCodeError);
        }
        Ok(Self(normalized))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn generated(value: String) -> Self {
        debug_assert_eq!(value.len(), PAIRING_CODE_LEN);
        Self(value)
    }
}

impl std::fmt::Display for ChannelPairingCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for ChannelPairingCode {
    type Error = ChannelPairingCodeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ChannelPairingCode> for String {
    fn from(value: ChannelPairingCode) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("invalid channel pairing code")]
pub(crate) struct ChannelPairingCodeError;

fn mint_pairing_code() -> ChannelPairingCode {
    use rand::RngExt;
    let mut rng = rand::rng();
    let code: String = (0..PAIRING_CODE_LEN)
        .map(|_| {
            let index = rng.random_range(0..PAIRING_CODE_ALPHABET.len());
            PAIRING_CODE_ALPHABET[index] as char
        })
        .collect();
    ChannelPairingCode::generated(code)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChannelPairingRecord {
    pub(crate) code: ChannelPairingCode,
    pub(crate) user_id: UserId,
    pub(crate) installation_id: AdapterInstallationId,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) consumed_at: Option<DateTime<Utc>>,
}

impl ChannelPairingRecord {
    pub(crate) fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.consumed_at.is_none() && self.expires_at > now
    }
}

/// A pairing-completion outbox entry: durable intent to (re)run the
/// completion effects for `(installation, user)` — DM-target write plus
/// blocked-run continuation — retried from status polling after a failed
/// dispatch or a process restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingPairingCompletion {
    installation_id: AdapterInstallationId,
    user_id: UserId,
    conversation_space_id: Option<String>,
    conversation_id: String,
    actor_kind: String,
    external_actor_id: String,
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
pub(crate) struct ChannelPairingIssue {
    pub(crate) code: ChannelPairingCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) deep_link: Option<String>,
    pub(crate) expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ChannelPairingStatus {
    pub(crate) connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pending: Option<ChannelPairingIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChannelPairingConsumeOutcome {
    Paired { user_id: UserId },
    AlreadyPairedSameUser { user_id: UserId },
    AlreadyBoundToOtherUser,
    ExpiredOrUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum ChannelPairingError {
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
pub(crate) trait ChannelPairingInstallationSource: Send + Sync {
    async fn current_installation(
        &self,
        caller: &UserId,
    ) -> Result<Option<AdapterInstallationId>, String>;
}

/// Resolves the non-secret template values a deep-link template may
/// reference (`{bot_username}` etc.) from the extension's channel config.
/// `{code}` is always supplied by the service itself.
#[async_trait]
pub(crate) trait ChannelPairingTemplateValues: Send + Sync {
    async fn template_values(&self) -> Result<BTreeMap<String, String>, String>;
}

/// [`ChannelPairingInstallationSource`] over the durable lifecycle store.
/// Pairing is setup work performed after install and before activation, so an
/// active-host snapshot is intentionally too narrow for this lookup.
pub(crate) struct StoredPairingInstallationSource {
    store: Arc<dyn ExtensionInstallationStore>,
    extension_id: ExtensionId,
}

impl StoredPairingInstallationSource {
    pub(crate) fn new(
        store: Arc<dyn ExtensionInstallationStore>,
        extension_id: ExtensionId,
    ) -> Self {
        Self {
            store,
            extension_id,
        }
    }
}

#[async_trait]
impl ChannelPairingInstallationSource for StoredPairingInstallationSource {
    async fn current_installation(
        &self,
        caller: &UserId,
    ) -> Result<Option<AdapterInstallationId>, String> {
        let installation = self
            .store
            .list_installations()
            .await
            .map_err(|error| format!("installation lookup failed: {error}"))?
            .into_iter()
            .find(|installation| {
                installation.extension_id() == &self.extension_id
                    && installation.owner().visible_to(caller)
            });
        installation
            .map(|installation| AdapterInstallationId::new(installation.installation_id().as_str()))
            .transpose()
            .map_err(|error| format!("installed installation id invalid: {error}"))
    }
}

/// [`ChannelPairingTemplateValues`] over the extension's saved non-secret
/// `[channel.config]` values: the deep-link template's handle placeholders
/// are parsed once at construction and resolved per read, so a configure
/// save takes effect on the next mint with no rewiring.
pub(crate) struct ChannelConfigPairingTemplateValues {
    channel_config: Arc<crate::extension_host::channel_config::ChannelConfigService>,
    extension_id: ExtensionId,
    handles: Vec<String>,
}

impl ChannelConfigPairingTemplateValues {
    pub(crate) fn new(
        channel_config: Arc<crate::extension_host::channel_config::ChannelConfigService>,
        extension_id: ExtensionId,
        template: Option<&str>,
    ) -> Self {
        let handles = template.map(template_handles).unwrap_or_default();
        Self {
            channel_config,
            extension_id,
            handles,
        }
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

#[async_trait]
impl ChannelPairingTemplateValues for ChannelConfigPairingTemplateValues {
    async fn template_values(&self) -> Result<BTreeMap<String, String>, String> {
        let mut values = BTreeMap::new();
        for handle in &self.handles {
            let value = self
                .channel_config
                .non_secret_value(&self.extension_id, handle)
                .await
                .map_err(|error| error.to_string())?;
            if let Some(value) = value {
                values.insert(handle.clone(), value);
            }
        }
        Ok(values)
    }
}

/// Filesystem-backed pairing state: one CAS-updated snapshot per extension
/// under `/tenant-shared/channel-pairing/{extension}.json`. In-memory test
/// composition rides the same store over `InMemoryBackend`
/// (arch-simplification §4.3: in-memory is a backend, not a store).
pub(crate) struct FilesystemChannelPairingStore {
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
    let tenant = crate::resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(PAIRING_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/channel-pairing"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

impl FilesystemChannelPairingStore {
    pub(crate) fn new(
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
pub(crate) struct ChannelPairingService {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    extension_id: ExtensionId,
    connection_notices: ChannelConnectionNoticePolicy,
    deep_link_template: Option<String>,
    store: Arc<FilesystemChannelPairingStore>,
    installation: Arc<dyn ChannelPairingInstallationSource>,
    template_values: Arc<dyn ChannelPairingTemplateValues>,
    identity_bind: Arc<dyn RebornUserIdentityBindingStore>,
    identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    identity_delete: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    continuation: Arc<dyn RebornAuthContinuationDispatcher>,
    /// Conversation-actor pairing cleanup on unpair (disconnect parity
    /// with the OAuth channel lane): without it a re-paired chat resurrects
    /// its old thread and any run parked there.
    conversation_actor_pairings: Arc<dyn ConversationActorPairingService>,
    /// The canonical DM-target store outbound delivery reads: pairing
    /// completion records the direct conversation here.
    dm_targets: Arc<ironclaw_extension_host::FilesystemChannelDmTargetStore>,
}

impl std::fmt::Debug for ChannelPairingService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelPairingService")
            .field("extension_id", &self.extension_id)
            .finish_non_exhaustive()
    }
}

pub(crate) struct ChannelPairingServiceParts {
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
    pub(crate) project_id: Option<ProjectId>,
    pub(crate) extension_id: ExtensionId,
    pub(crate) connection_notices: ChannelConnectionNoticePolicy,
    pub(crate) deep_link_template: Option<String>,
    pub(crate) store: Arc<FilesystemChannelPairingStore>,
    pub(crate) installation: Arc<dyn ChannelPairingInstallationSource>,
    pub(crate) template_values: Arc<dyn ChannelPairingTemplateValues>,
    pub(crate) identity_bind: Arc<dyn RebornUserIdentityBindingStore>,
    pub(crate) identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    pub(crate) identity_delete: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    pub(crate) continuation: Arc<dyn RebornAuthContinuationDispatcher>,
    pub(crate) conversation_actor_pairings: Arc<dyn ConversationActorPairingService>,
    pub(crate) dm_targets: Arc<ironclaw_extension_host::FilesystemChannelDmTargetStore>,
}

impl ChannelPairingService {
    pub(crate) fn new(parts: ChannelPairingServiceParts) -> Self {
        Self {
            tenant_id: parts.tenant_id,
            agent_id: parts.agent_id,
            project_id: parts.project_id,
            extension_id: parts.extension_id,
            connection_notices: parts.connection_notices,
            deep_link_template: parts.deep_link_template,
            store: parts.store,
            installation: parts.installation,
            template_values: parts.template_values,
            identity_bind: parts.identity_bind,
            identity_lookup: parts.identity_lookup,
            identity_delete: parts.identity_delete,
            continuation: parts.continuation,
            conversation_actor_pairings: parts.conversation_actor_pairings,
            dm_targets: parts.dm_targets,
        }
    }

    pub(crate) fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    pub(crate) fn connection_notices(&self) -> &ChannelConnectionNoticePolicy {
        &self.connection_notices
    }

    async fn resolve_deep_link(
        &self,
        code: &ChannelPairingCode,
    ) -> Result<Option<String>, ChannelPairingError> {
        let Some(template) = &self.deep_link_template else {
            return Ok(None);
        };
        let values = self
            .template_values
            .template_values()
            .await
            .map_err(store_unavailable)?;
        let mut link = template.clone();
        link = link.replace("{code}", code.as_str());
        for (handle, value) in &values {
            link = link.replace(&format!("{{{handle}}}"), value);
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
    pub(crate) async fn issue_or_rotate(
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
            code: mint_pairing_code(),
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

    pub(crate) async fn status_for(
        &self,
        caller: &UserId,
    ) -> Result<ChannelPairingStatus, ChannelPairingError> {
        let installation_id = self
            .installation
            .current_installation(caller)
            .await
            .map_err(store_unavailable)?;
        let connected = match &installation_id {
            Some(installation_id) => {
                let _ = installation_id;
                let snapshot = self.store.read_snapshot().await?;
                if let Some(pending) = snapshot.completions.iter().find(|completion| {
                    &completion.installation_id == installation_id && &completion.user_id == caller
                }) {
                    // Outbox retry: a completion that failed after the claim
                    // (or a restart between claim and completion) re-runs
                    // here; the user never re-sends a consumed code.
                    self.finish_pending_completion(pending.clone()).await?;
                }
                self.dm_targets
                    .load(self.extension_id.as_str(), caller)
                    .await
                    .map_err(store_unavailable)?
                    .is_some()
            }
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
    pub(crate) async fn consume(
        &self,
        authenticated_installation_id: &AdapterInstallationId,
        raw_code: &str,
        actor_kind: &str,
        external_actor_id: &str,
        conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<ChannelPairingConsumeOutcome, ChannelPairingError> {
        let Ok(code) = ChannelPairingCode::parse(raw_code) else {
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
        let provider_user_id =
            installation_scoped_provider_user_id(&record.installation_id, external_actor_id);
        match self.bound_user_for(&provider_user_id).await? {
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
        let binding = RebornUserIdentityBinding {
            provider: self.identity_provider()?,
            provider_user_id: RebornIdentityProviderUserId::new(provider_user_id.clone())
                .map_err(store_unavailable)?,
            user_id: record.user_id.clone(),
        };
        match self.identity_bind.bind_user_identity(binding).await {
            Ok(()) => {}
            Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound) => {
                return Ok(ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser);
            }
            Err(error) => return Err(store_unavailable(error)),
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

    /// The idempotent completion tail shared by first-time pairing and the
    /// repair path: persist the outbox entry, then dispatch the blocked-run
    /// continuation and record the peer delivery target.
    async fn complete_pairing(
        &self,
        record: &ChannelPairingRecord,
        actor_kind: &str,
        external_actor_id: &str,
        conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<(), ChannelPairingError> {
        let completion = PendingPairingCompletion {
            installation_id: record.installation_id.clone(),
            user_id: record.user_id.clone(),
            conversation_space_id: conversation_space_id.map(str::to_string),
            conversation_id: conversation_id.to_string(),
            actor_kind: actor_kind.to_string(),
            external_actor_id: external_actor_id.to_string(),
        };
        let provider_user_id =
            installation_scoped_provider_user_id(&record.installation_id, external_actor_id);
        let paired_actor = PairedActorRecord {
            provider_user_id,
            installation_id: record.installation_id.clone(),
            actor_kind: actor_kind.to_string(),
            external_actor_id: external_actor_id.to_string(),
            user_id: record.user_id.clone(),
        };
        let stored = completion.clone();
        self.store
            .update_snapshot(move |mut snapshot| {
                snapshot.completions.retain(|existing| {
                    existing.installation_id != stored.installation_id
                        || existing.user_id != stored.user_id
                });
                snapshot.completions.push(stored.clone());
                snapshot
                    .paired_actors
                    .retain(|existing| existing.provider_user_id != paired_actor.provider_user_id);
                snapshot.paired_actors.push(paired_actor.clone());
                (snapshot, ())
            })
            .await?;
        self.finish_pending_completion(completion).await
    }

    async fn finish_pending_completion(
        &self,
        completion: PendingPairingCompletion,
    ) -> Result<(), ChannelPairingError> {
        // Boxed: the continuation fan-out resumes parked runs through the
        // turn coordinator — a deep async subtree relative to this caller.
        Box::pin(self.dispatch_pairing_completion(&completion.user_id)).await?;
        self.dm_targets
            .upsert(
                self.extension_id.as_str(),
                &completion.user_id,
                completion.external_actor_id.clone(),
                ironclaw_extension_host::dm_target_payload(
                    completion.conversation_space_id.as_deref(),
                    &completion.conversation_id,
                ),
            )
            .await
            .map_err(store_unavailable)?;
        self.store
            .update_snapshot(move |mut snapshot| {
                snapshot.completions.retain(|existing| {
                    existing.installation_id != completion.installation_id
                        || existing.user_id != completion.user_id
                });
                (snapshot, ())
            })
            .await
    }

    /// Unpair the caller: bindings and peer targets removed, pending code
    /// invalidated. Only this user is affected; history is retained.
    ///
    /// Deliberately independent of the current installation: an admin
    /// clearing the deployment must not orphan a user's durable bindings —
    /// those would silently resurrect the connection when the same channel is
    /// reconfigured even though the user disconnected.
    pub(crate) async fn unpair(&self, caller: &UserId) -> Result<(), ChannelPairingError> {
        let provider = self.identity_provider()?;
        let removed = self
            .identity_delete
            .delete_user_identity_bindings_for_user(provider.as_str(), caller, None)
            .await
            .map_err(store_unavailable)?;
        let adapter_kind =
            AdapterKind::new(self.extension_id.as_str()).map_err(store_unavailable)?;
        self.dm_targets
            .delete(self.extension_id.as_str(), caller)
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
        let _ = removed;
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
        provider_user_id: &str,
    ) -> Result<Option<UserId>, ChannelPairingError> {
        self.identity_lookup
            .resolve_user_identity(self.extension_id.as_str(), provider_user_id)
            .await
            .map_err(store_unavailable)
    }

    fn identity_provider(&self) -> Result<RebornIdentityProviderId, ChannelPairingError> {
        RebornIdentityProviderId::new(self.extension_id.as_str()).map_err(store_unavailable)
    }

    /// Emit the standard auth-continuation completion so the
    /// `BlockedAuthResumeFanout` resumes every run parked on this extension's
    /// provider for this user. `SetupOnly` deliberately: the resumed run
    /// re-runs `extension_activate` and re-checks pairedness itself.
    async fn dispatch_pairing_completion(
        &self,
        user_id: &UserId,
    ) -> Result<(), ChannelPairingError> {
        self.dispatch_pairing_completion_with(
            user_id,
            self.tenant_id.clone(),
            Arc::clone(&self.continuation),
        )
        .await
    }

    async fn dispatch_pairing_completion_with(
        &self,
        user_id: &UserId,
        tenant_id: TenantId,
        continuation: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        let provider = AuthProviderId::new(self.extension_id.as_str()).map_err(|error| {
            ChannelPairingError::ContinuationDispatch {
                reason: error.to_string(),
            }
        })?;
        let event = AuthContinuationEvent {
            flow_id: AuthFlowId::new(),
            scope: AuthProductScope::new(
                ResourceScope {
                    tenant_id,
                    user_id: user_id.clone(),
                    agent_id: Some(self.agent_id.clone()),
                    project_id: self.project_id.clone(),
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
                AuthSurface::Callback,
            ),
            continuation: AuthContinuationRef::SetupOnly,
            provider,
            credential_account_id: None,
            emitted_at: Utc::now(),
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
    #[allow(dead_code)]
    pub(crate) async fn dispatch_pairing_completion_with_for_test(
        &self,
        user_id: &UserId,
        tenant_id: TenantId,
        continuation: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Result<(), ChannelPairingError> {
        self.dispatch_pairing_completion_with(user_id, tenant_id, continuation)
            .await
    }
}

/// Composition-built registry of pairing services keyed by extension id.
/// The factory populates it for every account-setup descriptor declaring the
/// `WebGeneratedCode` strategy; the channel-host assembly consults it when
/// wiring inbound sinks and actor resolution.
#[derive(Default)]
pub(crate) struct ChannelPairingRegistry {
    services: std::sync::RwLock<BTreeMap<String, Arc<ChannelPairingService>>>,
}

impl std::fmt::Debug for ChannelPairingRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ChannelPairingRegistry").finish()
    }
}

impl ChannelPairingRegistry {
    pub(crate) fn register(&self, service: Arc<ChannelPairingService>) {
        let mut services = self
            .services
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        services.insert(service.extension_id().as_str().to_string(), service);
    }

    pub(crate) fn get(&self, extension_id: &str) -> Option<Arc<ChannelPairingService>> {
        let services = self
            .services
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        services.get(extension_id).cloned()
    }
}

/// Extract a candidate pairing code from a direct-message text: either the
/// vendor-conventional `/start <CODE>` deep-link payload or a bare code.
fn candidate_code(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let candidate = match trimmed.strip_prefix("/start") {
        Some(rest) => rest.trim(),
        None => trimmed,
    };
    (candidate.len() == PAIRING_CODE_LEN
        && candidate
            .bytes()
            .all(|byte| PAIRING_CODE_ALPHABET.contains(&byte.to_ascii_uppercase())))
    .then_some(candidate)
}

#[async_trait]
impl crate::extension_host::extension_ingress::ChannelPairingInterceptor for ChannelPairingService {
    async fn intercept(
        &self,
        installation_id: &AdapterInstallationId,
        message: &ironclaw_product::NormalizedInboundMessage,
    ) -> crate::extension_host::extension_ingress::ChannelPairingInterception {
        use crate::extension_host::extension_ingress::ChannelPairingInterception;

        if message.trigger != ironclaw_product::ProductTriggerReason::DirectChat {
            return ChannelPairingInterception::NotHandled;
        }
        let Some(code) = candidate_code(&message.text) else {
            return ChannelPairingInterception::NotHandled;
        };
        let provider_user_id =
            installation_scoped_provider_user_id(installation_id, message.actor.id());
        match self.bound_user_for(&provider_user_id).await {
            Ok(None) => {}
            Ok(Some(_)) => {
                // A bound sender re-sending a code is the documented repair
                // path — service it (idempotent completion) and swallow.
            }
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::reborn::channel_pairing",
                    error = %error,
                    "pairing interceptor identity lookup failed; message flows to admission"
                );
                return ChannelPairingInterception::NotHandled;
            }
        }
        let outcome = Box::pin(self.consume(
            installation_id,
            code,
            message.actor.kind(),
            message.actor.id(),
            message.conversation.space_id(),
            message.conversation.conversation_id(),
        ))
        .await;
        match outcome {
            Ok(outcome) => {
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
                    "pairing consume failed; code message acknowledged without feedback"
                );
                ChannelPairingInterception::Failed
            }
        }
    }
}

/// The extension lifecycle's narrow connection-status probe: composition
/// connects the pairing service to the extension's declared account-setup
/// entry so activation can gate on the caller's pairing state without
/// holding the full pairing surface.
#[async_trait]
impl ironclaw_product::AccountConnectionStatusSource for ChannelPairingService {
    async fn connected(
        &self,
        user_id: &UserId,
    ) -> Result<bool, ironclaw_product::AccountConnectionStatusError> {
        let status = self.status_for(user_id).await.map_err(|error| {
            tracing::debug!(
                target: "ironclaw::reborn::channel_pairing",
                error = %error,
                "channel pairing status lookup failed"
            );
            ironclaw_product::AccountConnectionStatusError::new(
                "channel pairing status unavailable",
            )
        })?;
        Ok(status.connected)
    }
}

#[cfg(test)]
mod tests;
