//! Production registration helpers for the default planned Reborn loop.

use std::{error::Error, fmt, sync::Arc};

use ironclaw_agent_loop::{
    executor::CanonicalAgentLoopExecutor,
    family::{LoopFamilyId, LoopFamilyRegistry},
    state::CHECKPOINT_SCHEMA_ID,
};
use ironclaw_turns::{
    AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, RunProfileId,
    RunProfileVersion,
    run_profile::{
        CapabilitySurfaceProfileId, CheckpointSchemaId, InMemoryRunProfileRegistry,
        InMemoryRunProfileResolver, PersonalContextPolicy, RunProfileDefinition,
        RunProfileRegistryError,
    },
};

use crate::{
    driver_registry::{
        DriverKind, DriverRegistry, DriverRegistryError, DriverRequirements, LoopDriverRegistryKey,
        RequirementLevel,
    },
    planned_driver::PlannedDriver,
    text_loop_driver::{TextOnlyModelReplyDriver, TextOnlyModelReplyDriverConfig},
};

pub const PLANNED_DRIVER_DEFAULT_ID: &str = "reborn:planned-default";
pub const PLANNED_DRIVER_DEFAULT_VERSION: u64 = 1;
pub const PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID: &str = CHECKPOINT_SCHEMA_ID;
pub const PLANNED_DRIVER_CHECKPOINT_SCHEMA_VERSION: u64 = 1;
pub const PLANNED_DEFAULT_PROFILE_ID: &str = "reborn-planned-default";

pub struct DefaultPlannedDriverBuild {
    pub driver: Arc<dyn AgentLoopDriver>,
    pub descriptor: AgentLoopDriverDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultPlannedDriverRegistrationError {
    DriverBuild(AgentLoopDriverError),
    Registry(DriverRegistryError),
}

impl fmt::Display for DefaultPlannedDriverRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DriverBuild(error) => {
                write!(formatter, "default planned driver build failed: {error}")
            }
            Self::Registry(error) => write!(
                formatter,
                "default planned driver registration failed: {error}"
            ),
        }
    }
}

impl Error for DefaultPlannedDriverRegistrationError {}

impl From<AgentLoopDriverError> for DefaultPlannedDriverRegistrationError {
    fn from(error: AgentLoopDriverError) -> Self {
        Self::DriverBuild(error)
    }
}

impl From<DriverRegistryError> for DefaultPlannedDriverRegistrationError {
    fn from(error: DriverRegistryError) -> Self {
        Self::Registry(error)
    }
}

pub fn planned_driver_default_id() -> Result<ironclaw_turns::run_profile::LoopDriverId, String> {
    ironclaw_turns::run_profile::LoopDriverId::new(PLANNED_DRIVER_DEFAULT_ID)
}

pub fn planned_driver_checkpoint_schema_id() -> Result<CheckpointSchemaId, String> {
    CheckpointSchemaId::new(PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID)
}

pub fn planned_default_profile_id() -> Result<RunProfileId, String> {
    RunProfileId::new(PLANNED_DEFAULT_PROFILE_ID)
}

pub fn planned_driver_default_version() -> RunProfileVersion {
    RunProfileVersion::new(PLANNED_DRIVER_DEFAULT_VERSION)
}

pub fn planned_driver_checkpoint_schema_version() -> RunProfileVersion {
    RunProfileVersion::new(PLANNED_DRIVER_CHECKPOINT_SCHEMA_VERSION)
}

pub fn planned_driver_descriptor() -> Result<AgentLoopDriverDescriptor, String> {
    AgentLoopDriverDescriptor::new(PLANNED_DRIVER_DEFAULT_ID, planned_driver_default_version())?
        .with_checkpoint_schema(
            PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID,
            planned_driver_checkpoint_schema_version(),
        )
}

pub fn default_planned_driver(
    family_registry: Arc<LoopFamilyRegistry>,
) -> Result<DefaultPlannedDriverBuild, AgentLoopDriverError> {
    let family = family_registry.get(&LoopFamilyId::DEFAULT).ok_or_else(|| {
        AgentLoopDriverError::InvalidRequest {
            reason: "default loop family is not registered".to_string(),
        }
    })?;
    let descriptor = planned_driver_descriptor()
        .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?;
    let executor = Arc::new(CanonicalAgentLoopExecutor);
    let driver = PlannedDriver::from_family_with_descriptor(family, executor, descriptor.clone())?;
    Ok(DefaultPlannedDriverBuild {
        driver: Arc::new(driver),
        descriptor,
    })
}

pub fn planned_driver_requirements() -> DriverRequirements {
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

pub fn register_default_planned_driver(
    registry: &mut DriverRegistry,
    family_registry: Arc<LoopFamilyRegistry>,
) -> Result<LoopDriverRegistryKey, DefaultPlannedDriverRegistrationError> {
    let build = default_planned_driver(family_registry)?;
    registry
        .register_driver(
            build.driver,
            planned_driver_requirements(),
            DriverKind::Production,
        )
        .map_err(Into::into)
}

pub fn register_default_text_only_driver(
    registry: &mut DriverRegistry,
    config: TextOnlyModelReplyDriverConfig,
) -> Result<LoopDriverRegistryKey, DriverRegistryError> {
    registry.register_driver(
        Arc::new(TextOnlyModelReplyDriver::new(config)),
        DriverRequirements::all_optional(),
        DriverKind::Production,
    )
}

pub fn planned_default_profile_definition() -> Result<RunProfileDefinition, RunProfileRegistryError>
{
    let descriptor = planned_driver_descriptor()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    let profile_id = planned_default_profile_id()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    let checkpoint_schema_id = planned_driver_checkpoint_schema_id()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    let capability_surface_profile_id = CapabilitySurfaceProfileId::new("interactive_tools")
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    Ok(RunProfileDefinition::interactive_like(
        profile_id,
        descriptor,
        checkpoint_schema_id,
        planned_driver_checkpoint_schema_version(),
        capability_surface_profile_id,
    )
    .with_personal_context_policy(PersonalContextPolicy::Allowed))
}

pub fn register_default_planned_profile(
    registry: &mut InMemoryRunProfileRegistry,
) -> Result<(), RunProfileRegistryError> {
    registry.register(planned_default_profile_definition()?)
}

pub fn default_planned_run_profile_resolver()
-> Result<InMemoryRunProfileResolver, RunProfileRegistryError> {
    let mut registry = InMemoryRunProfileRegistry::with_builtin_profiles();
    register_default_planned_profile(&mut registry)?;
    let implicit_default = planned_default_profile_id()
        .map_err(|reason| RunProfileRegistryError::InvalidProfile { reason })?;
    Ok(InMemoryRunProfileResolver::new_with_implicit_default(
        registry,
        implicit_default,
    ))
}

#[cfg(test)]
mod tests {
    use ironclaw_turns::{
        RunProfileRequest, RunProfileResolutionRequest, RunProfileResolver,
        run_profile::{LoopDriverId, PersonalContextPolicy},
    };

    use super::*;
    use crate::app_loop_family::build_loop_family_registry;

    #[test]
    fn descriptor_carries_checkpoint_schema() {
        let descriptor = planned_driver_descriptor().expect("static descriptor should validate");

        assert_eq!(descriptor.id.as_str(), PLANNED_DRIVER_DEFAULT_ID);
        assert_eq!(descriptor.version, planned_driver_default_version());
        assert_eq!(
            descriptor
                .checkpoint_schema_id
                .as_ref()
                .map(|id| id.as_str()),
            Some(PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID)
        );
        assert_eq!(
            descriptor.checkpoint_schema_version,
            Some(planned_driver_checkpoint_schema_version())
        );
    }

    #[test]
    fn register_default_planned_driver_uses_v1_schema() {
        let mut registry = DriverRegistry::new();
        let key = register_default_planned_driver(
            &mut registry,
            build_loop_family_registry().expect("family registry should build"),
        )
        .expect("planned driver should register");

        assert_eq!(key.id.as_str(), PLANNED_DRIVER_DEFAULT_ID);
        assert_eq!(key.version, planned_driver_default_version());
        assert_eq!(
            key.checkpoint_schema_id.as_ref().map(|id| id.as_str()),
            Some(PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID)
        );
        assert_eq!(
            key.checkpoint_schema_version,
            Some(planned_driver_checkpoint_schema_version())
        );
    }

    #[test]
    fn key_collision_with_textonly_is_impossible() {
        let mut registry = DriverRegistry::new();
        let text_only_key = register_default_text_only_driver(
            &mut registry,
            TextOnlyModelReplyDriverConfig::default(),
        )
        .expect("text-only driver should register");
        let planned_key = register_default_planned_driver(
            &mut registry,
            build_loop_family_registry().expect("family registry should build"),
        )
        .expect("planned driver should register");

        assert_ne!(text_only_key, planned_key);
        assert_eq!(text_only_key.id.as_str(), "reborn:text-only-model-reply");
        assert_eq!(planned_key.id.as_str(), PLANNED_DRIVER_DEFAULT_ID);
    }

    #[tokio::test]
    async fn profile_resolves_to_planned_driver() {
        let mut registry = InMemoryRunProfileRegistry::with_builtin_profiles();
        register_default_planned_profile(&mut registry).expect("profile should register");
        let resolver = InMemoryRunProfileResolver::new(registry);
        let snapshot = resolver
            .resolve_run_profile(
                RunProfileResolutionRequest::interactive_default().with_requested_run_profile(
                    RunProfileRequest::new(PLANNED_DEFAULT_PROFILE_ID).unwrap(),
                ),
            )
            .await
            .expect("profile should resolve");

        assert_eq!(snapshot.profile_id.as_str(), PLANNED_DEFAULT_PROFILE_ID);
        assert_eq!(snapshot.loop_driver.id.as_str(), PLANNED_DRIVER_DEFAULT_ID);
        assert_eq!(
            snapshot.personal_context_policy,
            PersonalContextPolicy::Allowed
        );
        assert_eq!(
            snapshot
                .loop_driver
                .checkpoint_schema_id
                .as_ref()
                .map(|id| id.as_str()),
            Some(PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID)
        );
    }

    #[tokio::test]
    async fn implicit_default_resolves_to_planned_driver() {
        let resolver =
            default_planned_run_profile_resolver().expect("planned resolver should build");
        let snapshot = resolver
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("implicit default should resolve");

        assert_eq!(snapshot.profile_id.as_str(), PLANNED_DEFAULT_PROFILE_ID);
        assert_eq!(snapshot.loop_driver.id.as_str(), PLANNED_DRIVER_DEFAULT_ID);
        assert_eq!(
            snapshot.personal_context_policy,
            PersonalContextPolicy::Allowed
        );
    }

    #[tokio::test]
    async fn explicit_text_only_profile_still_resolves_textonly() {
        let resolver =
            default_planned_run_profile_resolver().expect("planned resolver should build");
        let snapshot = resolver
            .resolve_run_profile(
                RunProfileResolutionRequest::interactive_default().with_requested_run_profile(
                    RunProfileRequest::new("interactive_default").unwrap(),
                ),
            )
            .await
            .expect("explicit text-only profile should resolve");

        assert_eq!(
            snapshot.loop_driver.id,
            LoopDriverId::new("lightweight_loop").unwrap()
        );
    }
}
