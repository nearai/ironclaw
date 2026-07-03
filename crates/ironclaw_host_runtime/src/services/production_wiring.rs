use std::any::{TypeId, type_name};

use thiserror::Error;

use super::{
    DurableAuditSink, DurableEventSink, EmptyWasmRuntimeCredentials, InMemoryApprovalRequestStore,
    InMemoryAuditSink, InMemoryCapabilityLeaseStore, InMemoryCredentialBroker,
    InMemoryDurableAuditLog, InMemoryDurableEventLog, InMemoryEventSink,
    InMemoryPersistentApprovalPolicyStore, InMemoryProcessResultStore, InMemoryProcessStore,
    InMemoryResourceGovernor, InMemoryRunStateStore, InMemorySecretStore, InMemoryTurnStateStore,
    LocalFilesystem, LocalHostProcessPort, NoopTurnRunWakeNotifier, RebornEventStoreError,
    RuntimeKind,
};

#[derive(Debug, Error)]
pub enum ProductionEventStoreWiringError {
    #[error("failed to build Reborn event stores: {0}")]
    EventStore(#[from] RebornEventStoreError),
    #[error("host runtime production wiring failed")]
    ProductionWiring(ProductionWiringReport),
}

impl From<ProductionWiringReport> for ProductionEventStoreWiringError {
    fn from(report: ProductionWiringReport) -> Self {
        Self::ProductionWiring(report)
    }
}

/// Production wiring requirements used by composition roots before exposing a
/// [`HostRuntimeServices`] graph as production-ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionWiringConfig {
    pub(super) required_runtime_backends: Vec<RuntimeKind>,
    pub(super) require_runtime_http_egress: bool,
    pub(super) require_wasm_credentials: bool,
    pub(super) require_credential_broker: bool,
}

impl ProductionWiringConfig {
    pub fn new<I>(required_runtime_backends: I) -> Self
    where
        I: IntoIterator<Item = RuntimeKind>,
    {
        Self {
            required_runtime_backends: required_runtime_backends.into_iter().collect(),
            require_runtime_http_egress: false,
            require_wasm_credentials: false,
            require_credential_broker: false,
        }
    }

    pub fn require_runtime_http_egress(mut self) -> Self {
        self.require_runtime_http_egress = true;
        self
    }

    pub fn require_wasm_credentials(mut self) -> Self {
        self.require_wasm_credentials = true;
        self
    }

    pub fn require_credential_broker(mut self) -> Self {
        self.require_credential_broker = true;
        self
    }

    pub(super) fn requires_runtime(&self, runtime: RuntimeKind) -> bool {
        self.required_runtime_backends.contains(&runtime)
    }
}

/// Production component tracked by the host-runtime production wiring guardrail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProductionWiringComponent {
    RuntimeBackend,
    RuntimePolicy,
    TrustPolicy,
    Filesystem,
    ResourceGovernor,
    ProcessStore,
    ProcessResultStore,
    RunState,
    ApprovalRequests,
    CapabilityLeases,
    PersistentApprovalPolicies,
    EventSink,
    AuditSink,
    SecretStore,
    CredentialAccountStore,
    CredentialSessionStore,
    RuntimeHttpEgress,
    RuntimeProcessPort,
    WasmCredentialProvider,
    ScriptRuntime,
    McpRuntime,
    WasmRuntime,
    FirstPartyRuntime,
    TurnState,
    RunProfileResolver,
    TurnRunWakeNotifier,
}

impl ProductionWiringComponent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBackend => "runtime_backend",
            Self::RuntimePolicy => "runtime_policy",
            Self::TrustPolicy => "trust_policy",
            Self::Filesystem => "filesystem",
            Self::ResourceGovernor => "resource_governor",
            Self::ProcessStore => "process_store",
            Self::ProcessResultStore => "process_result_store",
            Self::RunState => "run_state",
            Self::ApprovalRequests => "approval_requests",
            Self::CapabilityLeases => "capability_leases",
            Self::PersistentApprovalPolicies => "persistent_approval_policies",
            Self::EventSink => "event_sink",
            Self::AuditSink => "audit_sink",
            Self::SecretStore => "secret_store",
            Self::CredentialAccountStore => "credential_account_store",
            Self::CredentialSessionStore => "credential_session_store",
            Self::RuntimeHttpEgress => "runtime_http_egress",
            Self::RuntimeProcessPort => "runtime_process_port",
            Self::WasmCredentialProvider => "wasm_credential_provider",
            Self::ScriptRuntime => "script_runtime",
            Self::McpRuntime => "mcp_runtime",
            Self::WasmRuntime => "wasm_runtime",
            Self::FirstPartyRuntime => "first_party_runtime",
            Self::TurnState => "turn_state",
            Self::RunProfileResolver => "run_profile_resolver",
            Self::TurnRunWakeNotifier => "turn_run_wake_notifier",
        }
    }
}

/// Category of production wiring issue found in a service graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionWiringIssueKind {
    Missing,
    UnsupportedRequirement,
    LocalOnlyImplementation,
    UnverifiedProductionImplementation,
}

/// One production wiring issue for a component in the host-runtime graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionWiringIssue {
    pub(super) component: ProductionWiringComponent,
    pub(super) kind: ProductionWiringIssueKind,
    pub(super) implementation: Option<&'static str>,
}

impl ProductionWiringIssue {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(component: ProductionWiringComponent, kind: ProductionWiringIssueKind) -> Self {
        Self {
            component,
            kind,
            implementation: None,
        }
    }

    pub fn component(&self) -> ProductionWiringComponent {
        self.component
    }

    pub fn kind(&self) -> ProductionWiringIssueKind {
        self.kind
    }

    pub fn implementation(&self) -> Option<&'static str> {
        self.implementation
    }
}

/// Report returned when a host-runtime graph is not production-ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionWiringReport {
    pub(super) issues: Vec<ProductionWiringIssue>,
}

impl ProductionWiringReport {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(issues: Vec<ProductionWiringIssue>) -> Self {
        Self { issues }
    }

    pub fn issues(&self) -> &[ProductionWiringIssue] {
        &self.issues
    }

    pub fn contains(
        &self,
        component: ProductionWiringComponent,
        kind: ProductionWiringIssueKind,
    ) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.component == component && issue.kind == kind)
    }
}

pub(super) fn production_wiring_report(
    component: ProductionWiringComponent,
    kind: ProductionWiringIssueKind,
    implementation: Option<&'static str>,
) -> ProductionWiringReport {
    ProductionWiringReport {
        issues: vec![ProductionWiringIssue {
            component,
            kind,
            implementation,
        }],
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProductionComponentTypes {
    pub(super) trust_policy: Option<ProductionComponentType>,
    pub(super) trust_policy_verified: bool,
    pub(super) filesystem: ProductionComponentType,
    pub(super) resource_governor: ProductionComponentType,
    pub(super) process_store: ProductionComponentType,
    pub(super) process_result_store: ProductionComponentType,
    pub(super) run_state: Option<ProductionComponentType>,
    pub(super) approval_requests: Option<ProductionComponentType>,
    pub(super) capability_leases: Option<ProductionComponentType>,
    pub(super) persistent_approval_policies: Option<ProductionComponentType>,
    pub(super) event_sink: Option<ProductionComponentType>,
    pub(super) audit_sink: Option<ProductionComponentType>,
    pub(super) secret_store: Option<ProductionComponentType>,
    pub(super) credential_account_store: Option<ProductionComponentType>,
    pub(super) credential_session_store: Option<ProductionComponentType>,
    pub(super) runtime_http_egress: Option<ProductionComponentType>,
    pub(super) runtime_http_egress_verified: bool,
    pub(super) runtime_process_port: ProductionComponentType,
    pub(super) tenant_sandbox_process_port: Option<ProductionComponentType>,
    pub(super) wasm_credential_provider: Option<ProductionComponentType>,
    pub(super) wasm_credential_provider_verified: bool,
    pub(super) wasm_runtime_credential_provider_captured: bool,
    pub(super) script_runtime: Option<ProductionComponentType>,
    pub(super) mcp_runtime: Option<ProductionComponentType>,
    pub(super) first_party_runtime: Option<ProductionComponentType>,
    pub(super) turn_state: Option<ProductionComponentType>,
    pub(super) run_profile_resolver: Option<ProductionComponentType>,
    pub(super) turn_run_transition_port: Option<ProductionComponentType>,
    pub(super) turn_run_transition_port_verified: bool,
    pub(super) turn_run_wake_notifier: Option<ProductionComponentType>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProductionComponentType {
    pub(super) implementation: &'static str,
    pub(super) readiness: ProductionImplementationReadiness,
}

impl ProductionComponentType {
    pub(super) fn of<T: ?Sized + 'static>() -> Self {
        Self {
            implementation: type_name::<T>(),
            readiness: classify_component_type::<T>(),
        }
    }

    pub(super) fn named(
        implementation: &'static str,
        readiness: ProductionImplementationReadiness,
    ) -> Self {
        Self {
            implementation,
            readiness,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProductionImplementationReadiness {
    ProductionCandidate,
    LocalOnly,
    UnverifiedProductionImplementation,
}

pub(super) fn component_name(component: Option<ProductionComponentType>) -> Option<&'static str> {
    component.map(|component| component.implementation)
}

fn classify_component_type<T: ?Sized + 'static>() -> ProductionImplementationReadiness {
    let type_id = TypeId::of::<T>();
    match () {
        () if type_id == TypeId::of::<LocalFilesystem>()
            || type_id == TypeId::of::<InMemoryResourceGovernor>()
            || type_id == TypeId::of::<InMemoryProcessStore>()
            || type_id == TypeId::of::<InMemoryProcessResultStore>()
            || type_id == TypeId::of::<InMemoryRunStateStore>()
            || type_id == TypeId::of::<InMemoryApprovalRequestStore>()
            || type_id == TypeId::of::<InMemoryCapabilityLeaseStore>()
            || type_id == TypeId::of::<InMemoryPersistentApprovalPolicyStore>()
            || type_id == TypeId::of::<InMemoryEventSink>()
            || type_id == TypeId::of::<InMemoryDurableEventLog>()
            || type_id == TypeId::of::<InMemoryAuditSink>()
            || type_id == TypeId::of::<InMemoryDurableAuditLog>()
            || type_id == TypeId::of::<InMemorySecretStore>()
            || type_id == TypeId::of::<InMemoryCredentialBroker>()
            || type_id == TypeId::of::<EmptyWasmRuntimeCredentials>()
            || type_id == TypeId::of::<InMemoryTurnStateStore>()
            || type_id == TypeId::of::<NoopTurnRunWakeNotifier>()
            || type_id == TypeId::of::<LocalHostProcessPort>() =>
        {
            ProductionImplementationReadiness::LocalOnly
        }
        () if type_id == TypeId::of::<DurableEventSink>()
            || type_id == TypeId::of::<DurableAuditSink>() =>
        {
            ProductionImplementationReadiness::UnverifiedProductionImplementation
        }
        () => ProductionImplementationReadiness::ProductionCandidate,
    }
}
