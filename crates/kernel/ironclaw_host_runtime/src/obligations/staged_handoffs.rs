//! Staged secret and network handoffs — one of the three chartered owners of
//! the obligation module (PROPOSAL §6.5.9, CHECKLIST WS3).
//!
//! Obligation *preparation* stages material that a later, separate actor
//! consumes: one-shot runtime secret material and per-invocation network
//! policy. This module owns those staging stores and the credential-account
//! resolver port that feeds them. It performs no obligation orchestration
//! (see [`super::handler`]) and no process-lifecycle bookkeeping (see
//! [`super::process_store`]).

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    Timestamp,
    action::NetworkPolicy,
    capability::RuntimeCredentialAccountSetup,
    dispatch::CredentialStageError,
    ids::{CapabilityId, ExtensionId, SecretHandle, VendorId},
    resource::ResourceScope,
};
use ironclaw_secrets::{
    SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStoreError, SecretStorePort,
};
use secrecy::ExposeSecret;

/// Default maximum lifetime for one-shot runtime secret material staged in memory.
pub(crate) const DEFAULT_RUNTIME_SECRET_INJECTION_TTL: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub struct RuntimeCredentialAccountRequest<'a> {
    pub scope: &'a ResourceScope,
    pub provider: &'a VendorId,
    pub setup: &'a RuntimeCredentialAccountSetup,
    pub provider_scopes: &'a [String],
    pub requester_extension: &'a ExtensionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCredentialAccessSecret {
    pub scope: ResourceScope,
    pub handle: SecretHandle,
}

#[async_trait]
pub trait RuntimeCredentialAccountResolver: Send + Sync + fmt::Debug {
    /// Resolve the access-secret source for the requested product-auth account.
    ///
    /// Returns [`CredentialStageError::AuthRequired`] when the account is
    /// missing/unconfigured/expired/revoked (user must re-authenticate), or
    /// [`CredentialStageError::Backend`] for internal failures not attributable
    /// to user credentials. Shares its error vocabulary with the rest of the
    /// staged-credential surface (`ProductAuthCredentialStageError`,
    /// `GsuiteCredentialStageError`) so no per-layer error mapping is needed.
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError>;
}

/// Runtime secret material staged after `InjectSecretOnce` lease consumption.
///
/// The store is keyed by scoped invocation, capability, and handle. Runtime adapters
/// borrow staged material during dispatch; `complete_dispatch`/`abort` removes it
/// after the scoped capability finishes. Entries also expire after a short TTL so
/// abandoned handoffs from setup failures, cancellation, or adapter bugs cannot
/// remain usable indefinitely.
#[derive(Clone)]
pub(crate) struct RuntimeSecretInjectionStore {
    state: Arc<RuntimeSecretInjectionState>,
}

struct RuntimeSecretInjectionState {
    secrets: Mutex<HashMap<RuntimeSecretInjectionKey, RuntimeSecretInjectionEntry>>,
    ttl: Duration,
}

struct RuntimeSecretInjectionEntry {
    material: SecretMaterial,
    expires_at: Instant,
}

impl RuntimeSecretInjectionStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_ttl(ttl: Duration) -> Self {
        Self {
            state: Arc::new(RuntimeSecretInjectionState {
                secrets: Mutex::new(HashMap::new()),
                ttl,
            }),
        }
    }

    pub(crate) fn insert(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
        material: SecretMaterial,
    ) -> Result<(), RuntimeSecretInjectionStoreError> {
        let now = Instant::now();
        let expires_at = now.checked_add(self.state.ttl).unwrap_or(now);
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, now);
        secrets.insert(
            RuntimeSecretInjectionKey::new(scope, capability_id, handle),
            RuntimeSecretInjectionEntry {
                material,
                expires_at,
            },
        );
        Ok(())
    }

    pub(crate) fn take(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, RuntimeSecretInjectionStoreError> {
        let now = Instant::now();
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, now);
        Ok(secrets
            .remove(&RuntimeSecretInjectionKey::new(
                scope,
                capability_id,
                handle,
            ))
            .map(|entry| entry.material))
    }

    pub(crate) fn clone_material(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, RuntimeSecretInjectionStoreError> {
        let now = Instant::now();
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, now);
        Ok(secrets
            .get(&RuntimeSecretInjectionKey::new(
                scope,
                capability_id,
                handle,
            ))
            .map(|entry| SecretMaterial::from(entry.material.expose_secret())))
    }

    /// Discard all staged secrets for a scoped capability before process ownership exists.
    ///
    /// Background process lifecycle cleanup is guarded by a single-active-handoff
    /// invariant for the scoped capability; this method remains the abort/inline cleanup seam.
    pub(crate) fn discard_for_capability(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Result<(), RuntimeSecretInjectionStoreError> {
        let scope_key = RuntimeSecretInjectionScopeKey::new(scope, capability_id);
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, Instant::now());
        secrets.retain(|key, _| !key.matches_scope(&scope_key));
        Ok(())
    }

    pub(super) fn has_for_capability(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Result<bool, RuntimeSecretInjectionStoreError> {
        let scope_key = RuntimeSecretInjectionScopeKey::new(scope, capability_id);
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, Instant::now());
        Ok(secrets.keys().any(|key| key.matches_scope(&scope_key)))
    }

    #[cfg(test)]
    pub(super) fn prune_expired(&self) -> Result<usize, RuntimeSecretInjectionStoreError> {
        let mut secrets = self.lock()?;
        Ok(prune_expired_entries(&mut secrets, Instant::now()))
    }

    fn lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, HashMap<RuntimeSecretInjectionKey, RuntimeSecretInjectionEntry>>,
        RuntimeSecretInjectionStoreError,
    > {
        self.state
            .secrets
            .lock()
            .map_err(|_| RuntimeSecretInjectionStoreError::Unavailable)
    }
}

impl Default for RuntimeSecretInjectionStore {
    fn default() -> Self {
        Self::with_ttl(DEFAULT_RUNTIME_SECRET_INJECTION_TTL)
    }
}

impl fmt::Debug for RuntimeSecretInjectionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSecretInjectionStore")
            .field("secrets", &"[REDACTED]")
            .field("ttl", &self.state.ttl)
            .finish()
    }
}

fn prune_expired_entries(
    secrets: &mut HashMap<RuntimeSecretInjectionKey, RuntimeSecretInjectionEntry>,
    now: Instant,
) -> usize {
    let before = secrets.len();
    secrets.retain(|_, entry| entry.expires_at > now);
    before.saturating_sub(secrets.len())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeSecretInjectionStoreError {
    Unavailable,
}

impl fmt::Display for RuntimeSecretInjectionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("runtime secret injection store unavailable"),
        }
    }
}

impl std::error::Error for RuntimeSecretInjectionStoreError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuntimeSecretInjectionKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
    handle: String,
}

impl RuntimeSecretInjectionKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId, handle: &SecretHandle) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
            handle: handle.as_str().to_string(),
        }
    }

    fn matches_scope(&self, scope: &RuntimeSecretInjectionScopeKey) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
            && self.project_id == scope.project_id
            && self.mission_id == scope.mission_id
            && self.thread_id == scope.thread_id
            && self.invocation_id == scope.invocation_id
            && self.capability_id == scope.capability_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuntimeSecretInjectionScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
}

impl RuntimeSecretInjectionScopeKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
        }
    }
}

/// In-memory policy handoff from obligation handling to runtime adapters.
///
/// Policies are keyed by tenant/user/project/mission/thread/invocation scope and
/// capability id. Runtime adapters and host egress borrow the staged policy for
/// every network operation in the invocation; obligation completion/abort or
/// process lifecycle cleanup owns the final discard.
#[derive(Debug, Clone, Default)]
pub(crate) struct NetworkObligationPolicyStore {
    policies: Arc<Mutex<HashMap<NetworkPolicyKey, NetworkPolicy>>>,
}

impl NetworkObligationPolicyStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: NetworkPolicy,
    ) {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(NetworkPolicyKey::new(scope, capability_id), policy);
    }

    pub(crate) fn get(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<NetworkPolicy> {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&NetworkPolicyKey::new(scope, capability_id))
            .cloned()
    }

    pub(crate) fn take(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<NetworkPolicy> {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&NetworkPolicyKey::new(scope, capability_id))
    }

    /// Discard a staged policy for a scoped capability before process ownership exists.
    ///
    /// Background process lifecycle cleanup is guarded by a single-active-handoff
    /// invariant for the scoped capability; this method remains the abort/inline cleanup seam.
    pub(crate) fn discard_for_capability(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) {
        let _ = self.take(scope, capability_id);
    }

    pub(super) fn contains(&self, scope: &ResourceScope, capability_id: &CapabilityId) -> bool {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&NetworkPolicyKey::new(scope, capability_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NetworkPolicyKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
}

impl NetworkPolicyKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct SharedSecretStore(pub(crate) Arc<dyn SecretStorePort>);

#[async_trait]
impl SecretStorePort for SharedSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.0.put(scope, handle, material, expires_at).await
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        self.0.metadata(scope, handle).await
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        self.0.metadata_for_scope(scope).await
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        self.0.delete(scope, handle).await
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        self.0.lease_once(scope, handle).await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        self.0.consume(scope, lease_id).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        self.0.revoke(scope, lease_id).await
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        self.0.leases_for_scope(scope).await
    }
}

/// **Finding H2 — compile-time regression guard.**
///
/// The original H2 claim was that `RuntimeSecretInjectionStore`'s
/// `HashMap<_, RuntimeSecretInjectionEntry>` would bitwise-copy plaintext out
/// of the old bucket array on rehash and free it without zeroization. On
/// closer inspection that does *not* happen, because `SecretMaterial =
/// secrecy::SecretBox<str>`: the rehash moves a `Box<str>` pointer plus the
/// `Instant`, while the actual buffer stays at its original heap address
/// until `SecretBox::drop` zeroizes it.
///
/// The protection is real but depends on the staged entry's `material` field
/// being a `ZeroizeOnDrop` carrier. If it ever swaps to a non-zeroizing type
/// (plain `String`, `Vec<u8>`, etc.), the bitwise-copy concern returns. This
/// `const _: fn(...) = ...` references the field through a
/// `ZeroizeOnDrop`-bounded helper, so the swap is rejected at compile time
/// rather than only failing a test run. The function is never called — only
/// type-checked.
const _: fn(&RuntimeSecretInjectionEntry) = |entry| {
    fn require_zeroize_on_drop<T: ?Sized + secrecy::zeroize::ZeroizeOnDrop>(_: &T) {}
    require_zeroize_on_drop(&entry.material);
};
