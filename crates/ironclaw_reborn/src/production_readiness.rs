//! Reborn loop production readiness validation.
//!
//! Host-runtime readiness stays substrate-scoped in
//! `ironclaw_host_runtime::ProductionWiringReport`. This module validates the
//! upper Reborn loop graph: selected profile identities, registered loop
//! drivers, host-loop ports, production safety class, and active-run drain
//! protection.
//!
//! Startup composition is expected to construct `RebornLoopProductionInputs`,
//! call `validate_reborn_loop_production_readiness`, gate production startup on
//! `report.is_ready()`, and still surface `report.issues` / `has_warnings()` for
//! operator diagnostics. The runtime gate is tracked separately from this pure
//! reporting slice so readiness semantics can stabilize before startup wiring.

use ironclaw_turns::{
    RunProfileId, RunProfileVersion, TurnStatus, run_profile::CheckpointSchemaId,
};

use crate::driver_registry::{
    ConfiguredRunProfile, DriverReadinessDiagnosticCode, DriverReadinessMode, DriverRegistry,
    DriverRequirements, HostGraphReadiness, LoopDriverRegistryKey, PersistedRunDriverIdentity,
    RequirementLevel,
};

/// Readiness mode for the Reborn loop graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornLoopReadinessMode {
    /// Explicit local/developer/test mode. Fake, non-durable, and no-op
    /// implementations are allowed but reported as degraded warnings.
    LocalDevTest,
    /// Production mode. Components must be production-verified; local durable
    /// implementations are valid, but fake/non-durable/no-op/unverified seams
    /// fail closed.
    Production,
}

impl From<RebornLoopReadinessMode> for DriverReadinessMode {
    fn from(mode: RebornLoopReadinessMode) -> Self {
        match mode {
            RebornLoopReadinessMode::LocalDevTest => Self::LocalDevTest,
            RebornLoopReadinessMode::Production => Self::Production,
        }
    }
}

/// Production safety class for a concrete component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornComponentSafetyClass {
    /// Verified for production invariants. The implementation may be local
    /// durable (for standalone-local production) or remote/cloud-backed.
    ProductionVerified,
    /// Test/fake/reference implementation.
    TestOnly,
    /// State is not durable enough for production restart/recovery semantics.
    NonDurable,
    /// Explicit no-op/null implementation.
    Noop,
    /// Not proven safe for production traffic yet.
    UnverifiedProductionImplementation,
}

impl RebornComponentSafetyClass {
    fn blocks_production(self) -> bool {
        self != Self::ProductionVerified
    }

    fn degraded_in_local_dev(self) -> bool {
        self != Self::ProductionVerified
    }

    fn issue_kind(self) -> Option<RebornLoopProductionIssueKind> {
        match self {
            Self::ProductionVerified => None,
            Self::TestOnly => Some(RebornLoopProductionIssueKind::TestOnlyImplementation),
            Self::NonDurable => Some(RebornLoopProductionIssueKind::NonDurableImplementation),
            Self::Noop => Some(RebornLoopProductionIssueKind::NoopImplementation),
            Self::UnverifiedProductionImplementation => {
                Some(RebornLoopProductionIssueKind::UnverifiedProductionImplementation)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornComponentRequirement {
    Required,
    Optional,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebornLoopProductionComponent {
    RunProfile,
    LoopDriver,
    CheckpointSchema,
    HostFactory,
    PromptPort,
    ModelGateway,
    TranscriptStore,
    CapabilityPort,
    CheckpointStateStore,
    InputControl,
    LoopExitApplier,
    TurnStateStore,
    SubagentGoalStore,
    WakeNotifier,
    ProgressEvents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebornLoopProductionIssueKind {
    Missing,
    TestOnlyImplementation,
    NonDurableImplementation,
    NoopImplementation,
    UnverifiedProductionImplementation,
    UnsupportedRequirement,
    VersionMismatch,
    ActiveRunsRequireVersion,
    ActiveRunDriverUnregistered,
    PolicyDenied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornLoopProductionIssue {
    pub component: RebornLoopProductionComponent,
    pub kind: RebornLoopProductionIssueKind,
    pub subject: String,
    pub profile_id: Option<RunProfileId>,
    pub profile_version: Option<RunProfileVersion>,
    pub driver_identity: Option<LoopDriverRegistryKey>,
    pub blocks_ready: bool,
}

impl RebornLoopProductionIssue {
    fn blocking(
        component: RebornLoopProductionComponent,
        kind: RebornLoopProductionIssueKind,
        subject: impl Into<String>,
    ) -> Self {
        Self::new(component, kind, subject, true)
    }

    fn warning(
        component: RebornLoopProductionComponent,
        kind: RebornLoopProductionIssueKind,
        subject: impl Into<String>,
    ) -> Self {
        Self::new(component, kind, subject, false)
    }

    fn new(
        component: RebornLoopProductionComponent,
        kind: RebornLoopProductionIssueKind,
        subject: impl Into<String>,
        blocks_ready: bool,
    ) -> Self {
        Self {
            component,
            kind,
            subject: subject.into(),
            profile_id: None,
            profile_version: None,
            driver_identity: None,
            blocks_ready,
        }
    }

    fn with_profile(mut self, profile: &RebornConfiguredRunProfile) -> Self {
        self.profile_id = Some(profile.profile_id.clone());
        self.profile_version = Some(profile.profile_version);
        self.driver_identity = Some(profile.driver_identity.clone());
        self
    }

    fn with_active_run(mut self, run: &RebornActiveRunIdentity) -> Self {
        self.profile_id = Some(run.profile_id.clone());
        self.profile_version = Some(run.profile_version);
        self.driver_identity = Some(run.driver_identity.clone());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornLoopProductionStatus {
    ProductionReady,
    LocalDevDegraded,
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornLoopProductionReport {
    pub status: RebornLoopProductionStatus,
    pub issues: Vec<RebornLoopProductionIssue>,
}

impl RebornLoopProductionReport {
    pub fn is_ready(&self) -> bool {
        matches!(self.status, RebornLoopProductionStatus::ProductionReady)
    }

    pub fn blocking_issues(&self) -> impl Iterator<Item = &RebornLoopProductionIssue> {
        self.issues.iter().filter(|issue| issue.blocks_ready)
    }

    pub fn has_warnings(&self) -> bool {
        self.issues.iter().any(|issue| !issue.blocks_ready)
    }

    pub fn contains(
        &self,
        component: RebornLoopProductionComponent,
        kind: RebornLoopProductionIssueKind,
    ) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.component == component && issue.kind == kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornConfiguredRunProfile {
    pub profile_id: RunProfileId,
    pub profile_version: RunProfileVersion,
    pub selected: bool,
    pub driver_identity: LoopDriverRegistryKey,
    pub checkpoint_schema_id: CheckpointSchemaId,
    pub checkpoint_schema_version: RunProfileVersion,
}

impl RebornConfiguredRunProfile {
    pub fn selected(
        profile_id: RunProfileId,
        profile_version: RunProfileVersion,
        driver_identity: LoopDriverRegistryKey,
        checkpoint_schema_id: CheckpointSchemaId,
        checkpoint_schema_version: RunProfileVersion,
    ) -> Self {
        Self {
            profile_id,
            profile_version,
            selected: true,
            driver_identity,
            checkpoint_schema_id,
            checkpoint_schema_version,
        }
    }

    pub fn optional(mut self) -> Self {
        self.selected = false;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornActiveRunIdentity {
    pub run_ref: String,
    pub status: TurnStatus,
    pub profile_id: RunProfileId,
    pub profile_version: RunProfileVersion,
    pub driver_identity: LoopDriverRegistryKey,
}

impl RebornActiveRunIdentity {
    pub fn new(
        run_ref: impl Into<String>,
        status: TurnStatus,
        profile_id: RunProfileId,
        profile_version: RunProfileVersion,
        driver_identity: LoopDriverRegistryKey,
    ) -> Self {
        Self {
            run_ref: run_ref.into(),
            status,
            profile_id,
            profile_version,
            driver_identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebornComponentReadiness {
    pub requirement: RebornComponentRequirement,
    pub safety: Option<RebornComponentSafetyClass>,
}

impl RebornComponentReadiness {
    pub fn production_verified(requirement: RebornComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(RebornComponentSafetyClass::ProductionVerified),
        }
    }

    pub fn test_only(requirement: RebornComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(RebornComponentSafetyClass::TestOnly),
        }
    }

    pub fn non_durable(requirement: RebornComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(RebornComponentSafetyClass::NonDurable),
        }
    }

    pub fn noop(requirement: RebornComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(RebornComponentSafetyClass::Noop),
        }
    }

    pub fn unverified(requirement: RebornComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(RebornComponentSafetyClass::UnverifiedProductionImplementation),
        }
    }

    pub fn missing(requirement: RebornComponentRequirement) -> Self {
        Self {
            requirement,
            safety: None,
        }
    }

    fn present(self) -> bool {
        self.safety.is_some()
    }

    fn available_for(self, mode: RebornLoopReadinessMode) -> bool {
        match mode {
            RebornLoopReadinessMode::LocalDevTest => self.present(),
            RebornLoopReadinessMode::Production => {
                self.safety == Some(RebornComponentSafetyClass::ProductionVerified)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornLoopComponentGraphReadiness {
    pub host_factory: RebornComponentReadiness,
    pub prompt_port: RebornComponentReadiness,
    pub model_gateway: RebornComponentReadiness,
    pub transcript_store: RebornComponentReadiness,
    pub capability_port: RebornComponentReadiness,
    pub checkpoint_state_store: RebornComponentReadiness,
    pub input_control: RebornComponentReadiness,
    pub loop_exit_applier: RebornComponentReadiness,
    pub turn_state_store: RebornComponentReadiness,
    pub subagent_goal_store: RebornComponentReadiness,
    pub wake_notifier: RebornComponentReadiness,
    pub progress_events: RebornComponentReadiness,
}

impl RebornLoopComponentGraphReadiness {
    pub fn production_verified() -> Self {
        let required = RebornComponentRequirement::Required;
        Self {
            host_factory: RebornComponentReadiness::production_verified(required),
            prompt_port: RebornComponentReadiness::production_verified(required),
            model_gateway: RebornComponentReadiness::production_verified(required),
            transcript_store: RebornComponentReadiness::production_verified(required),
            capability_port: RebornComponentReadiness::production_verified(required),
            checkpoint_state_store: RebornComponentReadiness::production_verified(required),
            input_control: RebornComponentReadiness::production_verified(required),
            loop_exit_applier: RebornComponentReadiness::production_verified(required),
            turn_state_store: RebornComponentReadiness::production_verified(required),
            subagent_goal_store: RebornComponentReadiness::production_verified(required),
            wake_notifier: RebornComponentReadiness::production_verified(required),
            progress_events: RebornComponentReadiness::production_verified(required),
        }
    }

    fn host_graph_for(&self, mode: RebornLoopReadinessMode) -> HostGraphReadiness {
        HostGraphReadiness {
            model: self.model_gateway.available_for(mode),
            prompt: self.prompt_port.available_for(mode),
            transcript: self.transcript_store.available_for(mode),
            checkpoint: self.checkpoint_state_store.available_for(mode),
            input_polling: self.input_control.available_for(mode),
            capabilities: self.capability_port.available_for(mode),
            progress_events: self.progress_events.available_for(mode),
        }
    }

    fn components(
        &self,
    ) -> impl Iterator<Item = (RebornLoopProductionComponent, RebornComponentReadiness)> {
        [
            (
                RebornLoopProductionComponent::HostFactory,
                self.host_factory,
            ),
            (RebornLoopProductionComponent::PromptPort, self.prompt_port),
            (
                RebornLoopProductionComponent::ModelGateway,
                self.model_gateway,
            ),
            (
                RebornLoopProductionComponent::TranscriptStore,
                self.transcript_store,
            ),
            (
                RebornLoopProductionComponent::CapabilityPort,
                self.capability_port,
            ),
            (
                RebornLoopProductionComponent::CheckpointStateStore,
                self.checkpoint_state_store,
            ),
            (
                RebornLoopProductionComponent::InputControl,
                self.input_control,
            ),
            (
                RebornLoopProductionComponent::LoopExitApplier,
                self.loop_exit_applier,
            ),
            (
                RebornLoopProductionComponent::TurnStateStore,
                self.turn_state_store,
            ),
            (
                RebornLoopProductionComponent::SubagentGoalStore,
                self.subagent_goal_store,
            ),
            (
                RebornLoopProductionComponent::WakeNotifier,
                self.wake_notifier,
            ),
            (
                RebornLoopProductionComponent::ProgressEvents,
                self.progress_events,
            ),
        ]
        .into_iter()
    }
}

pub struct RebornLoopProductionInputs<'a> {
    pub mode: RebornLoopReadinessMode,
    pub driver_registry: &'a DriverRegistry,
    pub component_graph: RebornLoopComponentGraphReadiness,
    pub configured_profiles: Vec<RebornConfiguredRunProfile>,
    pub active_runs: Vec<RebornActiveRunIdentity>,
}

pub fn validate_reborn_loop_production_readiness(
    inputs: RebornLoopProductionInputs<'_>,
) -> RebornLoopProductionReport {
    let mut issues = Vec::new();
    push_component_issues(inputs.mode, &inputs.component_graph, &mut issues);
    push_profile_identity_issues(&inputs.configured_profiles, &mut issues);
    push_active_run_profile_issues(
        &inputs.configured_profiles,
        &inputs.active_runs,
        &mut issues,
    );
    push_driver_readiness_issues(&inputs, &mut issues);
    push_optional_profile_issues(&inputs, &mut issues);

    let status = if issues.iter().any(|issue| issue.blocks_ready) {
        RebornLoopProductionStatus::NotReady
    } else if inputs.mode == RebornLoopReadinessMode::LocalDevTest
        && issues.iter().any(|issue| !issue.blocks_ready)
    {
        RebornLoopProductionStatus::LocalDevDegraded
    } else {
        RebornLoopProductionStatus::ProductionReady
    };

    RebornLoopProductionReport { status, issues }
}

fn push_component_issues(
    mode: RebornLoopReadinessMode,
    graph: &RebornLoopComponentGraphReadiness,
    issues: &mut Vec<RebornLoopProductionIssue>,
) {
    for (component, readiness) in graph.components() {
        match (mode, readiness.requirement, readiness.safety) {
            (_, RebornComponentRequirement::Unsupported, Some(_)) => {
                issues.push(RebornLoopProductionIssue::blocking(
                    component,
                    RebornLoopProductionIssueKind::UnsupportedRequirement,
                    component_subject(component),
                ))
            }
            (_, RebornComponentRequirement::Required, None) => {
                issues.push(RebornLoopProductionIssue::blocking(
                    component,
                    RebornLoopProductionIssueKind::Missing,
                    component_subject(component),
                ))
            }
            (
                RebornLoopReadinessMode::Production,
                RebornComponentRequirement::Required,
                Some(safety),
            ) if safety.blocks_production() => {
                let Some(issue_kind) = safety.issue_kind() else {
                    continue;
                };
                issues.push(RebornLoopProductionIssue::blocking(
                    component,
                    issue_kind,
                    component_subject(component),
                ));
            }
            (RebornLoopReadinessMode::LocalDevTest, _, Some(safety))
                if safety.degraded_in_local_dev() =>
            {
                let Some(issue_kind) = safety.issue_kind() else {
                    continue;
                };
                issues.push(RebornLoopProductionIssue::warning(
                    component,
                    issue_kind,
                    component_subject(component),
                ));
            }
            _ => {}
        }
    }
}

fn push_profile_identity_issues(
    profiles: &[RebornConfiguredRunProfile],
    issues: &mut Vec<RebornLoopProductionIssue>,
) {
    for profile in profiles.iter().filter(|profile| profile.selected) {
        if profile.driver_identity.checkpoint_schema_id.as_ref()
            != Some(&profile.checkpoint_schema_id)
            || profile.driver_identity.checkpoint_schema_version
                != Some(profile.checkpoint_schema_version)
        {
            issues.push(
                RebornLoopProductionIssue::blocking(
                    RebornLoopProductionComponent::CheckpointSchema,
                    RebornLoopProductionIssueKind::VersionMismatch,
                    profile.profile_id.as_str(),
                )
                .with_profile(profile),
            );
        }
    }
}

fn push_active_run_profile_issues(
    profiles: &[RebornConfiguredRunProfile],
    active_runs: &[RebornActiveRunIdentity],
    issues: &mut Vec<RebornLoopProductionIssue>,
) {
    for run in active_runs.iter().filter(|run| !run.status.is_terminal()) {
        let keeps_profile_version = profiles.iter().any(|profile| {
            profile.selected
                && profile.profile_id == run.profile_id
                && profile.profile_version == run.profile_version
                && profile.driver_identity == run.driver_identity
        });
        if !keeps_profile_version {
            issues.push(
                RebornLoopProductionIssue::blocking(
                    RebornLoopProductionComponent::RunProfile,
                    RebornLoopProductionIssueKind::ActiveRunsRequireVersion,
                    active_run_subject(),
                )
                .with_active_run(run),
            );
        }
    }
}

fn push_driver_readiness_issues(
    inputs: &RebornLoopProductionInputs<'_>,
    issues: &mut Vec<RebornLoopProductionIssue>,
) {
    let selected_profiles = inputs
        .configured_profiles
        .iter()
        .filter(|profile| profile.selected)
        .map(configured_driver_profile);
    let persisted_runs = inputs
        .active_runs
        .iter()
        .filter(|run| !run.status.is_terminal())
        .map(|run| {
            PersistedRunDriverIdentity::new(
                active_run_subject(),
                run.status,
                run.driver_identity.clone(),
            )
        });

    let report = inputs.driver_registry.validate_readiness_from_iter(
        inputs.mode.into(),
        inputs.component_graph.host_graph_for(inputs.mode),
        selected_profiles,
        persisted_runs,
    );
    push_mapped_driver_issues(
        report,
        true,
        &inputs.configured_profiles,
        &inputs.active_runs,
        issues,
    );
}

fn push_optional_profile_issues(
    inputs: &RebornLoopProductionInputs<'_>,
    issues: &mut Vec<RebornLoopProductionIssue>,
) {
    let optional_profiles = inputs
        .configured_profiles
        .iter()
        .filter(|profile| !profile.selected)
        .map(configured_driver_profile);

    let report = inputs.driver_registry.validate_readiness_from_iter(
        inputs.mode.into(),
        inputs.component_graph.host_graph_for(inputs.mode),
        optional_profiles,
        std::iter::empty::<PersistedRunDriverIdentity>(),
    );
    // Optional-profile validation never supplies persisted active runs, so
    // MissingNonTerminalRunDriver is unreachable on this path.
    push_mapped_driver_issues(report, false, &inputs.configured_profiles, &[], issues);
}

fn push_mapped_driver_issues(
    report: crate::driver_registry::DriverReadinessReport,
    keep_blocking: bool,
    configured_profiles: &[RebornConfiguredRunProfile],
    active_runs: &[RebornActiveRunIdentity],
    issues: &mut Vec<RebornLoopProductionIssue>,
) {
    for diagnostic in report.diagnostics {
        let (component, kind) = match diagnostic.code {
            DriverReadinessDiagnosticCode::MissingConfiguredDriver => (
                RebornLoopProductionComponent::LoopDriver,
                RebornLoopProductionIssueKind::Missing,
            ),
            DriverReadinessDiagnosticCode::MissingNonTerminalRunDriver => (
                RebornLoopProductionComponent::LoopDriver,
                RebornLoopProductionIssueKind::ActiveRunDriverUnregistered,
            ),
            DriverReadinessDiagnosticCode::ReferenceDriverNotProductionReady
            | DriverReadinessDiagnosticCode::ReferenceDriverAllowedForLocalDev => (
                RebornLoopProductionComponent::LoopDriver,
                RebornLoopProductionIssueKind::TestOnlyImplementation,
            ),
            DriverReadinessDiagnosticCode::MissingRequiredDriverRequirement => (
                RebornLoopProductionComponent::RunProfile,
                RebornLoopProductionIssueKind::Missing,
            ),
        };
        let blocks_ready = keep_blocking && diagnostic.blocks_ready;
        let mut issue = RebornLoopProductionIssue {
            component,
            kind,
            subject: diagnostic.subject,
            profile_id: None,
            profile_version: None,
            driver_identity: diagnostic.driver_identity,
            blocks_ready,
        };
        if let Some(profile) = matching_configured_profile(&issue, configured_profiles) {
            issue = issue.with_profile(profile);
        } else if let Some(run) = matching_active_run(&issue, active_runs) {
            issue = issue.with_active_run(run);
        }
        issues.push(issue);
    }
}

fn matching_configured_profile<'a>(
    issue: &RebornLoopProductionIssue,
    profiles: &'a [RebornConfiguredRunProfile],
) -> Option<&'a RebornConfiguredRunProfile> {
    profiles.iter().find(|profile| {
        issue.subject == profile.profile_id.as_str()
            && issue.driver_identity.as_ref() == Some(&profile.driver_identity)
    })
}

fn matching_active_run<'a>(
    issue: &RebornLoopProductionIssue,
    active_runs: &'a [RebornActiveRunIdentity],
) -> Option<&'a RebornActiveRunIdentity> {
    issue.driver_identity.as_ref().and_then(|driver_identity| {
        active_runs
            .iter()
            .find(|run| !run.status.is_terminal() && run.driver_identity == *driver_identity)
    })
}

fn configured_driver_profile(profile: &RebornConfiguredRunProfile) -> ConfiguredRunProfile {
    ConfiguredRunProfile::enabled(profile.profile_id.as_str(), profile.driver_identity.clone())
}

fn active_run_subject() -> &'static str {
    "active_run"
}

fn component_subject(component: RebornLoopProductionComponent) -> &'static str {
    match component {
        RebornLoopProductionComponent::RunProfile => "run_profile",
        RebornLoopProductionComponent::LoopDriver => "loop_driver",
        RebornLoopProductionComponent::CheckpointSchema => "checkpoint_schema",
        RebornLoopProductionComponent::HostFactory => "host_factory",
        RebornLoopProductionComponent::PromptPort => "prompt_port",
        RebornLoopProductionComponent::ModelGateway => "model_gateway",
        RebornLoopProductionComponent::TranscriptStore => "transcript_store",
        RebornLoopProductionComponent::CapabilityPort => "capability_port",
        RebornLoopProductionComponent::CheckpointStateStore => "checkpoint_state_store",
        RebornLoopProductionComponent::InputControl => "input_control",
        RebornLoopProductionComponent::LoopExitApplier => "loop_exit_applier",
        RebornLoopProductionComponent::TurnStateStore => "turn_state_store",
        RebornLoopProductionComponent::SubagentGoalStore => "subagent_goal_store",
        RebornLoopProductionComponent::WakeNotifier => "wake_notifier",
        RebornLoopProductionComponent::ProgressEvents => "progress_events",
    }
}

/// Utility for profiles that need every host surface, including a real
/// capability port.
pub fn tool_capable_driver_requirements() -> DriverRequirements {
    DriverRequirements {
        model: RequirementLevel::Required,
        prompt: RequirementLevel::Required,
        transcript: RequirementLevel::Required,
        checkpoint: RequirementLevel::Required,
        input_polling: RequirementLevel::Required,
        capabilities: RequirementLevel::Required,
        progress_events: RequirementLevel::Required,
    }
}

/// Utility for text-only profiles: capability calls are supported by an
/// explicit production-safe deny capability port, not by omitting the port.
pub fn text_only_driver_requirements() -> DriverRequirements {
    tool_capable_driver_requirements()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver_registry::{DriverReadinessReport, DriverReadinessStatus};

    #[test]
    fn production_verified_safety_has_no_issue_kind() {
        assert_eq!(
            RebornComponentSafetyClass::ProductionVerified.issue_kind(),
            None
        );
    }

    #[test]
    fn mapped_driver_issues_do_not_invent_unavailable_profiles_without_diagnostics() {
        let mut issues = Vec::new();

        push_mapped_driver_issues(
            DriverReadinessReport {
                status: DriverReadinessStatus::NotReady,
                diagnostics: Vec::new(),
            },
            false,
            &[],
            &[],
            &mut issues,
        );

        assert!(issues.is_empty());
    }
}
