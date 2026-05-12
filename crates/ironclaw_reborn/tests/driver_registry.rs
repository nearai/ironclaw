use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_reborn::driver_registry::{
    ConfiguredRunProfile, DriverKind, DriverReadinessInputs, DriverReadinessMode,
    DriverReadinessStatus, DriverRegistry, DriverRegistryError, DriverRequirements,
    HostGraphReadiness, LoopDriverRegistryKey, PersistedRunDriverIdentity, RequirementLevel,
};
use ironclaw_turns::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverResumeRequest,
    AgentLoopDriverRunRequest, LoopExit, RunProfileVersion, TurnStatus,
    run_profile::{AgentLoopDriverHost, CheckpointSchemaId, LoopDriverId},
};

#[test]
fn driver_registry_rejects_duplicate_exact_identity() {
    let mut registry = DriverRegistry::new();
    let requirements = DriverRequirements::all_optional();

    registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "lightweight_loop",
                1,
                "checkpoint_v1",
                1,
            ))),
            requirements.clone(),
            DriverKind::Production,
        )
        .expect("first registration should succeed");

    let error = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "lightweight_loop",
                1,
                "checkpoint_v1",
                1,
            ))),
            requirements,
            DriverKind::Production,
        )
        .expect_err("duplicate exact identity must be rejected");

    assert!(matches!(
        error,
        DriverRegistryError::DuplicateRegistration { .. }
    ));
}

#[test]
fn driver_registry_allows_side_by_side_versions() {
    let mut registry = DriverRegistry::new();
    let requirements = DriverRequirements::all_optional();

    let v1 = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "lightweight_loop",
                1,
                "checkpoint_v1",
                1,
            ))),
            requirements.clone(),
            DriverKind::Production,
        )
        .expect("v1 registration should succeed");
    let v2 = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "lightweight_loop",
                2,
                "checkpoint_v2",
                2,
            ))),
            requirements,
            DriverKind::Production,
        )
        .expect("v2 registration should succeed");

    assert_ne!(v1, v2);
    assert!(registry.get(&v1).is_some());
    assert!(registry.get(&v2).is_some());
}

#[test]
fn production_readiness_rejects_missing_configured_driver() {
    let registry = DriverRegistry::new();
    let missing_key = key("lightweight_loop", 1, "checkpoint_v1", 1);

    let report = registry.validate_readiness(
        DriverReadinessMode::Production,
        DriverReadinessInputs {
            host_graph: HostGraphReadiness::all_available(),
            configured_profiles: vec![ConfiguredRunProfile::enabled(
                "interactive_default",
                missing_key,
            )],
            persisted_runs: Vec::new(),
        },
    );

    assert_eq!(report.status, DriverReadinessStatus::NotReady);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_configured_driver")
    );
}

#[test]
fn production_readiness_rejects_fake_driver() {
    let mut registry = DriverRegistry::new();
    let fake_key = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "reference_echo_loop",
                1,
                "checkpoint_v1",
                1,
            ))),
            DriverRequirements::all_optional(),
            DriverKind::Reference,
        )
        .expect("reference driver registration should succeed");

    let report = registry.validate_readiness(
        DriverReadinessMode::Production,
        DriverReadinessInputs {
            host_graph: HostGraphReadiness::all_available(),
            configured_profiles: vec![ConfiguredRunProfile::enabled("local_reference", fake_key)],
            persisted_runs: Vec::new(),
        },
    );

    assert_eq!(report.status, DriverReadinessStatus::NotReady);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "reference_driver_not_production_ready")
    );
}

#[test]
fn local_dev_readiness_allows_fake_driver_with_degraded_status() {
    let mut registry = DriverRegistry::new();
    let fake_key = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "reference_echo_loop",
                1,
                "checkpoint_v1",
                1,
            ))),
            DriverRequirements::all_optional(),
            DriverKind::Reference,
        )
        .expect("reference driver registration should succeed");

    let report = registry.validate_readiness(
        DriverReadinessMode::LocalDevTest,
        DriverReadinessInputs {
            host_graph: HostGraphReadiness::all_available(),
            configured_profiles: vec![ConfiguredRunProfile::enabled("local_reference", fake_key)],
            persisted_runs: Vec::new(),
        },
    );

    assert_eq!(
        report.status,
        DriverReadinessStatus::LocalDevDegradedReference
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "reference_driver_allowed_for_local_dev")
    );
}

#[test]
fn readiness_requires_driver_for_non_terminal_run_identity() {
    let registry = DriverRegistry::new();
    let old_driver_key = key("lightweight_loop", 1, "checkpoint_v1", 1);

    let report = registry.validate_readiness(
        DriverReadinessMode::Production,
        DriverReadinessInputs {
            host_graph: HostGraphReadiness::all_available(),
            configured_profiles: Vec::new(),
            persisted_runs: vec![PersistedRunDriverIdentity::new(
                "run-1",
                TurnStatus::RecoveryRequired,
                old_driver_key,
            )],
        },
    );

    assert_eq!(report.status, DriverReadinessStatus::NotReady);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_non_terminal_run_driver")
    );
}

#[test]
fn readiness_ignores_terminal_run_driver_identity() {
    let registry = DriverRegistry::new();
    let old_driver_key = key("lightweight_loop", 1, "checkpoint_v1", 1);

    let report = registry.validate_readiness(
        DriverReadinessMode::Production,
        DriverReadinessInputs {
            host_graph: HostGraphReadiness::all_available(),
            configured_profiles: Vec::new(),
            persisted_runs: vec![PersistedRunDriverIdentity::new(
                "run-1",
                TurnStatus::Completed,
                old_driver_key,
            )],
        },
    );

    assert_eq!(report.status, DriverReadinessStatus::ProductionReady);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn registry_uses_registration_time_descriptor_snapshot() {
    let mut registry = DriverRegistry::new();
    let driver = Arc::new(TestDriver::new(descriptor(
        "lightweight_loop",
        1,
        "checkpoint_v1",
        1,
    )));

    let registered_key = registry
        .register_driver(
            driver.clone(),
            DriverRequirements::all_optional(),
            DriverKind::Production,
        )
        .expect("registration should succeed");
    driver.set_descriptor(descriptor("lightweight_loop", 2, "checkpoint_v2", 2));

    let registered = registry
        .get(&registered_key)
        .expect("registered entry should remain available by original key");
    assert_eq!(registered.descriptor().version, RunProfileVersion::new(1));
    assert_eq!(registered.key(), &registered_key);
    assert!(
        registry
            .get(&key("lightweight_loop", 2, "checkpoint_v2", 2))
            .is_none()
    );
}

#[test]
fn production_readiness_rejects_missing_required_host_component() {
    let mut registry = DriverRegistry::new();
    let driver_key = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "lightweight_loop",
                1,
                "checkpoint_v1",
                1,
            ))),
            DriverRequirements {
                model: RequirementLevel::Required,
                prompt: RequirementLevel::Optional,
                transcript: RequirementLevel::Optional,
                checkpoint: RequirementLevel::Optional,
                input_polling: RequirementLevel::Optional,
                capabilities: RequirementLevel::Optional,
                progress_events: RequirementLevel::Optional,
            },
            DriverKind::Production,
        )
        .expect("registration should succeed");

    let report = registry.validate_readiness(
        DriverReadinessMode::Production,
        DriverReadinessInputs {
            host_graph: HostGraphReadiness::all_available().without_model(),
            configured_profiles: vec![ConfiguredRunProfile::enabled(
                "interactive_default",
                driver_key,
            )],
            persisted_runs: Vec::new(),
        },
    );

    assert_eq!(report.status, DriverReadinessStatus::NotReady);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_required_driver_requirement")
    );
}

#[test]
fn production_readiness_enforces_required_prompt_port_availability() {
    let mut registry = DriverRegistry::new();
    let driver_key = registry
        .register_driver(
            Arc::new(TestDriver::new(descriptor(
                "lightweight_loop",
                1,
                "checkpoint_v1",
                1,
            ))),
            DriverRequirements {
                model: RequirementLevel::Optional,
                prompt: RequirementLevel::Required,
                transcript: RequirementLevel::Optional,
                checkpoint: RequirementLevel::Optional,
                input_polling: RequirementLevel::Optional,
                capabilities: RequirementLevel::Optional,
                progress_events: RequirementLevel::Optional,
            },
            DriverKind::Production,
        )
        .expect("registration should succeed");

    let report = registry.validate_readiness(
        DriverReadinessMode::Production,
        DriverReadinessInputs {
            host_graph: HostGraphReadiness::all_available().without_prompt(),
            configured_profiles: vec![ConfiguredRunProfile::enabled(
                "interactive_default",
                driver_key.clone(),
            )],
            persisted_runs: Vec::new(),
        },
    );

    assert_eq!(report.status, DriverReadinessStatus::NotReady);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "missing_required_driver_requirement"
            && diagnostic.message.contains("prompt bundle")
    }));

    let ready_report = registry.validate_readiness(
        DriverReadinessMode::Production,
        DriverReadinessInputs {
            host_graph: HostGraphReadiness::all_available(),
            configured_profiles: vec![ConfiguredRunProfile::enabled(
                "interactive_default",
                driver_key,
            )],
            persisted_runs: Vec::new(),
        },
    );

    assert_eq!(ready_report.status, DriverReadinessStatus::ProductionReady);
    assert!(ready_report.diagnostics.is_empty());
}

#[test]
fn registry_rejects_descriptor_with_partial_checkpoint_identity() {
    let mut registry = DriverRegistry::new();
    let error = registry
        .register_driver(
            Arc::new(TestDriver::new(AgentLoopDriverDescriptor {
                id: LoopDriverId::new("lightweight_loop").expect("valid driver id"),
                version: RunProfileVersion::new(1),
                checkpoint_schema_id: Some(
                    CheckpointSchemaId::new("checkpoint_v1").expect("valid checkpoint id"),
                ),
                checkpoint_schema_version: None,
            })),
            DriverRequirements::all_optional(),
            DriverKind::Production,
        )
        .expect_err("partial checkpoint identity should be invalid");

    assert!(matches!(
        error,
        DriverRegistryError::InvalidDescriptor { .. }
    ));
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

fn key(
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
    .expect("descriptor should produce a valid registry key")
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

    fn set_descriptor(&self, descriptor: AgentLoopDriverDescriptor) {
        *self.descriptor.lock().expect("test descriptor mutex") = descriptor;
    }
}

#[async_trait]
impl AgentLoopDriver for TestDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor
            .lock()
            .expect("test descriptor mutex")
            .clone()
    }

    async fn run(
        &self,
        _request: AgentLoopDriverRunRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        unreachable!("test driver is never executed")
    }

    async fn resume(
        &self,
        _request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        unreachable!("test driver is never executed")
    }
}
