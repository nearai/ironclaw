//! Mediated obligation handling for the kernel service graph.
//!
//! Obligation work has three distinct owners, and this module is split along
//! them so that no single file fuses them again (PROPOSAL §6.5.9, CHECKLIST
//! WS3 — "splits internally into its three chartered owners"):
//!
//! | Owner | Module | Responsibility |
//! |---|---|---|
//! | obligation handling | [`handler`] | which obligations apply, and what each does before/after dispatch |
//! | staged secret/network handoffs | [`staged_handoffs`] | one-shot secret material and per-invocation network policy staged for a later consumer |
//! | process-obligation store | [`process_store`] | discarding handoffs and reconciling reservations once a capability process has started |
//!
//! [`BuiltinObligationServices`] below is the assembly seam that binds the
//! three together for composition; it is deliberately the only place that names
//! all three owners at once.

use std::{fmt, sync::Arc};

use ironclaw_events::AuditSink;
use ironclaw_host_api::http::RuntimeHttpEgress;
use ironclaw_network::NetworkHttpEgress;
use ironclaw_processes::ProcessRuntimePort;
use ironclaw_resources::ResourceGovernor;
use ironclaw_secrets::SecretStorePort;

use crate::{
    ToolCallHttpEgress,
    http_body::{RuntimeHttpBodyStore, UnsupportedRuntimeHttpBodyStore},
};

mod handler;
mod process_store;
mod staged_handoffs;

#[cfg(test)]
mod tests;

pub use handler::{BuiltinObligationHandler, LEAK_REDACT_FAILED_CODE};
pub use process_store::ProcessObligationLifecycleStore;
pub use staged_handoffs::{
    RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver,
};

pub(crate) use handler::secret_owner_scope;
pub(crate) use staged_handoffs::{
    NetworkObligationPolicyStore, RuntimeSecretInjectionStore, SharedSecretStore,
};

/// Host-runtime-owned backing services for a fully configured built-in obligation handler.
///
/// This value is the production composition seam for obligation handling. It
/// keeps the in-memory network-policy and runtime-secret handoff stores alive
/// outside the handler so runtime adapters can consume the exact staged state
/// that [`BuiltinObligationHandler`] prepares before dispatch.
#[derive(Clone)]
pub struct BuiltinObligationServices {
    audit_sink: Arc<dyn AuditSink>,
    network_policies: Arc<NetworkObligationPolicyStore>,
    secret_store: Arc<dyn SecretStorePort>,
    secret_injections: Arc<RuntimeSecretInjectionStore>,
    resource_governor: Arc<dyn ResourceGovernor>,
    credential_account_resolver: Option<Arc<dyn RuntimeCredentialAccountResolver>>,
}

impl BuiltinObligationServices {
    pub fn new(
        audit_sink: Arc<dyn AuditSink>,
        secret_store: Arc<dyn SecretStorePort>,
        resource_governor: Arc<dyn ResourceGovernor>,
    ) -> Self {
        Self::with_handoff_stores(
            audit_sink,
            Arc::new(NetworkObligationPolicyStore::new()),
            secret_store,
            Arc::new(RuntimeSecretInjectionStore::new()),
            resource_governor,
        )
    }

    pub(crate) fn with_handoff_stores(
        audit_sink: Arc<dyn AuditSink>,
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_store: Arc<dyn SecretStorePort>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        resource_governor: Arc<dyn ResourceGovernor>,
    ) -> Self {
        Self {
            audit_sink,
            network_policies,
            secret_store,
            secret_injections,
            resource_governor,
            credential_account_resolver: None,
        }
    }

    pub fn with_credential_account_resolver<T>(mut self, resolver: Arc<T>) -> Self
    where
        T: RuntimeCredentialAccountResolver + 'static,
    {
        self.credential_account_resolver = Some(resolver);
        self
    }

    pub fn with_credential_account_resolver_dyn(
        mut self,
        resolver: Arc<dyn RuntimeCredentialAccountResolver>,
    ) -> Self {
        self.credential_account_resolver = Some(resolver);
        self
    }

    pub fn audit_sink(&self) -> Arc<dyn AuditSink> {
        self.audit_sink.clone()
    }

    pub fn secret_store(&self) -> Arc<dyn SecretStorePort> {
        self.secret_store.clone()
    }

    pub fn resource_governor(&self) -> Arc<dyn ResourceGovernor> {
        self.resource_governor.clone()
    }

    /// Builds host HTTP egress over this service graph's private handoff stores.
    /// Callers can supply concrete network transport without receiving mutable
    /// access to staged policy or secret material.
    pub fn host_http_egress<N>(
        &self,
        network: N,
    ) -> impl RuntimeHttpEgress + ToolCallHttpEgress + use<N>
    where
        N: NetworkHttpEgress + 'static,
    {
        self.host_http_egress_with_body_store(network, Arc::new(UnsupportedRuntimeHttpBodyStore))
    }

    pub fn host_http_egress_with_body_store<N, T>(
        &self,
        network: N,
        body_store: Arc<T>,
    ) -> impl RuntimeHttpEgress + ToolCallHttpEgress + use<N, T>
    where
        N: NetworkHttpEgress + 'static,
        T: RuntimeHttpBodyStore + 'static,
    {
        let body_store: Arc<dyn RuntimeHttpBodyStore> = body_store;
        crate::HostHttpEgressService::production(
            network,
            SharedSecretStore(self.secret_store.clone()),
            self.network_policies.clone(),
            self.secret_injections.clone(),
            body_store,
        )
    }

    pub fn process_obligation_lifecycle_store<S>(
        &self,
        inner: Arc<S>,
    ) -> ProcessObligationLifecycleStore
    where
        S: ProcessRuntimePort + 'static,
    {
        ProcessObligationLifecycleStore::new(
            inner,
            self.network_policies.clone(),
            self.secret_injections.clone(),
            self.resource_governor.clone(),
        )
    }

    pub fn process_obligation_lifecycle_store_dyn(
        &self,
        inner: Arc<dyn ProcessRuntimePort>,
    ) -> ProcessObligationLifecycleStore {
        ProcessObligationLifecycleStore::from_dyn(
            inner,
            self.network_policies.clone(),
            self.secret_injections.clone(),
            self.resource_governor.clone(),
        )
    }

    pub fn obligation_handler(&self) -> BuiltinObligationHandler {
        let handler = BuiltinObligationHandler::new()
            .with_audit_sink_dyn(self.audit_sink.clone())
            .with_network_policy_store(self.network_policies.clone())
            .with_secret_store_dyn(self.secret_store.clone())
            .with_secret_injection_store(self.secret_injections.clone())
            .with_resource_governor_dyn(self.resource_governor.clone());
        match &self.credential_account_resolver {
            Some(resolver) => handler.with_credential_account_resolver_dyn(Arc::clone(resolver)),
            None => handler,
        }
    }
}

impl fmt::Debug for BuiltinObligationServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuiltinObligationServices")
            .field("audit_sink", &"<audit_sink>")
            .field("network_policies", &self.network_policies)
            .field("secret_store", &"[REDACTED]")
            .field("secret_injections", &self.secret_injections)
            .field("resource_governor", &"<resource_governor>")
            .field(
                "credential_account_resolver",
                &self
                    .credential_account_resolver
                    .as_ref()
                    .map(|_| "<resolver>"),
            )
            .finish()
    }
}
