use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_reborn::{
    driver_registry::{DriverKind, DriverRegistry, DriverRequirements, LoopDriverRegistryKey},
    production_readiness::{
        RebornActiveRunIdentity, RebornComponentReadiness, RebornComponentRequirement,
        RebornConfiguredRunProfile, RebornLoopComponentGraphReadiness,
        RebornLoopProductionComponent, RebornLoopProductionInputs, RebornLoopProductionIssueKind,
        RebornLoopProductionStatus, RebornLoopReadinessMode, text_only_driver_requirements,
        tool_capable_driver_requirements, validate_reborn_loop_production_readiness,
    },
};
use ironclaw_turns::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverResumeRequest,
    AgentLoopDriverRunRequest, LoopExit, RunProfileId, RunProfileVersion, TurnStatus,
    run_profile::{AgentLoopDriverHost, CheckpointSchemaId, LoopDriverId},
};

#[test]
fn production_readiness_rejects_missing_selected_driver() {
    let registry = DriverRegistry::new();
    let profile = selected_profile(missing_key("text_loop", 1, "text_checkpoint", 1));

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![profile],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::LoopDriver,
        RebornLoopProductionIssueKind::Missing
    ));
}

#[test]
fn production_readiness_rejects_reference_driver() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "reference_echo", DriverKind::Reference);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::LoopDriver,
        RebornLoopProductionIssueKind::TestOnlyImplementation
    ));
}

#[test]
fn local_dev_allows_reference_driver_with_degraded_status() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "reference_echo", DriverKind::Reference);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::LocalDevTest,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::LocalDevDegraded);
    assert_eq!(report.blocking_issues().count(), 0);
    assert!(report.contains(
        RebornLoopProductionComponent::LoopDriver,
        RebornLoopProductionIssueKind::TestOnlyImplementation
    ));
}

#[test]
fn production_readiness_rejects_in_memory_checkpoint_store() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let mut graph = RebornLoopComponentGraphReadiness::production_verified();
    graph.checkpoint_state_store =
        RebornComponentReadiness::non_durable(RebornComponentRequirement::Required);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: graph,
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::CheckpointStateStore,
        RebornLoopProductionIssueKind::NonDurableImplementation
    ));
}

#[test]
fn production_readiness_rejects_non_durable_subagent_goal_store() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let mut graph = RebornLoopComponentGraphReadiness::production_verified();
    graph.subagent_goal_store =
        RebornComponentReadiness::non_durable(RebornComponentRequirement::Required);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: graph,
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::SubagentGoalStore,
        RebornLoopProductionIssueKind::NonDurableImplementation
    ));
}

#[test]
fn production_readiness_rejects_missing_subagent_completion_observer() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let mut graph = RebornLoopComponentGraphReadiness::production_verified();
    graph.subagent_completion_observer =
        RebornComponentReadiness::missing(RebornComponentRequirement::Required);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: graph,
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::SubagentCompletionObserver,
        RebornLoopProductionIssueKind::Missing
    ));
}

#[test]
fn production_readiness_rejects_noop_wake_notifier() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let mut graph = RebornLoopComponentGraphReadiness::production_verified();
    graph.wake_notifier = RebornComponentReadiness::noop(RebornComponentRequirement::Required);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: graph,
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::WakeNotifier,
        RebornLoopProductionIssueKind::NoopImplementation
    ));
}

#[test]
fn production_readiness_allows_text_only_with_deny_capability_port() {
    let mut registry = DriverRegistry::new();
    let key = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "text_loop",
                1,
                "text_checkpoint",
                1,
            ))),
            text_only_driver_requirements(),
            DriverKind::Production,
        )
        .expect("production text driver registration succeeds");
    let mut graph = RebornLoopComponentGraphReadiness::production_verified();
    graph.capability_port =
        RebornComponentReadiness::production_verified(RebornComponentRequirement::Required);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: graph,
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::ProductionReady);
    assert!(report.issues.is_empty());
}

#[test]
fn production_readiness_rejects_tool_profile_without_surface_service() {
    let mut registry = DriverRegistry::new();
    let key = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "tool_loop",
                1,
                "tool_checkpoint",
                1,
            ))),
            tool_capable_driver_requirements(),
            DriverKind::Production,
        )
        .expect("production tool driver registration succeeds");
    let mut graph = RebornLoopComponentGraphReadiness::production_verified();
    graph.capability_port = RebornComponentReadiness::missing(RebornComponentRequirement::Required);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: graph,
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::CapabilityPort,
        RebornLoopProductionIssueKind::Missing
    ));
}

#[test]
fn production_readiness_rejects_checkpoint_version_mismatch() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let profile = RebornConfiguredRunProfile::selected(
        RunProfileId::new("interactive_default").expect("valid profile id"),
        RunProfileVersion::new(1),
        key,
        CheckpointSchemaId::new("different_checkpoint").expect("valid schema id"),
        RunProfileVersion::new(1),
    );

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![profile],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::CheckpointSchema,
        RebornLoopProductionIssueKind::VersionMismatch
    ));
}

#[test]
fn production_readiness_rejects_removed_profile_version_with_active_runs() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let active_run = RebornActiveRunIdentity::new(
        "run-active-1",
        TurnStatus::Running,
        RunProfileId::new("old_interactive").expect("valid profile id"),
        RunProfileVersion::new(1),
        key.clone(),
    );

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![selected_profile(key)],
        active_runs: vec![active_run],
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::RunProfile,
        RebornLoopProductionIssueKind::ActiveRunsRequireVersion
    ));
}

#[test]
fn production_readiness_rejects_active_run_driver_identity_mismatch() {
    let mut registry = DriverRegistry::new();
    let configured_key = register_driver(&mut registry, "text_loop_v2", DriverKind::Production);
    let active_run_key = register_driver(&mut registry, "text_loop_v1", DriverKind::Production);
    let active_run = RebornActiveRunIdentity::new(
        "run-active-1",
        TurnStatus::Running,
        RunProfileId::new("interactive_default").expect("valid profile id"),
        RunProfileVersion::new(1),
        active_run_key,
    );

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![selected_profile(configured_key)],
        active_runs: vec![active_run],
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::RunProfile,
        RebornLoopProductionIssueKind::ActiveRunsRequireVersion
    ));
}

#[test]
fn production_readiness_rejects_active_run_checkpoint_schema_identity_mismatch() {
    let mut registry = DriverRegistry::new();
    let configured_key = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "text_loop",
                1,
                "new_text_checkpoint",
                1,
            ))),
            DriverRequirements::all_required(),
            DriverKind::Production,
        )
        .expect("configured driver registration succeeds");
    let active_run_key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let profile = RebornConfiguredRunProfile::selected(
        RunProfileId::new("interactive_default").expect("valid profile id"),
        RunProfileVersion::new(1),
        configured_key,
        CheckpointSchemaId::new("new_text_checkpoint").expect("valid checkpoint id"),
        RunProfileVersion::new(1),
    );
    let active_run = RebornActiveRunIdentity::new(
        "run-active-1",
        TurnStatus::Running,
        RunProfileId::new("interactive_default").expect("valid profile id"),
        RunProfileVersion::new(1),
        active_run_key,
    );

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![profile],
        active_runs: vec![active_run],
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    assert!(report.contains(
        RebornLoopProductionComponent::RunProfile,
        RebornLoopProductionIssueKind::ActiveRunsRequireVersion
    ));
}

#[test]
fn readiness_surface_redacts_active_run_subject() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let active_run = RebornActiveRunIdentity::new(
        "provider/sk-secret/prompt",
        TurnStatus::Running,
        RunProfileId::new("old_interactive").expect("valid profile id"),
        RunProfileVersion::new(1),
        missing_key("missing_loop", 1, "text_checkpoint", 1),
    );

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![selected_profile(key)],
        active_runs: vec![active_run],
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    for issue in report.issues {
        assert!(!issue.subject.contains('/'));
        assert!(!issue.subject.contains("sk-"));
        assert!(!issue.subject.contains("provider"));
        assert!(!issue.subject.contains("prompt"));
    }
}

#[test]
fn optional_profile_unavailable_does_not_block_startup() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let optional = RebornConfiguredRunProfile::selected(
        RunProfileId::new("optional_tool_profile").expect("valid profile id"),
        RunProfileVersion::new(1),
        missing_key("tool_loop", 1, "tool_checkpoint", 1),
        CheckpointSchemaId::new("tool_checkpoint").expect("valid checkpoint id"),
        RunProfileVersion::new(1),
    )
    .optional();

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![selected_profile(key), optional.clone()],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::ProductionReady);
    assert_eq!(report.blocking_issues().count(), 0);
    assert!(report.contains(
        RebornLoopProductionComponent::LoopDriver,
        RebornLoopProductionIssueKind::Missing
    ));
    assert!(report.has_warnings());
    let issue = report
        .issues
        .iter()
        .find(|issue| {
            issue.component == RebornLoopProductionComponent::LoopDriver
                && issue.kind == RebornLoopProductionIssueKind::Missing
        })
        .expect("optional missing driver issue is reported");
    assert_eq!(issue.profile_id.as_ref(), Some(&optional.profile_id));
    assert_eq!(issue.profile_version, Some(optional.profile_version));
    assert_eq!(
        issue.driver_identity.as_ref(),
        Some(&optional.driver_identity)
    );
}

#[test]
fn selected_driver_readiness_issues_include_profile_context() {
    let registry = DriverRegistry::new();
    let profile = selected_profile(missing_key("text_loop", 1, "text_checkpoint", 1));

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![profile.clone()],
        active_runs: Vec::new(),
    });

    let issue = report
        .issues
        .iter()
        .find(|issue| {
            issue.component == RebornLoopProductionComponent::LoopDriver
                && issue.kind == RebornLoopProductionIssueKind::Missing
        })
        .expect("selected missing driver issue is reported");
    assert_eq!(issue.profile_id.as_ref(), Some(&profile.profile_id));
    assert_eq!(issue.profile_version, Some(profile.profile_version));
    assert_eq!(
        issue.driver_identity.as_ref(),
        Some(&profile.driver_identity)
    );
}

#[test]
fn missing_active_run_driver_has_distinct_issue_kind_and_context() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let active_run = RebornActiveRunIdentity::new(
        "run-active-1",
        TurnStatus::Running,
        RunProfileId::new("interactive_default").expect("valid profile id"),
        RunProfileVersion::new(1),
        missing_key("missing_loop", 1, "text_checkpoint", 1),
    );

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![selected_profile(key)],
        active_runs: vec![active_run.clone()],
    });

    let issue = report
        .issues
        .iter()
        .find(|issue| {
            issue.component == RebornLoopProductionComponent::LoopDriver
                && issue.kind == RebornLoopProductionIssueKind::ActiveRunDriverUnregistered
        })
        .expect("active-run missing driver issue is reported");
    assert_eq!(issue.subject, "active_run");
    assert_eq!(issue.profile_id.as_ref(), Some(&active_run.profile_id));
    assert_eq!(issue.profile_version, Some(active_run.profile_version));
    assert_eq!(
        issue.driver_identity.as_ref(),
        Some(&active_run.driver_identity)
    );
}

#[test]
fn local_durable_components_are_not_rejected_by_name_or_locality() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(
        &mut registry,
        "standalone_local_text_loop",
        DriverKind::Production,
    );

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: RebornLoopComponentGraphReadiness::production_verified(),
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::ProductionReady);
    assert!(report.issues.is_empty());
}

#[test]
fn readiness_surface_stays_redaction_safe() {
    let mut registry = DriverRegistry::new();
    let key = register_driver(&mut registry, "text_loop", DriverKind::Production);
    let mut graph = RebornLoopComponentGraphReadiness::production_verified();
    graph.model_gateway =
        RebornComponentReadiness::unverified(RebornComponentRequirement::Required);

    let report = validate_reborn_loop_production_readiness(RebornLoopProductionInputs {
        mode: RebornLoopReadinessMode::Production,
        driver_registry: &registry,
        component_graph: graph,
        configured_profiles: vec![selected_profile(key)],
        active_runs: Vec::new(),
    });

    assert_eq!(report.status, RebornLoopProductionStatus::NotReady);
    for issue in report.issues {
        assert!(!issue.subject.contains('/'));
        assert!(!issue.subject.contains("sk-"));
        assert!(!issue.subject.contains("provider"));
        assert!(!issue.subject.contains("prompt"));
    }
}

fn register_driver(
    registry: &mut DriverRegistry,
    driver_id: &str,
    kind: DriverKind,
) -> LoopDriverRegistryKey {
    registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                driver_id,
                1,
                "text_checkpoint",
                1,
            ))),
            DriverRequirements::all_required(),
            kind,
        )
        .expect("driver registration succeeds")
}

fn selected_profile(driver_identity: LoopDriverRegistryKey) -> RebornConfiguredRunProfile {
    RebornConfiguredRunProfile::selected(
        RunProfileId::new("interactive_default").expect("valid profile id"),
        RunProfileVersion::new(1),
        driver_identity,
        CheckpointSchemaId::new("text_checkpoint").expect("valid checkpoint id"),
        RunProfileVersion::new(1),
    )
}

fn descriptor(
    driver_id: &str,
    driver_version: u64,
    checkpoint_schema_id: &str,
    checkpoint_schema_version: u64,
) -> AgentLoopDriverDescriptor {
    AgentLoopDriverDescriptor {
        id: LoopDriverId::new(driver_id).expect("valid driver id"),
        version: RunProfileVersion::new(driver_version),
        checkpoint_schema_id: Some(
            CheckpointSchemaId::new(checkpoint_schema_id).expect("valid checkpoint schema id"),
        ),
        checkpoint_schema_version: Some(RunProfileVersion::new(checkpoint_schema_version)),
    }
}

fn missing_key(
    driver_id: &str,
    driver_version: u64,
    checkpoint_schema_id: &str,
    checkpoint_schema_version: u64,
) -> LoopDriverRegistryKey {
    LoopDriverRegistryKey::from_descriptor(&descriptor(
        driver_id,
        driver_version,
        checkpoint_schema_id,
        checkpoint_schema_version,
    ))
    .expect("valid missing key")
}

struct TestDriver {
    descriptor: Mutex<AgentLoopDriverDescriptor>,
}

impl TestDriver {
    fn new(descriptor: AgentLoopDriverDescriptor) -> Self {
        Self {
            descriptor: Mutex::new(descriptor),
        }
    }
}

#[async_trait]
impl AgentLoopDriver for TestDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor
            .lock()
            .expect("test descriptor lock")
            .clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        Err(AgentLoopDriverError::Unavailable {
            reason: "test driver does not execute".to_string(),
        })
    }

    async fn resume(
        &self,
        _request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        Err(AgentLoopDriverError::Unavailable {
            reason: "test driver does not execute".to_string(),
        })
    }
}
