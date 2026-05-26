//! Reborn-native product command contract.
//!
//! Slash strings are only an edge syntax. This module starts from normalized
//! command payloads so command parsing does not depend on v1 agent routing or on
//! the product surface that produced the command.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    InboundCommandPayload, ProductInboundAck, ProductRejection, ProductRejectionKind,
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
    parse: fn(&InboundCommandPayload) -> Result<ProductCommand, ProductWorkflowError>,
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
    Unknown { name: String, arguments: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProductModelCommand {
    Status,
    Set { model: String },
}

impl ProductCommand {
    pub fn from_payload(payload: &InboundCommandPayload) -> Result<Self, ProductWorkflowError> {
        match command_spec_for_name(&payload.command) {
            Some(spec) => (spec.parse)(payload),
            None => Ok(Self::Unknown {
                name: payload.command.clone(),
                arguments: payload.arguments.clone(),
            }),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Lifecycle { action } => action.command_name(),
            Self::Model { .. } => "model",
            Self::Status => "status",
            Self::Unknown { name, .. } => name.as_str(),
        }
    }

    pub fn descriptor(&self) -> Option<&'static ProductCommandDescriptor> {
        command_spec_for_name(self.name()).map(|spec| &spec.descriptor)
    }
}

fn command_spec_for_name(name: &str) -> Option<&'static ProductCommandSpec> {
    COMMAND_SPECS
        .iter()
        .find(|spec| spec.descriptor.name == name || spec.descriptor.aliases.contains(&name))
}

fn parse_model_command(
    payload: &InboundCommandPayload,
) -> Result<ProductCommand, ProductWorkflowError> {
    let model = payload.arguments.split_whitespace().next();
    Ok(match model {
        Some(model) => ProductCommand::Model {
            action: ProductModelCommand::Set {
                model: model.to_string(),
            },
        },
        None => ProductCommand::Model {
            action: ProductModelCommand::Status,
        },
    })
}

fn parse_status_command(
    _payload: &InboundCommandPayload,
) -> Result<ProductCommand, ProductWorkflowError> {
    Ok(ProductCommand::Status)
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
        // Lifecycle commands are admitted and executed by the facade;
        // the structured response (phase, blockers, payload) belongs to
        // the lifecycle projection stream, not the command ack channel.
        // TODO: once the product surface surfaces lifecycle projections
        // to the caller, wire the response into the projection stream.
        self.facade
            .execute(LifecycleProductContext::Command(context), action)
            .await?;
        Ok(ProductInboundAck::NoOp)
    }
}

fn parse_lifecycle_command_payload(
    payload: &InboundCommandPayload,
) -> Result<ProductCommand, ProductWorkflowError> {
    match payload.command.as_str() {
        "extension_search" => Ok(ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionSearch {
                query: payload.arguments.trim().to_string(),
            },
        }),
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
        "skill_search" => Ok(ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillSearch {
                query: payload.arguments.trim().to_string(),
            },
        }),
        "skill_install" => parse_skill_install_command(payload),
        "skill_remove" => parse_skill_remove_command(payload),
        _ => Ok(unknown_lifecycle_command(payload)),
    }
}

fn parse_extension_configure_command(
    payload: &InboundCommandPayload,
) -> Result<ProductCommand, ProductWorkflowError> {
    let args = payload.arguments.trim();
    let (id, config_payload) = match serde_json::from_str::<Value>(args) {
        Ok(json) => {
            let id = json
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (id, json.get("payload").cloned())
        }
        Err(_) => (first_argument(args).to_string(), None),
    };
    match lifecycle_package_ref(LifecyclePackageKind::Extension, id) {
        Ok(package_ref) => Ok(ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionConfigure {
                package_ref,
                payload: config_payload,
            },
        }),
        Err(error) => Err(error),
    }
}

fn parse_skill_install_command(
    payload: &InboundCommandPayload,
) -> Result<ProductCommand, ProductWorkflowError> {
    let args = payload.arguments.trim();
    let Ok(json) = serde_json::from_str::<Value>(args) else {
        return malformed_command(payload, "expected JSON command arguments");
    };
    let content = match json.get("content").and_then(Value::as_str) {
        Some(content) => content,
        None => return malformed_command(payload, "missing skill content"),
    };
    let content = match validate_lifecycle_text(content.to_string(), "skill content", 64 * 1024) {
        Ok(content) => content,
        Err(error) => return Err(error),
    };
    let name = match json.get("name").and_then(Value::as_str) {
        Some(name) => match LifecyclePackageId::new(name) {
            Ok(name) => Some(name),
            Err(error) => {
                return Err(ProductWorkflowError::InvalidBindingRequest {
                    reason: error.to_string(),
                });
            }
        },
        None => None,
    };
    Ok(ProductCommand::Lifecycle {
        action: LifecycleProductAction::SkillInstall { name, content },
    })
}

fn parse_skill_remove_command(
    payload: &InboundCommandPayload,
) -> Result<ProductCommand, ProductWorkflowError> {
    let args = payload.arguments.trim();
    let id = serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|json| {
            json.get("id")
                .or_else(|| json.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| first_argument(args).to_string());
    match lifecycle_package_ref(LifecyclePackageKind::Skill, id) {
        Ok(package_ref) => Ok(ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillRemove { package_ref },
        }),
        Err(error) => Err(error),
    }
}

fn extension_package_command(
    payload: &InboundCommandPayload,
    build: fn(LifecyclePackageRef) -> LifecycleProductAction,
) -> Result<ProductCommand, ProductWorkflowError> {
    let id = lifecycle_ref_argument(payload);
    match lifecycle_package_ref(LifecyclePackageKind::Extension, id) {
        Ok(package_ref) => Ok(ProductCommand::Lifecycle {
            action: build(package_ref),
        }),
        Err(error) => Err(error),
    }
}

fn lifecycle_ref_argument(payload: &InboundCommandPayload) -> String {
    let args = payload.arguments.trim();
    serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|json| json.get("id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| first_argument(args).to_string())
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

fn malformed_command<T>(
    payload: &InboundCommandPayload,
    reason: impl Into<String>,
) -> Result<T, ProductWorkflowError> {
    Err(ProductWorkflowError::InvalidBindingRequest {
        reason: format!("malformed {} command: {}", payload.command, reason.into()),
    })
}

fn lifecycle_package_ref(
    kind: LifecyclePackageKind,
    id: impl Into<String>,
) -> Result<LifecyclePackageRef, ProductWorkflowError> {
    LifecyclePackageRef::new(kind, id)
}
