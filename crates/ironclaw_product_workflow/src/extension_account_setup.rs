//! Product-owned account-setup declarations for extension activation.
//!
//! Extension owners declare immutable setup metadata during composition and
//! connect the corresponding status source when their host surface is mounted.
//! Keeping those transitions separate makes a declared-but-unmounted host fail
//! closed without teaching the generic lifecycle about a concrete extension.

use std::collections::{BTreeMap, btree_map::Entry as MapEntry};
use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use ironclaw_host_api::{ExtensionId, RuntimeCredentialAuthRequirement, UserId};
use thiserror::Error;

use crate::ChannelConnectionRequirement;

/// A connection-status read failed inside the extension-owned host service.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("account connection status read failed: {reason}")]
pub struct AccountConnectionStatusError {
    reason: String,
}

impl AccountConnectionStatusError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Narrow per-user account-connection probe used during activation preflight.
#[async_trait]
pub trait AccountConnectionStatusSource: Send + Sync + std::fmt::Debug {
    async fn connected(&self, user_id: &UserId) -> Result<bool, AccountConnectionStatusError>;
}

/// Product-owned copy for a channel account's pairing lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConnectionNoticePolicy {
    pub connect_required: String,
    pub paired: String,
    pub already_paired_same_user: String,
    pub already_bound_to_other_user: String,
    pub expired_or_unknown: String,
}

impl ChannelConnectionNoticePolicy {
    pub fn generic(display_name: &str) -> Self {
        Self {
            connect_required: format!(
                "👋 To use {display_name}, connect it in the Ironclaw web app, then message me here again."
            ),
            paired: format!("✅ {display_name} is paired. You can talk to Ironclaw here."),
            already_paired_same_user: format!(
                "✅ This {display_name} account is already paired to you."
            ),
            already_bound_to_other_user: format!(
                "This {display_name} account is already paired to another Ironclaw user."
            ),
            expired_or_unknown: format!(
                "That {display_name} pairing code is invalid or expired. Get a fresh code from Ironclaw and try again."
            ),
        }
    }
}

/// Immutable product metadata for an extension whose activation depends on a
/// user-scoped external-account connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionAccountSetupDescriptor {
    pub extension_id: ExtensionId,
    pub auth_requirement: RuntimeCredentialAuthRequirement,
    pub connection_requirement: ChannelConnectionRequirement,
    pub connection_notices: ChannelConnectionNoticePolicy,
    pub activation_success_message: String,
    /// `WebGeneratedCode` presentation: an optional deep-link template with
    /// `{code}` plus non-secret `[channel.config]` field-handle placeholders
    /// (e.g. `https://vendor.example/{bot_username}?start={code}`). `None`
    /// presents the minted code alone.
    pub pairing_deep_link_template: Option<String>,
}

/// Sanitized lifecycle classification for an unavailable setup host or status
/// backend. The concrete backend error never crosses this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExtensionAccountSetupError {
    #[error("account setup host is unavailable for extension {extension_id}")]
    HostUnavailable { extension_id: ExtensionId },
    #[error("account connection status is unavailable for extension {extension_id}")]
    StatusUnavailable {
        extension_id: ExtensionId,
        #[source]
        source: AccountConnectionStatusError,
    },
}

#[derive(Debug)]
struct RegistryEntry {
    descriptor: ExtensionAccountSetupDescriptor,
    status_source: OnceLock<Arc<dyn AccountConnectionStatusSource>>,
}

impl RegistryEntry {
    fn new(descriptor: ExtensionAccountSetupDescriptor) -> Self {
        Self {
            descriptor,
            status_source: OnceLock::new(),
        }
    }
}

/// Owner-controlled registry for extension account-setup gates.
///
/// Declarations and source connections are single-assignment. This prevents a
/// later mount from silently replacing the setup contract or status authority.
#[derive(Clone, Default, Debug)]
pub struct ExtensionAccountSetupRegistry {
    entries: Arc<RwLock<BTreeMap<ExtensionId, RegistryEntry>>>,
}

impl ExtensionAccountSetupRegistry {
    /// Declares one immutable setup descriptor. Returns `false` when that
    /// extension was already declared.
    pub fn declare(&self, descriptor: ExtensionAccountSetupDescriptor) -> bool {
        let mut entries = write_entries(&self.entries);
        match entries.entry(descriptor.extension_id.clone()) {
            MapEntry::Vacant(entry) => {
                entry.insert(RegistryEntry::new(descriptor));
                true
            }
            MapEntry::Occupied(_) => false,
        }
    }

    /// Connects the extension-owned status source once. Returns `false` for an
    /// undeclared extension or when a source was already connected.
    pub fn connect(
        &self,
        extension_id: &ExtensionId,
        source: Arc<dyn AccountConnectionStatusSource>,
    ) -> bool {
        let entries = read_entries(&self.entries);
        entries
            .get(extension_id)
            .is_some_and(|entry| entry.status_source.set(source).is_ok())
    }

    pub fn descriptor(
        &self,
        extension_id: &ExtensionId,
    ) -> Option<ExtensionAccountSetupDescriptor> {
        read_entries(&self.entries)
            .get(extension_id)
            .map(|entry| entry.descriptor.clone())
    }

    /// Returns the requirement only when the declared account is disconnected.
    /// Undeclared extensions have no account gate; declared extensions whose
    /// host or status backend is unavailable fail closed.
    pub async fn missing_requirement(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<Option<RuntimeCredentialAuthRequirement>, ExtensionAccountSetupError> {
        let (descriptor, status_source) = {
            let entries = read_entries(&self.entries);
            let Some(entry) = entries.get(extension_id) else {
                return Ok(None);
            };
            let Some(status_source) = entry.status_source.get().map(Arc::clone) else {
                return Err(ExtensionAccountSetupError::HostUnavailable {
                    extension_id: extension_id.clone(),
                });
            };
            (entry.descriptor.clone(), status_source)
        };

        let connected = status_source.connected(user_id).await.map_err(|source| {
            ExtensionAccountSetupError::StatusUnavailable {
                extension_id: extension_id.clone(),
                source,
            }
        })?;
        Ok((!connected).then_some(descriptor.auth_requirement))
    }
}

fn read_entries(
    entries: &RwLock<BTreeMap<ExtensionId, RegistryEntry>>,
) -> RwLockReadGuard<'_, BTreeMap<ExtensionId, RegistryEntry>> {
    match entries.read() {
        Ok(entries) => entries,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_entries(
    entries: &RwLock<BTreeMap<ExtensionId, RegistryEntry>>,
) -> RwLockWriteGuard<'_, BTreeMap<ExtensionId, RegistryEntry>> {
    match entries.write() {
        Ok(entries) => entries,
        Err(poisoned) => poisoned.into_inner(),
    }
}
