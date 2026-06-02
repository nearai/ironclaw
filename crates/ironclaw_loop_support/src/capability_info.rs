use std::collections::BTreeSet;

use ironclaw_host_api::{CapabilityId, EffectKind, RuntimeKind};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, ProviderToolDefinition,
};

pub(crate) const TOOL_NAME: &str = "capability_info";
pub(crate) const CAPABILITY_ID: &str = "ironclaw.loop.capability_info";

pub(super) struct CapabilityInfoEntry<'a> {
    pub(super) capability_id: &'a CapabilityId,
    pub(super) provider_tool_name: &'a str,
    pub(super) safe_description: &'a str,
    pub(super) parameters_schema: &'a serde_json::Value,
    pub(super) runtime: RuntimeKind,
    pub(super) effects: &'a [EffectKind],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Detail {
    Names,
    Summary,
    Schema,
}

impl Detail {
    fn parse(input: &serde_json::Value) -> Result<Self, AgentLoopHostError> {
        if let Some(include_schema) = input.get("include_schema") {
            let Some(include_schema) = include_schema.as_bool() else {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "capability_info include_schema must be boolean",
                ));
            };
            if include_schema {
                return Ok(Self::Schema);
            }
        }
        let Some(detail) = input.get("detail") else {
            return Ok(Self::Names);
        };
        let Some(detail) = detail.as_str() else {
            return Err(invalid_detail());
        };
        match detail {
            "names" => Ok(Self::Names),
            "summary" => Ok(Self::Summary),
            "schema" => Ok(Self::Schema),
            _ => Err(invalid_detail()),
        }
    }
}

pub(super) struct CapabilityInfoRequest {
    requested_name: String,
    detail: Detail,
}

impl CapabilityInfoRequest {
    pub(super) fn parse(input: &serde_json::Value) -> Result<Self, AgentLoopHostError> {
        Ok(Self {
            requested_name: requested_name(input)?.to_string(),
            detail: Detail::parse(input)?,
        })
    }

    pub(super) fn requested_name(&self) -> &str {
        self.requested_name.as_str()
    }

    fn detail(&self) -> Detail {
        self.detail
    }
}

pub(crate) fn capability_id() -> Result<CapabilityId, AgentLoopHostError> {
    CapabilityId::new(CAPABILITY_ID).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "capability info id could not be represented",
        )
    })
}

pub(crate) fn is_capability_id(capability_id: &CapabilityId) -> bool {
    capability_id.as_str() == CAPABILITY_ID
}

pub(super) fn tool_definition() -> Result<ProviderToolDefinition, AgentLoopHostError> {
    Ok(ProviderToolDefinition {
        capability_id: capability_id()?,
        name: TOOL_NAME.to_string(),
        description: "Get names, summary, or schema details for a currently visible capability."
            .to_string(),
        parameters: schema(),
    })
}

pub(super) fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Visible provider tool name or canonical capability id to inspect"
            },
            "capability_id": {
                "type": "string",
                "description": "Deprecated alias for name when passing a canonical capability id"
            },
            "detail": {
                "type": "string",
                "enum": ["names", "summary", "schema"],
                "default": "names",
                "description": "Response detail level. names returns parameter names only, summary adds required fields and effect notes, schema returns the full input schema."
            },
            "include_schema": {
                "type": "boolean",
                "default": false,
                "description": "Compatibility alias for detail=schema."
            }
        },
        "required": ["name"],
    })
}

pub(super) fn output<'a>(
    input: &serde_json::Value,
    resolve: impl FnOnce(&str) -> Option<CapabilityInfoEntry<'a>>,
) -> Result<serde_json::Value, AgentLoopHostError> {
    let request = CapabilityInfoRequest::parse(input)?;
    let capability = resolve(request.requested_name()).ok_or_else(target_not_visible)?;
    let schema_summary = SchemaSummary::for_schema(capability.parameters_schema);
    let mut output = serde_json::json!({
        "name": capability.provider_tool_name,
        "capability_id": capability.capability_id.as_str(),
        "description": capability.safe_description,
        "parameters": schema_summary.parameter_names,
    });
    match request.detail() {
        Detail::Names => {}
        Detail::Summary => {
            output["summary"] = serde_json::json!({
                "always_required": schema_summary.required_names,
                "notes": notes(&capability),
            });
        }
        Detail::Schema => {
            output["schema"] = capability.parameters_schema.clone();
        }
    }
    Ok(output)
}

pub(super) fn requested_name(input: &serde_json::Value) -> Result<&str, AgentLoopHostError> {
    let requested = input
        .get("name")
        .or_else(|| input.get("capability_id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability_info requires name",
            )
        })?;
    validate_name(requested)?;
    Ok(requested)
}

fn validate_name(value: &str) -> Result<(), AgentLoopHostError> {
    if value.is_empty() || value.len() > 160 {
        return Err(invalid_name());
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(invalid_name());
    }
    Ok(())
}

struct SchemaSummary {
    parameter_names: Vec<String>,
    required_names: Vec<String>,
}

impl SchemaSummary {
    fn for_schema(schema: &serde_json::Value) -> Self {
        let mut parameter_names = BTreeSet::new();
        let mut required_names = BTreeSet::new();
        let mut stack = vec![(schema, true)];
        while let Some((current, contributes_required)) = stack.pop() {
            if let Some(properties) = current
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                parameter_names.extend(properties.keys().cloned());
            }
            if contributes_required
                && let Some(required) = current
                    .get("required")
                    .and_then(serde_json::Value::as_array)
            {
                required_names.extend(
                    required
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string)),
                );
            }
            if let Some(variants) = current.get("allOf").and_then(serde_json::Value::as_array) {
                stack.extend(
                    variants
                        .iter()
                        .map(|variant| (variant, contributes_required)),
                );
            }
            for key in ["oneOf", "anyOf"] {
                if let Some(variants) = current.get(key).and_then(serde_json::Value::as_array) {
                    stack.extend(variants.iter().map(|variant| (variant, false)));
                }
            }
        }
        Self {
            parameter_names: parameter_names.into_iter().collect(),
            required_names: required_names.into_iter().collect(),
        }
    }
}

fn notes(capability: &CapabilityInfoEntry<'_>) -> Vec<String> {
    let mut notes = vec![format!(
        "runtime: {}",
        runtime_kind_label(capability.runtime)
    )];
    if !capability.effects.is_empty() {
        notes.push(format!(
            "effects: {}",
            capability
                .effects
                .iter()
                .map(|effect| effect_kind_label(*effect))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    notes
}

fn runtime_kind_label(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Wasm => "wasm",
        RuntimeKind::Mcp => "mcp",
        RuntimeKind::Script => "script",
        RuntimeKind::FirstParty => "first_party",
        RuntimeKind::System => "system",
    }
}

fn effect_kind_label(effect: EffectKind) -> &'static str {
    match effect {
        EffectKind::ReadFilesystem => "read_filesystem",
        EffectKind::WriteFilesystem => "write_filesystem",
        EffectKind::DeleteFilesystem => "delete_filesystem",
        EffectKind::Network => "network",
        EffectKind::UseSecret => "use_secret",
        EffectKind::ExecuteCode => "execute_code",
        EffectKind::SpawnProcess => "spawn_process",
        EffectKind::DispatchCapability => "dispatch_capability",
        EffectKind::ModifyExtension => "modify_extension",
        EffectKind::ModifyApproval => "modify_approval",
        EffectKind::ModifyBudget => "modify_budget",
        EffectKind::ExternalWrite => "external_write",
        EffectKind::Financial => "financial",
    }
}

fn invalid_detail() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        "capability_info detail must be names, summary, or schema",
    )
}

fn invalid_name() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        "capability_info name is invalid",
    )
}

fn target_not_visible() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        "capability_info target is not on the visible surface",
    )
}
