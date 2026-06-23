use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::RebornCompositionProfile;
use ironclaw_host_runtime::{
    ProductionWiringComponent, ProductionWiringIssue, ProductionWiringIssueKind,
    ProductionWiringReport,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RebornReadinessState {
    #[default]
    Disabled,
    DevOnly,
    HostedSingleTenantValidated,
    ProductionValidated,
    MigrationDryRunValidated,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornFacadeReadiness {
    pub host_runtime: bool,
    pub turn_coordinator: bool,
    pub product_auth: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornWorkerReadiness {
    pub turn_runner: bool,
    pub trigger_poller: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornReadinessDiagnosticStatus {
    Info,
    Warning,
    Blocking,
    Unknown(String),
}

impl RebornReadinessDiagnosticStatus {
    fn as_str(&self) -> &str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Blocking => "blocking",
            Self::Unknown(value) => value,
        }
    }
}

impl Serialize for RebornReadinessDiagnosticStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RebornReadinessDiagnosticStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "info" => Self::Info,
            "warning" => Self::Warning,
            "blocking" => Self::Blocking,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornReadinessDiagnosticReason {
    Disabled,
    DevOnlyProfile,
    Missing,
    LocalOnly,
    Unverified,
    Unsupported,
    Unknown(String),
}

impl RebornReadinessDiagnosticReason {
    fn as_str(&self) -> &str {
        match self {
            Self::Disabled => "disabled",
            Self::DevOnlyProfile => "dev-only-profile",
            Self::Missing => "missing",
            Self::LocalOnly => "local-only",
            Self::Unverified => "unverified",
            Self::Unsupported => "unsupported",
            Self::Unknown(value) => value,
        }
    }
}

impl Serialize for RebornReadinessDiagnosticReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RebornReadinessDiagnosticReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "disabled" => Self::Disabled,
            "dev-only-profile" => Self::DevOnlyProfile,
            "missing" => Self::Missing,
            "local-only" => Self::LocalOnly,
            "unverified" => Self::Unverified,
            "unsupported" => Self::Unsupported,
            _ => Self::Unknown(value),
        })
    }
}

/// Stable operator-facing component names.
///
/// The serialized names intentionally use `snake_case` to match the
/// host-runtime production-wiring component vocabulary consumed by readiness
/// diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornReadinessDiagnosticComponent {
    CompositionProfile,
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
    Unknown(String),
}

impl RebornReadinessDiagnosticComponent {
    fn as_str(&self) -> &str {
        match self {
            Self::CompositionProfile => "composition_profile",
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
            Self::Unknown(value) => value,
        }
    }
}

impl Serialize for RebornReadinessDiagnosticComponent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RebornReadinessDiagnosticComponent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "composition_profile" => Self::CompositionProfile,
            "runtime_backend" => Self::RuntimeBackend,
            "runtime_policy" => Self::RuntimePolicy,
            "trust_policy" => Self::TrustPolicy,
            "filesystem" => Self::Filesystem,
            "resource_governor" => Self::ResourceGovernor,
            "process_store" => Self::ProcessStore,
            "process_result_store" => Self::ProcessResultStore,
            "run_state" => Self::RunState,
            "approval_requests" => Self::ApprovalRequests,
            "capability_leases" => Self::CapabilityLeases,
            "persistent_approval_policies" => Self::PersistentApprovalPolicies,
            "event_sink" => Self::EventSink,
            "audit_sink" => Self::AuditSink,
            "secret_store" => Self::SecretStore,
            "credential_account_store" => Self::CredentialAccountStore,
            "credential_session_store" => Self::CredentialSessionStore,
            "runtime_http_egress" => Self::RuntimeHttpEgress,
            "runtime_process_port" => Self::RuntimeProcessPort,
            "wasm_credential_provider" => Self::WasmCredentialProvider,
            "script_runtime" => Self::ScriptRuntime,
            "mcp_runtime" => Self::McpRuntime,
            "wasm_runtime" => Self::WasmRuntime,
            "first_party_runtime" => Self::FirstPartyRuntime,
            "turn_state" => Self::TurnState,
            "run_profile_resolver" => Self::RunProfileResolver,
            "turn_run_wake_notifier" => Self::TurnRunWakeNotifier,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornReadinessDiagnostic {
    pub profile: RebornCompositionProfile,
    pub component: RebornReadinessDiagnosticComponent,
    pub reason: RebornReadinessDiagnosticReason,
    pub status: RebornReadinessDiagnosticStatus,
    /// Whether this diagnostic prevents production Reborn traffic exposure.
    ///
    /// `RebornReadiness::state` remains the primary readiness state. This field
    /// lets consumers identify which diagnostics are production blockers when
    /// a profile is disabled, dev-only, or production-shaped but incomplete.
    pub blocks_production: bool,
}

impl RebornReadinessDiagnostic {
    pub fn disabled() -> Self {
        Self {
            profile: RebornCompositionProfile::Disabled,
            component: RebornReadinessDiagnosticComponent::CompositionProfile,
            reason: RebornReadinessDiagnosticReason::Disabled,
            status: RebornReadinessDiagnosticStatus::Blocking,
            blocks_production: true,
        }
    }

    pub fn local_dev() -> Self {
        Self::dev_only_profile(RebornCompositionProfile::LocalDev)
    }

    pub fn local_dev_yolo() -> Self {
        Self::dev_only_profile(RebornCompositionProfile::LocalDevYolo)
    }

    pub fn hosted_single_tenant() -> Self {
        Self {
            profile: RebornCompositionProfile::HostedSingleTenant,
            component: RebornReadinessDiagnosticComponent::CompositionProfile,
            reason: RebornReadinessDiagnosticReason::Unverified,
            status: RebornReadinessDiagnosticStatus::Info,
            blocks_production: false,
        }
    }

    fn dev_only_profile(profile: RebornCompositionProfile) -> Self {
        Self {
            profile,
            component: RebornReadinessDiagnosticComponent::CompositionProfile,
            reason: RebornReadinessDiagnosticReason::DevOnlyProfile,
            status: RebornReadinessDiagnosticStatus::Blocking,
            blocks_production: true,
        }
    }

    pub fn production_blocker(
        profile: RebornCompositionProfile,
        component: RebornReadinessDiagnosticComponent,
        reason: RebornReadinessDiagnosticReason,
    ) -> Option<Self> {
        if !profile.requires_production_shape() {
            return None;
        }
        Some(Self {
            profile,
            component,
            reason,
            status: RebornReadinessDiagnosticStatus::Blocking,
            blocks_production: true,
        })
    }

    pub fn from_production_wiring_report(
        profile: RebornCompositionProfile,
        report: &ProductionWiringReport,
    ) -> Vec<Self> {
        if !profile.requires_production_shape() {
            return Vec::new();
        }

        report
            .issues()
            .iter()
            .filter_map(|issue| Self::from_production_wiring_issue(profile, issue))
            .collect()
    }

    pub fn from_production_wiring_issue(
        profile: RebornCompositionProfile,
        issue: &ProductionWiringIssue,
    ) -> Option<Self> {
        Self::production_blocker(profile, issue.component().into(), issue.kind().into())
    }
}

impl From<ProductionWiringComponent> for RebornReadinessDiagnosticComponent {
    fn from(component: ProductionWiringComponent) -> Self {
        match component {
            ProductionWiringComponent::RuntimeBackend => Self::RuntimeBackend,
            ProductionWiringComponent::RuntimePolicy => Self::RuntimePolicy,
            ProductionWiringComponent::TrustPolicy => Self::TrustPolicy,
            ProductionWiringComponent::Filesystem => Self::Filesystem,
            ProductionWiringComponent::ResourceGovernor => Self::ResourceGovernor,
            ProductionWiringComponent::ProcessStore => Self::ProcessStore,
            ProductionWiringComponent::ProcessResultStore => Self::ProcessResultStore,
            ProductionWiringComponent::RunState => Self::RunState,
            ProductionWiringComponent::ApprovalRequests => Self::ApprovalRequests,
            ProductionWiringComponent::CapabilityLeases => Self::CapabilityLeases,
            ProductionWiringComponent::PersistentApprovalPolicies => {
                Self::PersistentApprovalPolicies
            }
            ProductionWiringComponent::EventSink => Self::EventSink,
            ProductionWiringComponent::AuditSink => Self::AuditSink,
            ProductionWiringComponent::SecretStore => Self::SecretStore,
            ProductionWiringComponent::CredentialAccountStore => Self::CredentialAccountStore,
            ProductionWiringComponent::CredentialSessionStore => Self::CredentialSessionStore,
            ProductionWiringComponent::RuntimeHttpEgress => Self::RuntimeHttpEgress,
            ProductionWiringComponent::RuntimeProcessPort => Self::RuntimeProcessPort,
            ProductionWiringComponent::WasmCredentialProvider => Self::WasmCredentialProvider,
            ProductionWiringComponent::ScriptRuntime => Self::ScriptRuntime,
            ProductionWiringComponent::McpRuntime => Self::McpRuntime,
            ProductionWiringComponent::WasmRuntime => Self::WasmRuntime,
            ProductionWiringComponent::FirstPartyRuntime => Self::FirstPartyRuntime,
            ProductionWiringComponent::TurnState => Self::TurnState,
            ProductionWiringComponent::RunProfileResolver => Self::RunProfileResolver,
            ProductionWiringComponent::TurnRunWakeNotifier => Self::TurnRunWakeNotifier,
        }
    }
}

impl From<ProductionWiringIssueKind> for RebornReadinessDiagnosticReason {
    fn from(kind: ProductionWiringIssueKind) -> Self {
        match kind {
            ProductionWiringIssueKind::Missing => Self::Missing,
            ProductionWiringIssueKind::UnsupportedRequirement => Self::Unsupported,
            ProductionWiringIssueKind::LocalOnlyImplementation => Self::LocalOnly,
            ProductionWiringIssueKind::UnverifiedProductionImplementation => Self::Unverified,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornReadiness {
    pub profile: RebornCompositionProfile,
    pub state: RebornReadinessState,
    pub facades: RebornFacadeReadiness,
    #[serde(default)]
    pub workers: RebornWorkerReadiness,
    #[serde(default)]
    pub diagnostics: Vec<RebornReadinessDiagnostic>,
}

impl Default for RebornReadiness {
    fn default() -> Self {
        Self::disabled()
    }
}

impl RebornReadiness {
    /// Disabled readiness snapshot with its operator-facing diagnostic.
    ///
    /// This is intentionally not `const`: the rich snapshot includes the
    /// diagnostics vector that downstream readiness surfaces consume.
    pub fn disabled() -> Self {
        Self {
            profile: RebornCompositionProfile::Disabled,
            state: RebornReadinessState::Disabled,
            facades: RebornFacadeReadiness {
                host_runtime: false,
                turn_coordinator: false,
                product_auth: false,
            },
            workers: RebornWorkerReadiness {
                turn_runner: false,
                trigger_poller: false,
            },
            diagnostics: vec![RebornReadinessDiagnostic::disabled()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_default_matches_disabled_snapshot() {
        let readiness = RebornReadiness::default();

        assert_eq!(readiness.profile, RebornCompositionProfile::Disabled);
        assert_eq!(readiness.state, RebornReadinessState::Disabled);
        assert_eq!(readiness.diagnostics.len(), 1);
        assert_eq!(
            readiness.diagnostics[0].reason,
            RebornReadinessDiagnosticReason::Disabled
        );
        assert_eq!(
            readiness.diagnostics[0].status,
            RebornReadinessDiagnosticStatus::Blocking
        );
        assert!(readiness.diagnostics[0].blocks_production);
    }

    #[test]
    fn readiness_deserializes_without_workers_for_older_payloads() {
        let readiness: RebornReadiness = serde_json::from_str(
            r#"{
                "profile": "local-dev",
                "state": "dev-only",
                "facades": {
                    "host_runtime": true,
                    "turn_coordinator": true,
                    "product_auth": false
                }
            }"#,
        )
        .expect("readiness deserializes");

        assert_eq!(readiness.profile, RebornCompositionProfile::LocalDev);
        assert_eq!(readiness.state, RebornReadinessState::DevOnly);
        assert_eq!(readiness.workers, RebornWorkerReadiness::default());
        assert!(readiness.diagnostics.is_empty());
    }
}
