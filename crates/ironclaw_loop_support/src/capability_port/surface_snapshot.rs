use std::collections::HashMap;

use ironclaw_host_api::{CapabilityId, EffectKind, ExtensionId, ResourceEstimate, RuntimeKind};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, ProviderToolCall, ProviderToolDefinition,
};

use crate::capability_info::{self, CapabilityInfoEntry};

#[derive(Clone)]
pub(super) struct RuntimeSurfaceCapabilitySnapshot {
    pub(super) provider: ExtensionId,
    pub(super) runtime: RuntimeKind,
    pub(super) estimate: ResourceEstimate,
    pub(super) safe_description: String,
    pub(super) parameters_schema: serde_json::Value,
    pub(super) effects: Vec<EffectKind>,
    pub(super) provider_tool_name: String,
}

#[derive(Clone)]
pub(super) struct SyntheticSurfaceCapabilitySnapshot {
    provider_tool_name: String,
    kind: SyntheticCapabilityKind,
}

#[derive(Clone, Copy)]
enum SyntheticCapabilityKind {
    CapabilityInfo,
}

#[derive(Clone)]
pub(super) enum SurfaceCapabilitySnapshot {
    Runtime(Box<RuntimeSurfaceCapabilitySnapshot>),
    Synthetic(SyntheticSurfaceCapabilitySnapshot),
}

#[derive(Clone, Default)]
pub(super) struct SurfaceSnapshot {
    pub(super) capabilities: HashMap<CapabilityId, SurfaceCapabilitySnapshot>,
    pub(super) provider_names: HashMap<String, CapabilityId>,
}

pub(super) struct PreparedSurfaceCapabilityCall {
    pub(super) capability_id: CapabilityId,
    pub(super) normalized_arguments: serde_json::Value,
    pub(super) effective_capability_ids: Vec<CapabilityId>,
}

impl SurfaceSnapshot {
    pub(super) fn with_synthetic_capabilities() -> Result<Self, AgentLoopHostError> {
        let mut snapshot = Self::default();
        snapshot.insert_synthetic_capabilities()?;
        Ok(snapshot)
    }

    fn insert_synthetic_capabilities(&mut self) -> Result<(), AgentLoopHostError> {
        let capability_id = capability_info::capability_id()?;
        self.provider_names.insert(
            capability_info::TOOL_NAME.to_string(),
            capability_id.clone(),
        );
        self.capabilities.insert(
            capability_id,
            SurfaceCapabilitySnapshot::Synthetic(SyntheticSurfaceCapabilitySnapshot {
                provider_tool_name: capability_info::TOOL_NAME.to_string(),
                kind: SyntheticCapabilityKind::CapabilityInfo,
            }),
        );
        Ok(())
    }

    pub(super) fn capability_info(&self, requested: &str) -> Option<CapabilityInfoEntry<'_>> {
        if let Some(capability_id) = self.provider_names.get(requested)
            && let Some(capability) = self
                .capabilities
                .get(capability_id)
                .and_then(SurfaceCapabilitySnapshot::as_runtime)
        {
            return Some(capability.capability_info(capability_id));
        }
        let requested_id = CapabilityId::new(requested).ok()?;
        self.capabilities
            .get_key_value(&requested_id)
            .and_then(|(capability_id, capability)| {
                capability
                    .as_runtime()
                    .map(|capability| (capability_id, capability))
            })
            .map(|(capability_id, capability)| capability.capability_info(capability_id))
    }

    pub(super) fn provider_capability(
        &self,
        provider_tool_name: &str,
    ) -> Result<(&CapabilityId, &SurfaceCapabilitySnapshot), AgentLoopHostError> {
        let Some(capability_id) = self.provider_names.get(provider_tool_name) else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call is outside the visible capability surface",
            ));
        };
        let Some(capability) = self.capabilities.get(capability_id) else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface snapshot is missing provider metadata",
            ));
        };
        Ok((capability_id, capability))
    }
}

impl RuntimeSurfaceCapabilitySnapshot {
    fn capability_info<'a>(&'a self, capability_id: &'a CapabilityId) -> CapabilityInfoEntry<'a> {
        CapabilityInfoEntry {
            capability_id,
            provider_tool_name: &self.provider_tool_name,
            safe_description: &self.safe_description,
            parameters_schema: &self.parameters_schema,
            runtime: self.runtime,
            effects: &self.effects,
        }
    }
}

impl SurfaceCapabilitySnapshot {
    fn as_runtime(&self) -> Option<&RuntimeSurfaceCapabilitySnapshot> {
        match self {
            Self::Runtime(capability) => Some(capability.as_ref()),
            Self::Synthetic(_) => None,
        }
    }

    pub(super) fn tool_definition(
        &self,
        capability_id: &CapabilityId,
    ) -> Result<Option<ProviderToolDefinition>, AgentLoopHostError> {
        match self {
            Self::Runtime(capability) => {
                if !super::provider_schema_is_usable(&capability.parameters_schema) {
                    tracing::debug!(
                        capability_id = capability_id.as_str(),
                        "capability omitted from provider tool definitions because its parameter schema is not provider-usable"
                    );
                    return Ok(None);
                }
                Ok(Some(ProviderToolDefinition {
                    capability_id: capability_id.clone(),
                    name: capability.provider_tool_name.clone(),
                    description: capability.safe_description.clone(),
                    parameters: capability.parameters_schema.clone(),
                }))
            }
            Self::Synthetic(capability) => capability.tool_definition(capability_id).map(Some),
        }
    }

    pub(super) fn prepare_provider_tool_call(
        &self,
        capability_id: &CapabilityId,
        snapshot: &SurfaceSnapshot,
        tool_call: &ProviderToolCall,
    ) -> Result<PreparedSurfaceCapabilityCall, AgentLoopHostError> {
        match self {
            Self::Runtime(capability) => {
                capability.prepare_provider_tool_call(capability_id, tool_call)
            }
            Self::Synthetic(capability) => {
                capability.prepare_provider_tool_call(capability_id, snapshot, tool_call)
            }
        }
    }
}

impl RuntimeSurfaceCapabilitySnapshot {
    fn prepare_provider_tool_call(
        &self,
        capability_id: &CapabilityId,
        tool_call: &ProviderToolCall,
    ) -> Result<PreparedSurfaceCapabilityCall, AgentLoopHostError> {
        if !super::provider_schema_is_usable(&self.parameters_schema) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "provider tool call was not advertised to the model",
            ));
        }
        let normalized_arguments = super::normalize_provider_arguments(
            &tool_call.arguments,
            &self.parameters_schema,
            "provider arguments",
        )?;
        super::validate_provider_arguments(&normalized_arguments)?;
        Ok(PreparedSurfaceCapabilityCall {
            capability_id: capability_id.clone(),
            normalized_arguments,
            effective_capability_ids: vec![capability_id.clone()],
        })
    }
}

impl SyntheticSurfaceCapabilitySnapshot {
    fn tool_definition(
        &self,
        capability_id: &CapabilityId,
    ) -> Result<ProviderToolDefinition, AgentLoopHostError> {
        match self.kind {
            SyntheticCapabilityKind::CapabilityInfo => {
                debug_assert!(capability_info::is_capability_id(capability_id));
                let mut definition = capability_info::tool_definition()?;
                definition.name = self.provider_tool_name.clone();
                Ok(definition)
            }
        }
    }

    fn prepare_provider_tool_call(
        &self,
        capability_id: &CapabilityId,
        snapshot: &SurfaceSnapshot,
        tool_call: &ProviderToolCall,
    ) -> Result<PreparedSurfaceCapabilityCall, AgentLoopHostError> {
        match self.kind {
            SyntheticCapabilityKind::CapabilityInfo => {
                let normalized_arguments = super::normalize_provider_arguments(
                    &tool_call.arguments,
                    &capability_info::schema(),
                    "provider arguments",
                )?;
                super::validate_provider_arguments(&normalized_arguments)?;
                let request = capability_info::CapabilityInfoRequest::parse(&normalized_arguments)?;
                let target = snapshot
                    .capability_info(request.requested_name())
                    .ok_or_else(|| {
                        AgentLoopHostError::new(
                            AgentLoopHostErrorKind::InvalidInvocation,
                            "capability_info target is not on the visible surface",
                        )
                    })?;
                Ok(PreparedSurfaceCapabilityCall {
                    capability_id: capability_id.clone(),
                    normalized_arguments,
                    effective_capability_ids: vec![
                        capability_id.clone(),
                        target.capability_id.clone(),
                    ],
                })
            }
        }
    }

    pub(super) fn output<'a>(
        &self,
        input: &serde_json::Value,
        resolve: impl FnOnce(&str) -> Option<CapabilityInfoEntry<'a>>,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        match self.kind {
            SyntheticCapabilityKind::CapabilityInfo => capability_info::output(input, resolve),
        }
    }
}
