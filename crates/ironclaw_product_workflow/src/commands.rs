//! Reborn-native product command contract.
//!
//! Slash strings are only an edge syntax. This module starts from normalized
//! command payloads so command parsing does not depend on v1 agent routing or on
//! the product surface that produced the command.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    InboundCommandPayload, ProductCommandResultPayload, ProductInboundAck, ProductRejection,
    ProductRejectionKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ProductCommandContext, ProductCommandService, ProductWorkflowError,
    lifecycle::{
        LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction,
        LifecycleProductContext, LifecycleProductFacade, validate_lifecycle_text,
    },
};

/// Public command inventory metadata. Policy decisions based on actor,
/// installation, trigger, or product surface belong to `ProductCommandAdmissionService`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductCommandDescriptor {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
}

struct ProductCommandSpec {
    descriptor: ProductCommandDescriptor,
    parse: fn(&InboundCommandPayload) -> ProductCommand,
}

const COMMAND_SPECS: &[ProductCommandSpec] = &[
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "extension_search",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "extension_install",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "extension_auth",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "extension_activate",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "extension_configure",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "extension_remove",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "model",
            aliases: &[],
        },
        parse: parse_model_command,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "status",
            aliases: &["progress"],
        },
        parse: parse_status_command,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "skill_search",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "skill_install",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "skill_remove",
            aliases: &[],
        },
        parse: parse_lifecycle_command_payload,
    },
];

pub fn product_command_descriptors() -> impl Iterator<Item = &'static ProductCommandDescriptor> {
    COMMAND_SPECS.iter().map(|spec| &spec.descriptor)
}

/// Typed command family produced from a normalized command payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ProductCommand {
    Lifecycle { action: LifecycleProductAction },
    Model { action: ProductModelCommand },
    Status,
    Invalid { name: String, reason: String },
    Unknown { name: String, arguments: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProductModelCommand {
    Status,
    Set { model: String },
}

impl ProductCommand {
    pub fn from_payload(payload: &InboundCommandPayload) -> Self {
        match command_spec_for_name(&payload.command) {
            Some(spec) => (spec.parse)(payload),
            None => Self::Unknown {
                name: payload.command.clone(),
                arguments: payload.arguments.clone(),
            },
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Lifecycle { action } => action.command_name(),
            Self::Model { .. } => "model",
            Self::Status => "status",
            Self::Invalid { name, .. } => name.as_str(),
            Self::Unknown { name, .. } => name.as_str(),
        }
    }

    pub fn descriptor(&self) -> Option<&'static ProductCommandDescriptor> {
        command_spec_for_name(self.name()).map(|spec| &spec.descriptor)
    }

    pub(crate) fn invalid_rejection(&self) -> Option<ProductRejection> {
        let Self::Invalid { reason, .. } = self else {
            return None;
        };
        Some(ProductRejection::permanent(
            ProductRejectionKind::InvalidRequest,
            reason.clone(),
        ))
    }
}

fn command_spec_for_name(name: &str) -> Option<&'static ProductCommandSpec> {
    COMMAND_SPECS
        .iter()
        .find(|spec| spec.descriptor.name == name || spec.descriptor.aliases.contains(&name))
}

fn parse_model_command(payload: &InboundCommandPayload) -> ProductCommand {
    let model = payload.arguments.split_whitespace().next();
    match model {
        Some(model) => ProductCommand::Model {
            action: ProductModelCommand::Set {
                model: model.to_string(),
            },
        },
        None => ProductCommand::Model {
            action: ProductModelCommand::Status,
        },
    }
}

fn parse_status_command(_payload: &InboundCommandPayload) -> ProductCommand {
    ProductCommand::Status
}

pub struct LifecycleProductCommandService {
    facade: Arc<dyn LifecycleProductFacade>,
}

impl LifecycleProductCommandService {
    pub fn new(facade: Arc<dyn LifecycleProductFacade>) -> Self {
        Self { facade }
    }
}

#[async_trait]
impl ProductCommandService for LifecycleProductCommandService {
    async fn execute(
        &self,
        context: ProductCommandContext,
        command: ProductCommand,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let ProductCommand::Lifecycle { action } = command else {
            return Ok(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("command routing unavailable: {}", command.name()),
            )));
        };
        let command_name = action.command_name().to_string();
        let response = self
            .facade
            .execute(LifecycleProductContext::Command(Box::new(context)), action)
            .await?;
        let payload =
            serde_json::to_value(response).map_err(|error| ProductWorkflowError::Transient {
                reason: format!("lifecycle command response serialization failed: {error}"),
            })?;
        Ok(ProductInboundAck::CommandResult {
            command: command_name,
            payload: ProductCommandResultPayload::new(payload),
        })
    }
}

fn parse_lifecycle_command_payload(payload: &InboundCommandPayload) -> ProductCommand {
    match payload.command.as_str() {
        "extension_search" => ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionSearch {
                query: payload.arguments.trim().to_string(),
            },
        },
        "extension_install" => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionInstall { package_ref }
        }),
        "extension_auth" => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionAuth { package_ref }
        }),
        "extension_activate" => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionActivate { package_ref }
        }),
        "extension_configure" => parse_extension_configure_command(payload),
        "extension_remove" => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionRemove { package_ref }
        }),
        "skill_search" => ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillSearch {
                query: payload.arguments.trim().to_string(),
            },
        },
        "skill_install" => parse_skill_install_command(payload),
        "skill_remove" => parse_skill_remove_command(payload),
        _ => unknown_lifecycle_command(payload),
    }
}

fn parse_extension_configure_command(payload: &InboundCommandPayload) -> ProductCommand {
    let args = payload.arguments.trim();
    let (id, config_payload) = match serde_json::from_str::<Value>(args) {
        Ok(json) => {
            let Some(id) = json.get("id").and_then(Value::as_str).map(str::to_string) else {
                return invalid_lifecycle_command(payload, "extension_configure.id is required");
            };
            (id, json.get("payload").cloned())
        }
        Err(_) => (first_argument(args).to_string(), None),
    };
    match lifecycle_package_ref(LifecyclePackageKind::Extension, id) {
        Ok(package_ref) => ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionConfigure {
                package_ref,
                payload: config_payload,
            },
        },
        Err(error) => invalid_lifecycle_command(payload, error.to_string()),
    }
}

fn parse_skill_install_command(payload: &InboundCommandPayload) -> ProductCommand {
    let args = payload.arguments.trim();
    let Ok(json) = serde_json::from_str::<Value>(args) else {
        return invalid_lifecycle_command(payload, "skill_install expects a JSON payload");
    };
    let content = match json.get("content").and_then(Value::as_str) {
        Some(content) => content,
        None => return invalid_lifecycle_command(payload, "skill_install.content is required"),
    };
    let content = match validate_lifecycle_text(content.to_string(), "skill content", 64 * 1024) {
        Ok(content) => content,
        Err(error) => return invalid_lifecycle_command(payload, error.to_string()),
    };
    let name = match json.get("name").and_then(Value::as_str) {
        Some(name) => match LifecyclePackageId::new(name) {
            Ok(name) => Some(name),
            Err(error) => return invalid_lifecycle_command(payload, error.to_string()),
        },
        None => None,
    };
    ProductCommand::Lifecycle {
        action: LifecycleProductAction::SkillInstall { name, content },
    }
}

fn parse_skill_remove_command(payload: &InboundCommandPayload) -> ProductCommand {
    let args = payload.arguments.trim();
    let id = match skill_remove_ref_argument(args) {
        Ok(id) => id,
        Err(reason) => return invalid_lifecycle_command(payload, reason),
    };
    match lifecycle_package_ref(LifecyclePackageKind::Skill, id) {
        Ok(package_ref) => ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillRemove { package_ref },
        },
        Err(error) => invalid_lifecycle_command(payload, error.to_string()),
    }
}

fn extension_package_command(
    payload: &InboundCommandPayload,
    build: fn(LifecyclePackageRef) -> LifecycleProductAction,
) -> ProductCommand {
    let id = match lifecycle_ref_argument(payload) {
        Ok(id) => id,
        Err(reason) => return invalid_lifecycle_command(payload, reason),
    };
    match lifecycle_package_ref(LifecyclePackageKind::Extension, id) {
        Ok(package_ref) => ProductCommand::Lifecycle {
            action: build(package_ref),
        },
        Err(error) => invalid_lifecycle_command(payload, error.to_string()),
    }
}

fn lifecycle_ref_argument(payload: &InboundCommandPayload) -> Result<String, String> {
    let args = payload.arguments.trim();
    match serde_json::from_str::<Value>(args) {
        Ok(json) => json
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("{}.id is required", payload.command)),
        Err(_) => Ok(first_argument(args).to_string()),
    }
}

fn skill_remove_ref_argument(args: &str) -> Result<String, String> {
    match serde_json::from_str::<Value>(args) {
        Ok(json) => json
            .get("id")
            .or_else(|| json.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "skill_remove.id or skill_remove.name is required".to_string()),
        Err(_) => Ok(first_argument(args).to_string()),
    }
}

fn first_argument(args: &str) -> &str {
    args.split_whitespace().next().unwrap_or("")
}

fn unknown_lifecycle_command(payload: &InboundCommandPayload) -> ProductCommand {
    ProductCommand::Unknown {
        name: payload.command.clone(),
        arguments: payload.arguments.clone(),
    }
}

fn invalid_lifecycle_command(
    payload: &InboundCommandPayload,
    reason: impl Into<String>,
) -> ProductCommand {
    ProductCommand::Invalid {
        name: payload.command.clone(),
        reason: reason.into(),
    }
}

fn lifecycle_package_ref(
    kind: LifecyclePackageKind,
    id: impl Into<String>,
) -> Result<LifecyclePackageRef, ProductWorkflowError> {
    LifecyclePackageRef::new(kind, id)
}
