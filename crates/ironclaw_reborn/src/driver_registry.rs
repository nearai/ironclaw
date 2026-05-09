//! Reborn agent-loop driver registry and readiness validation.
//!
//! `ironclaw_turns` owns the neutral driver/profile contracts. This module is
//! the Reborn composition layer that stores concrete driver instances, freezes
//! descriptor metadata at startup, and validates that configured and persisted
//! run identities can be served before traffic is accepted.

use std::{collections::HashMap, error::Error, fmt, sync::Arc};

use ironclaw_turns::{
    AgentLoopDriver, AgentLoopDriverDescriptor, RunProfileVersion, TurnStatus,
    run_profile::{CheckpointSchemaId, LoopDriverId},
};

/// Exact persisted identity for a registered loop driver.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoopDriverRegistryKey {
    pub id: LoopDriverId,
    pub version: RunProfileVersion,
    pub checkpoint_schema_id: Option<CheckpointSchemaId>,
    pub checkpoint_schema_version: Option<RunProfileVersion>,
}

impl LoopDriverRegistryKey {
    pub fn from_descriptor(descriptor: &AgentLoopDriverDescriptor) -> Result<Self, String> {
        validate_descriptor(descriptor)?;
        Ok(Self {
            id: descriptor.id.clone(),
            version: descriptor.version,
            checkpoint_schema_id: descriptor.checkpoint_schema_id.clone(),
            checkpoint_schema_version: descriptor.checkpoint_schema_version,
        })
    }
}

impl fmt::Display for LoopDriverRegistryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.id.as_str(), self.version.as_u64())?;
        if let (Some(schema_id), Some(schema_version)) = (
            self.checkpoint_schema_id.as_ref(),
            self.checkpoint_schema_version,
        ) {
            write!(
                f,
                " checkpoint {}@{}",
                schema_id.as_str(),
                schema_version.as_u64()
            )?;
        }
        Ok(())
    }
}

/// Host-service dependency level for a driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementLevel {
    Required,
    Optional,
    Unsupported,
}

/// Static requirements Reborn checks before selecting a driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverRequirements {
    pub model: RequirementLevel,
    pub transcript: RequirementLevel,
    pub checkpoint: RequirementLevel,
    pub input_polling: RequirementLevel,
    pub capabilities: RequirementLevel,
    pub progress_events: RequirementLevel,
}

impl DriverRequirements {
    pub fn all_optional() -> Self {
        Self {
            model: RequirementLevel::Optional,
            transcript: RequirementLevel::Optional,
            checkpoint: RequirementLevel::Optional,
            input_polling: RequirementLevel::Optional,
            capabilities: RequirementLevel::Optional,
            progress_events: RequirementLevel::Optional,
        }
    }

    pub fn all_required() -> Self {
        Self {
            model: RequirementLevel::Required,
            transcript: RequirementLevel::Required,
            checkpoint: RequirementLevel::Required,
            input_polling: RequirementLevel::Required,
            capabilities: RequirementLevel::Required,
            progress_events: RequirementLevel::Required,
        }
    }
}

impl Default for DriverRequirements {
    fn default() -> Self {
        Self::all_optional()
    }
}

/// Whether a driver can satisfy production readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    Production,
    /// Fake/reference drivers are allowed only in explicit local-dev/test readiness.
    Reference,
}

/// Concrete driver plus registration-time metadata snapshot.
pub struct RegisteredDriver {
    key: LoopDriverRegistryKey,
    driver: Arc<dyn AgentLoopDriver>,
    descriptor: AgentLoopDriverDescriptor,
    requirements: DriverRequirements,
    kind: DriverKind,
}

impl RegisteredDriver {
    pub fn key(&self) -> &LoopDriverRegistryKey {
        &self.key
    }

    pub fn driver(&self) -> Arc<dyn AgentLoopDriver> {
        Arc::clone(&self.driver)
    }

    pub fn descriptor(&self) -> &AgentLoopDriverDescriptor {
        &self.descriptor
    }

    pub fn requirements(&self) -> &DriverRequirements {
        &self.requirements
    }

    pub fn kind(&self) -> DriverKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverRegistryError {
    InvalidDescriptor { reason: String },
    DuplicateRegistration { key: LoopDriverRegistryKey },
}

impl fmt::Display for DriverRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDescriptor { reason } => write!(f, "invalid driver descriptor: {reason}"),
            Self::DuplicateRegistration { key } => {
                write!(f, "duplicate driver registration for {key}")
            }
        }
    }
}

impl Error for DriverRegistryError {}

#[derive(Default)]
pub struct DriverRegistry {
    entries: HashMap<LoopDriverRegistryKey, RegisteredDriver>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a concrete driver and cache its validated descriptor snapshot.
    pub fn register_driver(
        &mut self,
        driver: Arc<dyn AgentLoopDriver>,
        requirements: DriverRequirements,
        kind: DriverKind,
    ) -> Result<LoopDriverRegistryKey, DriverRegistryError> {
        let descriptor = driver.descriptor();
        let key = LoopDriverRegistryKey::from_descriptor(&descriptor)
            .map_err(|reason| DriverRegistryError::InvalidDescriptor { reason })?;
        if self.entries.contains_key(&key) {
            return Err(DriverRegistryError::DuplicateRegistration { key });
        }

        self.entries.insert(
            key.clone(),
            RegisteredDriver {
                key: key.clone(),
                driver,
                descriptor,
                requirements,
                kind,
            },
        );
        Ok(key)
    }

    pub fn get(&self, key: &LoopDriverRegistryKey) -> Option<&RegisteredDriver> {
        self.entries.get(key)
    }

    pub fn validate_readiness(
        &self,
        mode: DriverReadinessMode,
        inputs: DriverReadinessInputs,
    ) -> DriverReadinessReport {
        let mut diagnostics = Vec::new();
        let mut has_reference_driver = false;

        for profile in inputs
            .configured_profiles
            .iter()
            .filter(|profile| profile.enabled)
        {
            match self.get(&profile.driver_identity) {
                Some(entry) => {
                    validate_entry_readiness(
                        mode,
                        &inputs.host_graph,
                        &profile.name,
                        &profile.driver_identity,
                        entry,
                        &mut has_reference_driver,
                        &mut diagnostics,
                    );
                }
                None => diagnostics.push(DriverReadinessDiagnostic::new(
                    "missing_configured_driver",
                    profile.name.clone(),
                    Some(profile.driver_identity.clone()),
                    "enabled run profile references an unregistered loop driver identity",
                )),
            }
        }

        for persisted_run in inputs
            .persisted_runs
            .iter()
            .filter(|run| !run.status.is_terminal())
        {
            match self.get(&persisted_run.driver_identity) {
                Some(entry) => {
                    validate_entry_readiness(
                        mode,
                        &inputs.host_graph,
                        &persisted_run.run_id,
                        &persisted_run.driver_identity,
                        entry,
                        &mut has_reference_driver,
                        &mut diagnostics,
                    );
                }
                None => diagnostics.push(DriverReadinessDiagnostic::new(
                    "missing_non_terminal_run_driver",
                    persisted_run.run_id.clone(),
                    Some(persisted_run.driver_identity.clone()),
                    "non-terminal persisted run requires an unregistered loop driver identity",
                )),
            }
        }

        let status = if diagnostics.iter().any(|diagnostic| diagnostic.blocks_ready) {
            DriverReadinessStatus::NotReady
        } else if has_reference_driver {
            DriverReadinessStatus::LocalDevDegradedReference
        } else {
            DriverReadinessStatus::ProductionReady
        };

        DriverReadinessReport {
            status,
            diagnostics,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverReadinessMode {
    Production,
    LocalDevTest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverReadinessInputs {
    pub host_graph: HostGraphReadiness,
    pub configured_profiles: Vec<ConfiguredRunProfile>,
    pub persisted_runs: Vec<PersistedRunDriverIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredRunProfile {
    pub name: String,
    pub enabled: bool,
    pub driver_identity: LoopDriverRegistryKey,
}

impl ConfiguredRunProfile {
    pub fn enabled(name: impl Into<String>, driver_identity: LoopDriverRegistryKey) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            driver_identity,
        }
    }

    pub fn disabled(name: impl Into<String>, driver_identity: LoopDriverRegistryKey) -> Self {
        Self {
            name: name.into(),
            enabled: false,
            driver_identity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedRunDriverIdentity {
    pub run_id: String,
    pub status: TurnStatus,
    pub driver_identity: LoopDriverRegistryKey,
}

impl PersistedRunDriverIdentity {
    pub fn new(
        run_id: impl Into<String>,
        status: TurnStatus,
        driver_identity: LoopDriverRegistryKey,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            status,
            driver_identity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGraphReadiness {
    pub model: bool,
    pub transcript: bool,
    pub checkpoint: bool,
    pub input_polling: bool,
    pub capabilities: bool,
    pub progress_events: bool,
}

impl HostGraphReadiness {
    pub fn all_available() -> Self {
        Self {
            model: true,
            transcript: true,
            checkpoint: true,
            input_polling: true,
            capabilities: true,
            progress_events: true,
        }
    }

    pub fn without_model(mut self) -> Self {
        self.model = false;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverReadinessStatus {
    ProductionReady,
    LocalDevDegradedReference,
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverReadinessReport {
    pub status: DriverReadinessStatus,
    pub diagnostics: Vec<DriverReadinessDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverReadinessDiagnostic {
    pub code: &'static str,
    pub subject: String,
    pub driver_identity: Option<LoopDriverRegistryKey>,
    pub message: String,
    pub blocks_ready: bool,
}

impl DriverReadinessDiagnostic {
    fn new(
        code: &'static str,
        subject: String,
        driver_identity: Option<LoopDriverRegistryKey>,
        message: &'static str,
    ) -> Self {
        Self {
            code,
            subject,
            driver_identity,
            message: message.to_string(),
            blocks_ready: true,
        }
    }

    fn non_blocking(
        code: &'static str,
        subject: String,
        driver_identity: Option<LoopDriverRegistryKey>,
        message: &'static str,
    ) -> Self {
        Self {
            code,
            subject,
            driver_identity,
            message: message.to_string(),
            blocks_ready: false,
        }
    }
}

fn validate_descriptor(descriptor: &AgentLoopDriverDescriptor) -> Result<(), String> {
    if descriptor.version.as_u64() == 0 {
        return Err("driver version must be greater than zero".to_string());
    }
    match (
        descriptor.checkpoint_schema_id.as_ref(),
        descriptor.checkpoint_schema_version,
    ) {
        (Some(_), Some(version)) if version.as_u64() == 0 => {
            Err("checkpoint schema version must be greater than zero".to_string())
        }
        (Some(_), Some(_)) | (None, None) => Ok(()),
        (Some(_), None) | (None, Some(_)) => Err(
            "checkpoint schema id and checkpoint schema version must both be present or both absent"
                .to_string(),
        ),
    }
}

fn validate_entry_readiness(
    mode: DriverReadinessMode,
    host_graph: &HostGraphReadiness,
    subject: &str,
    driver_identity: &LoopDriverRegistryKey,
    entry: &RegisteredDriver,
    has_reference_driver: &mut bool,
    diagnostics: &mut Vec<DriverReadinessDiagnostic>,
) {
    match (mode, entry.kind()) {
        (DriverReadinessMode::Production, DriverKind::Reference) => {
            diagnostics.push(DriverReadinessDiagnostic::new(
                "reference_driver_not_production_ready",
                subject.to_string(),
                Some(driver_identity.clone()),
                "fake/reference loop driver cannot satisfy production readiness",
            ))
        }
        (DriverReadinessMode::LocalDevTest, DriverKind::Reference) => {
            *has_reference_driver = true;
            diagnostics.push(DriverReadinessDiagnostic::non_blocking(
                "reference_driver_allowed_for_local_dev",
                subject.to_string(),
                Some(driver_identity.clone()),
                "fake/reference loop driver allowed only for explicit local-dev/test readiness",
            ));
        }
        (_, DriverKind::Production) => {}
    }

    for requirement in missing_requirements(entry.requirements(), host_graph) {
        diagnostics.push(DriverReadinessDiagnostic::new(
            "missing_required_driver_requirement",
            subject.to_string(),
            Some(driver_identity.clone()),
            requirement.message,
        ));
    }
}

struct MissingRequirement {
    message: &'static str,
}

fn missing_requirements(
    requirements: &DriverRequirements,
    host_graph: &HostGraphReadiness,
) -> Vec<MissingRequirement> {
    let mut missing = Vec::new();
    push_missing(
        &mut missing,
        requirements.model,
        host_graph.model,
        "driver requires a model gateway, but the host graph does not provide one",
    );
    push_missing(
        &mut missing,
        requirements.transcript,
        host_graph.transcript,
        "driver requires transcript storage, but the host graph does not provide it",
    );
    push_missing(
        &mut missing,
        requirements.checkpoint,
        host_graph.checkpoint,
        "driver requires checkpoint storage, but the host graph does not provide it",
    );
    push_missing(
        &mut missing,
        requirements.input_polling,
        host_graph.input_polling,
        "driver requires input polling, but the host graph does not provide it",
    );
    push_missing(
        &mut missing,
        requirements.capabilities,
        host_graph.capabilities,
        "driver requires capability execution, but the host graph does not provide it",
    );
    push_missing(
        &mut missing,
        requirements.progress_events,
        host_graph.progress_events,
        "driver requires progress events, but the host graph does not provide them",
    );
    missing
}

fn push_missing(
    missing: &mut Vec<MissingRequirement>,
    level: RequirementLevel,
    available: bool,
    message: &'static str,
) {
    if level == RequirementLevel::Required && !available {
        missing.push(MissingRequirement { message });
    }
}
