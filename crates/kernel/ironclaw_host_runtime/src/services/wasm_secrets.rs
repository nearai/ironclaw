//! Production backing for the WASM guest `secret-exists` host import.
//!
//! Guests (including third-party registry/ironhub tools such as `attio`) use
//! `secret-exists` as their only credential probe: they abort with a
//! "credential not configured" failure when it returns `false`. Historically
//! every production invocation ran with [`WasmHostSecrets`] left at the
//! [`DenyWasmHostSecrets`] default, so the probe returned `false` even when a
//! real, staged credential was available — every such tool failed with an
//! opaque `operation_failed` before ever issuing a request.
//!
//! This implementation answers the probe from the per-invocation staged
//! secret injection store ([`RuntimeSecretInjectionStore`]): authorization
//! stages granted secret material under `(scope, capability_id, handle)` (see
//! `obligations/handler.rs`), so `exists` reports `true` exactly when a
//! non-empty credential for this invocation was actually leased and consumed
//! — not merely because a manifest declares the handle.

use std::sync::Arc;

use ironclaw_host_api::{
    ids::{CapabilityId, SecretHandle},
    resource::ResourceScope,
};
use ironclaw_wasm::WasmHostSecrets;
use secrecy::ExposeSecret;

use crate::obligations::RuntimeSecretInjectionStore;

/// Per-invocation `secret-exists` view over the staged secret injection store.
///
/// The store is keyed by the invocation's scope, capability, and the slot
/// handle the guest is expected to probe, so the view is closed over exactly
/// those three values. Material is read non-destructively
/// (`clone_material`) — answering the probe must not consume the one-shot
/// staged secret that the HTTP egress still needs.
#[derive(Debug)]
pub(crate) struct StagedWasmHostSecrets {
    store: Arc<RuntimeSecretInjectionStore>,
    scope: ResourceScope,
    capability_id: CapabilityId,
}

impl StagedWasmHostSecrets {
    pub(crate) fn new(
        store: Arc<RuntimeSecretInjectionStore>,
        scope: ResourceScope,
        capability_id: CapabilityId,
    ) -> Self {
        Self {
            store,
            scope,
            capability_id,
        }
    }
}

impl WasmHostSecrets for StagedWasmHostSecrets {
    fn exists(&self, name: &str) -> bool {
        let Ok(handle) = SecretHandle::new(name) else {
            // A guest probing a malformed handle name gets a truthful `false`.
            return false;
        };
        match self
            .store
            .clone_material(&self.scope, &self.capability_id, &handle)
        {
            // Fail closed: an empty staged credential is not a usable
            // credential — the guest must surface its own re-auth path rather
            // than send an empty key.
            Ok(Some(material)) => !material.expose_secret().is_empty(),
            // No staged material for this invocation (or a poisoned store
            // lock) means the credential was never authorized for this call.
            Ok(None) | Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::InvocationId;

    fn store() -> Arc<RuntimeSecretInjectionStore> {
        Arc::new(RuntimeSecretInjectionStore::new())
    }

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: ironclaw_host_api::ids::TenantId::new("test-tenant").unwrap(),
            user_id: ironclaw_host_api::ids::UserId::new("test-user").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn capability() -> CapabilityId {
        CapabilityId::new("attio.invoke").unwrap()
    }

    fn secrets() -> StagedWasmHostSecrets {
        StagedWasmHostSecrets::new(store(), scope(), capability())
    }

    #[test]
    fn exists_true_for_staged_non_empty_material() {
        let store = store();
        let scope = scope();
        let handle = SecretHandle::new("attio_api_key").unwrap();
        store
            .insert(
                &scope,
                &capability(),
                &handle,
                ironclaw_secrets::SecretMaterial::from("att-123"),
            )
            .expect("staging should succeed");
        let secrets = StagedWasmHostSecrets::new(store, scope, capability());
        assert!(secrets.exists("attio_api_key"));
    }

    #[test]
    fn exists_false_for_staged_empty_material() {
        let store = store();
        let scope = scope();
        let handle = SecretHandle::new("attio_api_key").unwrap();
        store
            .insert(
                &scope,
                &capability(),
                &handle,
                ironclaw_secrets::SecretMaterial::from(""),
            )
            .expect("staging should succeed");
        let secrets = StagedWasmHostSecrets::new(store, scope, capability());
        assert!(!secrets.exists("attio_api_key"));
    }

    #[test]
    fn exists_false_without_staged_material() {
        assert!(!secrets().exists("attio_api_key"));
    }

    #[test]
    fn exists_false_for_other_capability_or_scope() {
        let store = store();
        let scope = scope();
        let handle = SecretHandle::new("attio_api_key").unwrap();
        store
            .insert(
                &scope,
                &capability(),
                &handle,
                ironclaw_secrets::SecretMaterial::from("att-123"),
            )
            .expect("staging should succeed");
        let other_capability = CapabilityId::new("other.invoke").unwrap();
        assert!(
            !StagedWasmHostSecrets::new(Arc::clone(&store), scope.clone(), other_capability)
                .exists("attio_api_key")
        );
        assert!(!StagedWasmHostSecrets::new(store, scope, capability()).exists("other_secret"));
    }

    #[test]
    fn exists_false_for_malformed_handle() {
        assert!(!secrets().exists("not a valid handle"));
    }

    #[test]
    fn exists_reads_do_not_consume_staged_material() {
        let store = store();
        let scope = scope();
        let handle = SecretHandle::new("attio_api_key").unwrap();
        store
            .insert(
                &scope,
                &capability(),
                &handle,
                ironclaw_secrets::SecretMaterial::from("att-123"),
            )
            .expect("staging should succeed");
        let secrets = StagedWasmHostSecrets::new(store, scope, capability());
        assert!(secrets.exists("attio_api_key"));
        // The HTTP egress still needs the staged material after the probe.
        assert!(secrets.exists("attio_api_key"));
    }
}
