use std::any::{TypeId, type_name};
use std::fmt;

use thiserror::Error;

use ironclaw_approvals::FilesystemPersistentApprovalPolicyStore;
use ironclaw_authorization::FilesystemCapabilityLeaseStore;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_processes::{FilesystemProcessResultStore, FilesystemProcessStore};
use ironclaw_run_state::{FilesystemApprovalRequestStore, FilesystemRunStateStore};

use super::{
    DiskFilesystem, DurableAuditSink, DurableEventSink, EmptyWasmRuntimeCredentials,
    FilesystemSecretStore, HostProcessPort, InMemoryAuditSink, InMemoryCredentialBroker,
    InMemoryDurableAuditLog, InMemoryDurableEventLog, InMemoryEventSink, InMemoryResourceGovernor,
    NoopTurnRunWakeNotifier, RebornEventStoreError, RuntimeKind,
};

#[derive(Debug, Error)]
pub enum ProductionEventStoreWiringError {
    #[error("failed to build Reborn event stores: {0}")]
    EventStore(#[from] RebornEventStoreError),
    #[error("host runtime production wiring failed: {0}")]
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

impl ProductionWiringIssueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::UnsupportedRequirement => "unsupported_requirement",
            Self::LocalOnlyImplementation => "local_only_implementation",
            Self::UnverifiedProductionImplementation => "unverified_production_implementation",
        }
    }
}

/// One production wiring issue for a component in the host-runtime graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionWiringIssue {
    pub(super) component: ProductionWiringComponent,
    pub(super) kind: ProductionWiringIssueKind,
    pub(super) implementation: Option<&'static str>,
}

impl ProductionWiringIssue {
    pub fn new(
        component: ProductionWiringComponent,
        kind: ProductionWiringIssueKind,
        implementation: Option<&'static str>,
    ) -> Self {
        Self {
            component,
            kind,
            implementation,
        }
    }

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

impl fmt::Display for ProductionWiringIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.component.as_str())?;
        if let Some(implementation) = self.implementation {
            write!(formatter, "={implementation}")?;
        }
        write!(formatter, " ({})", self.kind.as_str())
    }
}

/// Report returned when a host-runtime graph is not production-ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionWiringReport {
    pub(super) issues: Vec<ProductionWiringIssue>,
}

impl ProductionWiringReport {
    pub fn new(issues: Vec<ProductionWiringIssue>) -> Self {
        Self { issues }
    }

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

impl fmt::Display for ProductionWiringReport {
    /// Render the unwired components so an operator sees *which* component
    /// failed production validation, not just that validation failed. The
    /// `issues` list is built deterministically by the wiring check, so the
    /// order is stable; `implementation` is a compile-time `type_name`, never
    /// a secret or host path, so it is safe to surface to the operator.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.issues.is_empty() {
            return formatter.write_str("no production wiring issues recorded");
        }
        for (index, issue) in self.issues.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{issue}")?;
        }
        Ok(())
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
        () if type_id == TypeId::of::<DiskFilesystem>()
            || type_id == TypeId::of::<InMemoryResourceGovernor>()
            // The process lifecycle/result stores no longer have bespoke
            // in-memory implementations; "in-memory" is the `InMemoryBackend`
            // behind the one production `FilesystemProcess*Store<F>`
            // (arch-simplification §4.3). A store backed by `InMemoryBackend` is
            // still local-only; libSQL/Postgres monomorphizations are distinct.
            || type_id == TypeId::of::<FilesystemProcessStore<InMemoryBackend>>()
            || type_id == TypeId::of::<FilesystemProcessResultStore<InMemoryBackend>>()
            // The run-state and approval-request stores no longer have bespoke
            // in-memory implementations; "in-memory" is the `InMemoryBackend`
            // behind the one production `Filesystem*Store<F>` (arch-simplification
            // §4.3). A store backed by `InMemoryBackend` is still local-only;
            // libSQL/Postgres monomorphizations are distinct.
            || type_id == TypeId::of::<FilesystemRunStateStore<InMemoryBackend>>()
            || type_id == TypeId::of::<FilesystemApprovalRequestStore<InMemoryBackend>>()
            // The persistent-approval and capability-lease stores no longer have
            // bespoke in-memory implementations; "in-memory" is now the
            // `InMemoryBackend` behind the one production `Filesystem*Store<F>`
            // (arch-simplification §4.3). A store backed by `InMemoryBackend` is
            // still local-only; the durable libSQL/Postgres monomorphizations are
            // distinct types and correctly classify as production candidates.
            || type_id == TypeId::of::<FilesystemPersistentApprovalPolicyStore<InMemoryBackend>>()
            || type_id == TypeId::of::<FilesystemCapabilityLeaseStore<InMemoryBackend>>()
            || type_id == TypeId::of::<InMemoryEventSink>()
            || type_id == TypeId::of::<InMemoryDurableEventLog>()
            || type_id == TypeId::of::<InMemoryAuditSink>()
            || type_id == TypeId::of::<InMemoryDurableAuditLog>()
            // The secret store no longer has a bespoke in-memory implementation;
            // "in-memory" is the `InMemoryBackend` behind the one production
            // encrypted `FilesystemSecretStore<F>` (arch-simplification §4.3).
            // A store backed by `InMemoryBackend` is still local-only; the
            // durable libSQL/Postgres monomorphizations are distinct types and
            // correctly classify as production candidates.
            || type_id == TypeId::of::<FilesystemSecretStore<InMemoryBackend>>()
            || type_id == TypeId::of::<InMemoryCredentialBroker>()
            || type_id == TypeId::of::<EmptyWasmRuntimeCredentials>()
            || type_id == TypeId::of::<NoopTurnRunWakeNotifier>()
            || type_id == TypeId::of::<HostProcessPort>() =>
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_display_names_component_kind_and_implementation() {
        let report = ProductionWiringReport {
            issues: vec![
                ProductionWiringIssue {
                    component: ProductionWiringComponent::TurnState,
                    kind: ProductionWiringIssueKind::LocalOnlyImplementation,
                    implementation: Some("ironclaw_turns::InMemoryTurnStateStore"),
                },
                ProductionWiringIssue {
                    component: ProductionWiringComponent::SecretStore,
                    kind: ProductionWiringIssueKind::Missing,
                    implementation: None,
                },
            ],
        };

        assert_eq!(
            report.to_string(),
            "turn_state=ironclaw_turns::InMemoryTurnStateStore (local_only_implementation); \
             secret_store (missing)"
        );
    }

    #[test]
    fn report_display_handles_empty_issue_list() {
        let report = ProductionWiringReport { issues: Vec::new() };

        assert_eq!(report.to_string(), "no production wiring issues recorded");
    }

    #[test]
    fn event_store_wiring_error_surfaces_report_detail() {
        let error = ProductionEventStoreWiringError::from(ProductionWiringReport {
            issues: vec![ProductionWiringIssue {
                component: ProductionWiringComponent::EventSink,
                kind: ProductionWiringIssueKind::LocalOnlyImplementation,
                implementation: Some("ironclaw_host_runtime::InMemoryEventSink"),
            }],
        });

        let rendered = error.to_string();
        assert!(rendered.contains("event_sink"), "got: {rendered}");
        assert!(
            rendered.contains("local_only_implementation"),
            "got: {rendered}"
        );
    }
}
