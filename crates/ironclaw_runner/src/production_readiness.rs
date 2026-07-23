//! IronClaw loop production readiness validation.
//!
//! Host-runtime readiness stays substrate-scoped in
//! `ironclaw_host_runtime::ProductionWiringReport`. This module validates the
//! upper IronClaw loop graph: selected profile identities, registered loop
//! drivers, host-loop ports, production safety class, and active-run drain
//! protection.
//!
//! Startup composition is expected to construct `IronClawLoopProductionInputs`,
//! call `validate_ironclaw_loop_production_readiness`, gate production startup on
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

/// Readiness mode for the IronClaw loop graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IronClawLoopReadinessMode {
    /// Explicit local/developer/test mode. Fake, non-durable, and no-op
    /// implementations are allowed but reported as degraded warnings.
    LocalDevTest,
    /// Production mode. Components must be production-verified; local durable
    /// implementations are valid, but fake/non-durable/no-op/unverified seams
    /// fail closed.
    Production,
}

impl From<IronClawLoopReadinessMode> for DriverReadinessMode {
    fn from(mode: IronClawLoopReadinessMode) -> Self {
        match mode {
            IronClawLoopReadinessMode::LocalDevTest => Self::LocalDevTest,
            IronClawLoopReadinessMode::Production => Self::Production,
        }
    }
}

/// Production safety class for a concrete component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IronClawComponentSafetyClass {
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

impl IronClawComponentSafetyClass {
    fn blocks_production(self) -> bool {
        self != Self::ProductionVerified
    }

    fn degraded_in_local_dev(self) -> bool {
        self != Self::ProductionVerified
    }

    fn issue_kind(self) -> Option<IronClawLoopProductionIssueKind> {
        match self {
            Self::ProductionVerified => None,
            Self::TestOnly => Some(IronClawLoopProductionIssueKind::TestOnlyImplementation),
            Self::NonDurable => Some(IronClawLoopProductionIssueKind::NonDurableImplementation),
            Self::Noop => Some(IronClawLoopProductionIssueKind::NoopImplementation),
            Self::UnverifiedProductionImplementation => {
                Some(IronClawLoopProductionIssueKind::UnverifiedProductionImplementation)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IronClawComponentRequirement {
    Required,
    Optional,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IronClawLoopProductionComponent {
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
    SubagentCompletionObserver,
    SubagentAwaitEdgeStore,
    WakeNotifier,
    ProgressEvents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IronClawLoopProductionIssueKind {
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
pub struct IronClawLoopProductionIssue {
    pub component: IronClawLoopProductionComponent,
    pub kind: IronClawLoopProductionIssueKind,
    pub subject: String,
    pub profile_id: Option<RunProfileId>,
    pub profile_version: Option<RunProfileVersion>,
    pub driver_identity: Option<LoopDriverRegistryKey>,
    pub blocks_ready: bool,
}

impl IronClawLoopProductionIssue {
    fn blocking(
        component: IronClawLoopProductionComponent,
        kind: IronClawLoopProductionIssueKind,
        subject: impl Into<String>,
    ) -> Self {
        Self::new(component, kind, subject, true)
    }

    fn warning(
        component: IronClawLoopProductionComponent,
        kind: IronClawLoopProductionIssueKind,
        subject: impl Into<String>,
    ) -> Self {
        Self::new(component, kind, subject, false)
    }

    fn new(
        component: IronClawLoopProductionComponent,
        kind: IronClawLoopProductionIssueKind,
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

    fn with_profile(mut self, profile: &IronClawConfiguredRunProfile) -> Self {
        self.profile_id = Some(profile.profile_id.clone());
        self.profile_version = Some(profile.profile_version);
        self.driver_identity = Some(profile.driver_identity.clone());
        self
    }

    fn with_active_run(mut self, run: &IronClawActiveRunIdentity) -> Self {
        self.profile_id = Some(run.profile_id.clone());
        self.profile_version = Some(run.profile_version);
        self.driver_identity = Some(run.driver_identity.clone());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IronClawLoopProductionStatus {
    ProductionReady,
    LocalDevDegraded,
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawLoopProductionReport {
    pub status: IronClawLoopProductionStatus,
    pub issues: Vec<IronClawLoopProductionIssue>,
}

impl IronClawLoopProductionReport {
    pub fn is_ready(&self) -> bool {
        matches!(self.status, IronClawLoopProductionStatus::ProductionReady)
    }

    pub fn blocking_issues(&self) -> impl Iterator<Item = &IronClawLoopProductionIssue> {
        self.issues.iter().filter(|issue| issue.blocks_ready)
    }

    pub fn has_warnings(&self) -> bool {
        self.issues.iter().any(|issue| !issue.blocks_ready)
    }

    pub fn contains(
        &self,
        component: IronClawLoopProductionComponent,
        kind: IronClawLoopProductionIssueKind,
    ) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.component == component && issue.kind == kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawConfiguredRunProfile {
    pub profile_id: RunProfileId,
    pub profile_version: RunProfileVersion,
    pub selected: bool,
    pub driver_identity: LoopDriverRegistryKey,
    pub checkpoint_schema_id: CheckpointSchemaId,
    pub checkpoint_schema_version: RunProfileVersion,
}

impl IronClawConfiguredRunProfile {
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
pub struct IronClawActiveRunIdentity {
    pub run_ref: String,
    pub status: TurnStatus,
    pub profile_id: RunProfileId,
    pub profile_version: RunProfileVersion,
    pub driver_identity: LoopDriverRegistryKey,
}

impl IronClawActiveRunIdentity {
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
pub struct IronClawComponentReadiness {
    pub requirement: IronClawComponentRequirement,
    pub safety: Option<IronClawComponentSafetyClass>,
}

impl IronClawComponentReadiness {
    pub fn production_verified(requirement: IronClawComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(IronClawComponentSafetyClass::ProductionVerified),
        }
    }

    pub fn test_only(requirement: IronClawComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(IronClawComponentSafetyClass::TestOnly),
        }
    }

    pub fn non_durable(requirement: IronClawComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(IronClawComponentSafetyClass::NonDurable),
        }
    }

    pub fn noop(requirement: IronClawComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(IronClawComponentSafetyClass::Noop),
        }
    }

    pub fn unverified(requirement: IronClawComponentRequirement) -> Self {
        Self {
            requirement,
            safety: Some(IronClawComponentSafetyClass::UnverifiedProductionImplementation),
        }
    }

    pub fn missing(requirement: IronClawComponentRequirement) -> Self {
        Self {
            requirement,
            safety: None,
        }
    }

    fn present(self) -> bool {
        self.safety.is_some()
    }

    fn available_for(self, mode: IronClawLoopReadinessMode) -> bool {
        match mode {
            IronClawLoopReadinessMode::LocalDevTest => self.present(),
            IronClawLoopReadinessMode::Production => {
                self.safety == Some(IronClawComponentSafetyClass::ProductionVerified)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawLoopComponentGraphReadiness {
    pub host_factory: IronClawComponentReadiness,
    pub prompt_port: IronClawComponentReadiness,
    pub model_gateway: IronClawComponentReadiness,
    pub transcript_store: IronClawComponentReadiness,
    pub capability_port: IronClawComponentReadiness,
    pub checkpoint_state_store: IronClawComponentReadiness,
    pub input_control: IronClawComponentReadiness,
    pub loop_exit_applier: IronClawComponentReadiness,
    pub turn_state_store: IronClawComponentReadiness,
    pub subagent_goal_store: IronClawComponentReadiness,
    pub subagent_completion_observer: IronClawComponentReadiness,
    /// §3 replacement: the 3 dead-component readiness fields this used to
    /// sit alongside (`subagent_result_tombstone_store` — unwired dead code;
    /// `subagent_autonomous_continuation_budget`/`subagent_restart_reconciler`
    /// — readiness metadata for components never built, §8.3/§4.3 subsume
    /// their concerns) are deleted; this one field replaces them, covering
    /// the new durable CAS'd await-edge store this PR actually ships.
    pub subagent_await_edge_store: IronClawComponentReadiness,
    pub wake_notifier: IronClawComponentReadiness,
    pub progress_events: IronClawComponentReadiness,
}

impl IronClawLoopComponentGraphReadiness {
    pub fn production_verified() -> Self {
        let required = IronClawComponentRequirement::Required;
        Self {
            host_factory: IronClawComponentReadiness::production_verified(required),
            prompt_port: IronClawComponentReadiness::production_verified(required),
            model_gateway: IronClawComponentReadiness::production_verified(required),
            transcript_store: IronClawComponentReadiness::production_verified(required),
            capability_port: IronClawComponentReadiness::production_verified(required),
            checkpoint_state_store: IronClawComponentReadiness::production_verified(required),
            input_control: IronClawComponentReadiness::production_verified(required),
            loop_exit_applier: IronClawComponentReadiness::production_verified(required),
            turn_state_store: IronClawComponentReadiness::production_verified(required),
            subagent_goal_store: IronClawComponentReadiness::production_verified(required),
            subagent_completion_observer: IronClawComponentReadiness::production_verified(required),
            subagent_await_edge_store: IronClawComponentReadiness::production_verified(required),
            wake_notifier: IronClawComponentReadiness::production_verified(required),
            progress_events: IronClawComponentReadiness::production_verified(required),
        }
    }

    fn host_graph_for(&self, mode: IronClawLoopReadinessMode) -> HostGraphReadiness {
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
    ) -> impl Iterator<Item = (IronClawLoopProductionComponent, IronClawComponentReadiness)> {
        [
            (
                IronClawLoopProductionComponent::HostFactory,
                self.host_factory,
            ),
            (
                IronClawLoopProductionComponent::PromptPort,
                self.prompt_port,
            ),
            (
                IronClawLoopProductionComponent::ModelGateway,
                self.model_gateway,
            ),
            (
                IronClawLoopProductionComponent::TranscriptStore,
                self.transcript_store,
            ),
            (
                IronClawLoopProductionComponent::CapabilityPort,
                self.capability_port,
            ),
            (
                IronClawLoopProductionComponent::CheckpointStateStore,
                self.checkpoint_state_store,
            ),
            (
                IronClawLoopProductionComponent::InputControl,
                self.input_control,
            ),
            (
                IronClawLoopProductionComponent::LoopExitApplier,
                self.loop_exit_applier,
            ),
            (
                IronClawLoopProductionComponent::TurnStateStore,
                self.turn_state_store,
            ),
            (
                IronClawLoopProductionComponent::SubagentGoalStore,
                self.subagent_goal_store,
            ),
            (
                IronClawLoopProductionComponent::SubagentCompletionObserver,
                self.subagent_completion_observer,
            ),
            (
                IronClawLoopProductionComponent::SubagentAwaitEdgeStore,
                self.subagent_await_edge_store,
            ),
            (
                IronClawLoopProductionComponent::WakeNotifier,
                self.wake_notifier,
            ),
            (
                IronClawLoopProductionComponent::ProgressEvents,
                self.progress_events,
            ),
        ]
        .into_iter()
    }
}

pub struct IronClawLoopProductionInputs<'a> {
    pub mode: IronClawLoopReadinessMode,
    pub driver_registry: &'a DriverRegistry,
    pub component_graph: IronClawLoopComponentGraphReadiness,
    pub configured_profiles: Vec<IronClawConfiguredRunProfile>,
    pub active_runs: Vec<IronClawActiveRunIdentity>,
}

pub fn validate_ironclaw_loop_production_readiness(
    inputs: IronClawLoopProductionInputs<'_>,
) -> IronClawLoopProductionReport {
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
        IronClawLoopProductionStatus::NotReady
    } else if inputs.mode == IronClawLoopReadinessMode::LocalDevTest
        && issues.iter().any(|issue| !issue.blocks_ready)
    {
        IronClawLoopProductionStatus::LocalDevDegraded
    } else {
        IronClawLoopProductionStatus::ProductionReady
    };

    IronClawLoopProductionReport { status, issues }
}

fn push_component_issues(
    mode: IronClawLoopReadinessMode,
    graph: &IronClawLoopComponentGraphReadiness,
    issues: &mut Vec<IronClawLoopProductionIssue>,
) {
    for (component, readiness) in graph.components() {
        match (mode, readiness.requirement, readiness.safety) {
            (_, IronClawComponentRequirement::Unsupported, Some(_)) => {
                issues.push(IronClawLoopProductionIssue::blocking(
                    component,
                    IronClawLoopProductionIssueKind::UnsupportedRequirement,
                    component_subject(component),
                ))
            }
            (_, IronClawComponentRequirement::Required, None) => {
                issues.push(IronClawLoopProductionIssue::blocking(
                    component,
                    IronClawLoopProductionIssueKind::Missing,
                    component_subject(component),
                ))
            }
            (
                IronClawLoopReadinessMode::Production,
                IronClawComponentRequirement::Required,
                Some(safety),
            ) if safety.blocks_production() => {
                let Some(issue_kind) = safety.issue_kind() else {
                    continue;
                };
                issues.push(IronClawLoopProductionIssue::blocking(
                    component,
                    issue_kind,
                    component_subject(component),
                ));
            }
            (IronClawLoopReadinessMode::LocalDevTest, _, Some(safety))
                if safety.degraded_in_local_dev() =>
            {
                let Some(issue_kind) = safety.issue_kind() else {
                    continue;
                };
                issues.push(IronClawLoopProductionIssue::warning(
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
    profiles: &[IronClawConfiguredRunProfile],
    issues: &mut Vec<IronClawLoopProductionIssue>,
) {
    for profile in profiles.iter().filter(|profile| profile.selected) {
        if profile.driver_identity.checkpoint_schema_id.as_ref()
            != Some(&profile.checkpoint_schema_id)
            || profile.driver_identity.checkpoint_schema_version
                != Some(profile.checkpoint_schema_version)
        {
            issues.push(
                IronClawLoopProductionIssue::blocking(
                    IronClawLoopProductionComponent::CheckpointSchema,
                    IronClawLoopProductionIssueKind::VersionMismatch,
                    profile.profile_id.as_str(),
                )
                .with_profile(profile),
            );
        }
    }
}

fn push_active_run_profile_issues(
    profiles: &[IronClawConfiguredRunProfile],
    active_runs: &[IronClawActiveRunIdentity],
    issues: &mut Vec<IronClawLoopProductionIssue>,
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
                IronClawLoopProductionIssue::blocking(
                    IronClawLoopProductionComponent::RunProfile,
                    IronClawLoopProductionIssueKind::ActiveRunsRequireVersion,
                    active_run_subject(),
                )
                .with_active_run(run),
            );
        }
    }
}

fn push_driver_readiness_issues(
    inputs: &IronClawLoopProductionInputs<'_>,
    issues: &mut Vec<IronClawLoopProductionIssue>,
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
    inputs: &IronClawLoopProductionInputs<'_>,
    issues: &mut Vec<IronClawLoopProductionIssue>,
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
    configured_profiles: &[IronClawConfiguredRunProfile],
    active_runs: &[IronClawActiveRunIdentity],
    issues: &mut Vec<IronClawLoopProductionIssue>,
) {
    for diagnostic in report.diagnostics {
        let (component, kind) = match diagnostic.code {
            DriverReadinessDiagnosticCode::MissingConfiguredDriver => (
                IronClawLoopProductionComponent::LoopDriver,
                IronClawLoopProductionIssueKind::Missing,
            ),
            DriverReadinessDiagnosticCode::MissingNonTerminalRunDriver => (
                IronClawLoopProductionComponent::LoopDriver,
                IronClawLoopProductionIssueKind::ActiveRunDriverUnregistered,
            ),
            DriverReadinessDiagnosticCode::ReferenceDriverNotProductionReady
            | DriverReadinessDiagnosticCode::ReferenceDriverAllowedForLocalDev => (
                IronClawLoopProductionComponent::LoopDriver,
                IronClawLoopProductionIssueKind::TestOnlyImplementation,
            ),
            DriverReadinessDiagnosticCode::MissingRequiredDriverRequirement => (
                IronClawLoopProductionComponent::RunProfile,
                IronClawLoopProductionIssueKind::Missing,
            ),
        };
        let blocks_ready = keep_blocking && diagnostic.blocks_ready;
        let mut issue = IronClawLoopProductionIssue {
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
    issue: &IronClawLoopProductionIssue,
    profiles: &'a [IronClawConfiguredRunProfile],
) -> Option<&'a IronClawConfiguredRunProfile> {
    profiles.iter().find(|profile| {
        issue.subject == profile.profile_id.as_str()
            && issue.driver_identity.as_ref() == Some(&profile.driver_identity)
    })
}

fn matching_active_run<'a>(
    issue: &IronClawLoopProductionIssue,
    active_runs: &'a [IronClawActiveRunIdentity],
) -> Option<&'a IronClawActiveRunIdentity> {
    issue.driver_identity.as_ref().and_then(|driver_identity| {
        active_runs
            .iter()
            .find(|run| !run.status.is_terminal() && run.driver_identity == *driver_identity)
    })
}

fn configured_driver_profile(profile: &IronClawConfiguredRunProfile) -> ConfiguredRunProfile {
    ConfiguredRunProfile::enabled(profile.profile_id.as_str(), profile.driver_identity.clone())
}

fn active_run_subject() -> &'static str {
    "active_run"
}

fn component_subject(component: IronClawLoopProductionComponent) -> &'static str {
    match component {
        IronClawLoopProductionComponent::RunProfile => "run_profile",
        IronClawLoopProductionComponent::LoopDriver => "loop_driver",
        IronClawLoopProductionComponent::CheckpointSchema => "checkpoint_schema",
        IronClawLoopProductionComponent::HostFactory => "host_factory",
        IronClawLoopProductionComponent::PromptPort => "prompt_port",
        IronClawLoopProductionComponent::ModelGateway => "model_gateway",
        IronClawLoopProductionComponent::TranscriptStore => "transcript_store",
        IronClawLoopProductionComponent::CapabilityPort => "capability_port",
        IronClawLoopProductionComponent::CheckpointStateStore => "checkpoint_state_store",
        IronClawLoopProductionComponent::InputControl => "input_control",
        IronClawLoopProductionComponent::LoopExitApplier => "loop_exit_applier",
        IronClawLoopProductionComponent::TurnStateStore => "turn_state_store",
        IronClawLoopProductionComponent::SubagentGoalStore => "subagent_goal_store",
        IronClawLoopProductionComponent::SubagentCompletionObserver => {
            "subagent_completion_observer"
        }
        IronClawLoopProductionComponent::SubagentAwaitEdgeStore => "subagent_await_edge_store",
        IronClawLoopProductionComponent::WakeNotifier => "wake_notifier",
        IronClawLoopProductionComponent::ProgressEvents => "progress_events",
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

/// Subagent profiles need the full tool-capable loop-host surface, including a
/// real capability port for the `spawn_subagent` entry and attenuated flavor
/// tools.
pub fn subagent_driver_requirements() -> DriverRequirements {
    tool_capable_driver_requirements()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver_registry::{DriverReadinessReport, DriverReadinessStatus};

    #[test]
    fn production_verified_safety_has_no_issue_kind() {
        assert_eq!(
            IronClawComponentSafetyClass::ProductionVerified.issue_kind(),
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
