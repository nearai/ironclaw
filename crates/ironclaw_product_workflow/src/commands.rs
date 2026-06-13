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
        LifecycleCommandKind, LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef,
        LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
        validate_lifecycle_text,
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
    parse: fn(&InboundCommandPayload) -> ProductCommandParseResult,
}

const COMMAND_SPECS: &[ProductCommandSpec] = &[
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
];

type ProductCommandParseResult = Result<ProductCommand, ProductRejection>;

pub fn product_command_descriptors() -> impl Iterator<Item = ProductCommandDescriptor> {
    LifecycleCommandKind::ALL
        .iter()
        .copied()
        .map(|kind| ProductCommandDescriptor {
            name: kind.command_name(),
            aliases: &[],
        })
        .chain(COMMAND_SPECS.iter().map(|spec| spec.descriptor.clone()))
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
    Set {
        model: String,
    },
    SetProvider {
        provider: String,
        model: Option<String>,
    },
}

impl ProductCommand {
    pub fn from_payload(payload: &InboundCommandPayload) -> ProductCommandParseResult {
        if let Some(kind) = LifecycleCommandKind::from_command_name(&payload.command) {
            return parse_lifecycle_command_payload(kind, payload);
        }
        Ok(match command_spec_for_name(&payload.command) {
            Some(spec) => (spec.parse)(payload)?,
            None => ProductCommand::Unknown {
                name: payload.command.clone(),
                arguments: payload.arguments.clone(),
            },
        })
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Lifecycle { action } => action.command_name(),
            Self::Model { .. } => "model",
            Self::Status => "status",
            Self::Unknown { name, .. } => name.as_str(),
        }
    }

    pub fn descriptor(&self) -> Option<ProductCommandDescriptor> {
        product_command_descriptors().find(|descriptor| {
            descriptor.name == self.name() || descriptor.aliases.contains(&self.name())
        })
    }
}

fn command_spec_for_name(name: &str) -> Option<&'static ProductCommandSpec> {
    COMMAND_SPECS
        .iter()
        .find(|spec| spec.descriptor.name == name || spec.descriptor.aliases.contains(&name))
}

fn parse_model_command(payload: &InboundCommandPayload) -> ProductCommandParseResult {
    let mut args = payload.arguments.split_whitespace();
    let Some(first) = args.next() else {
        return Ok(ProductCommand::Model {
            action: ProductModelCommand::Status,
        });
    };
    match ModelCommandHead::parse(first)? {
        ModelCommandHead::SetProvider => {
            let Some(provider) = args.next() else {
                return invalid_lifecycle_command("model set-provider requires a provider id");
            };
            let remaining = args.collect::<Vec<_>>();
            let model = parse_model_option(&remaining)?;
            Ok(ProductCommand::Model {
                action: ProductModelCommand::SetProvider {
                    provider: provider.to_string(),
                    model,
                },
            })
        }
        ModelCommandHead::SetModel(model) => Ok(ProductCommand::Model {
            action: ProductModelCommand::Set {
                model: model.to_string(),
            },
        }),
    }
}

enum ModelCommandHead<'a> {
    SetProvider,
    SetModel(&'a str),
}

impl<'a> ModelCommandHead<'a> {
    fn parse(value: &'a str) -> Result<Self, ProductRejection> {
        match value {
            "set-provider" | "provider" => Ok(Self::SetProvider),
            flag if flag.starts_with('-') => Err(ProductRejection::permanent(
                ProductRejectionKind::InvalidRequest,
                "model set requires a model name; flags are only valid after `set-provider`",
            )),
            model => Ok(Self::SetModel(model)),
        }
    }
}

fn parse_status_command(_payload: &InboundCommandPayload) -> ProductCommandParseResult {
    Ok(ProductCommand::Status)
}

fn parse_model_option(args: &[&str]) -> Result<Option<String>, ProductRejection> {
    if args.is_empty() {
        return Ok(None);
    }
    if args.len() == 2 && args[0] == "--model" {
        return Ok(Some(args[1].to_string()));
    }
    Err(ProductRejection::permanent(
        ProductRejectionKind::InvalidRequest,
        "model set-provider accepts only `--model <model>` after provider",
    ))
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

fn parse_lifecycle_command_payload(
    kind: LifecycleCommandKind,
    payload: &InboundCommandPayload,
) -> ProductCommandParseResult {
    Ok(match kind {
        LifecycleCommandKind::ExtensionSearch => ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionSearch {
                query: payload.arguments.trim().to_string(),
            },
        },
        LifecycleCommandKind::ExtensionList => ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionList,
        },
        LifecycleCommandKind::ExtensionInstall => {
            extension_package_command(payload, |package_ref| {
                LifecycleProductAction::ExtensionInstall { package_ref }
            })?
        }
        LifecycleCommandKind::ExtensionAuth => extension_package_command(payload, |package_ref| {
            LifecycleProductAction::ExtensionAuth { package_ref }
        })?,
        LifecycleCommandKind::ExtensionActivate => {
            extension_package_command(payload, |package_ref| {
                LifecycleProductAction::ExtensionActivate { package_ref }
            })?
        }
        LifecycleCommandKind::ExtensionConfigure => parse_extension_configure_command(payload)?,
        LifecycleCommandKind::ExtensionRemove => {
            extension_package_command(payload, |package_ref| {
                LifecycleProductAction::ExtensionRemove { package_ref }
            })?
        }
        LifecycleCommandKind::SkillSearch => ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillSearch {
                query: payload.arguments.trim().to_string(),
            },
        },
        LifecycleCommandKind::SkillInstall => parse_skill_install_command(payload)?,
        LifecycleCommandKind::SkillRemove => parse_skill_remove_command(payload)?,
    })
}

fn parse_extension_configure_command(payload: &InboundCommandPayload) -> ProductCommandParseResult {
    let args = payload.arguments.trim();
    let (id, config_payload) = match serde_json::from_str::<Value>(args) {
        Ok(json) => {
            let Some(id) = json.get("id").and_then(Value::as_str).map(str::to_string) else {
                return invalid_lifecycle_command("extension_configure.id is required");
            };
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
        Err(error) => invalid_lifecycle_command(error.to_string()),
    }
}

fn parse_skill_install_command(payload: &InboundCommandPayload) -> ProductCommandParseResult {
    let args = payload.arguments.trim();
    let Ok(json) = serde_json::from_str::<Value>(args) else {
        return invalid_lifecycle_command("skill_install expects a JSON payload");
    };
    let content = match json.get("content").and_then(Value::as_str) {
        Some(content) => content,
        None => return invalid_lifecycle_command("skill_install.content is required"),
    };
    let content = match validate_lifecycle_text(content.to_string(), "skill content", 64 * 1024) {
        Ok(content) => content,
        Err(error) => return invalid_lifecycle_command(error.to_string()),
    };
    let name = match json.get("name").and_then(Value::as_str) {
        Some(name) => match LifecyclePackageId::new(name) {
            Ok(name) => Some(name),
            Err(error) => return invalid_lifecycle_command(error.to_string()),
        },
        None => None,
    };
    Ok(ProductCommand::Lifecycle {
        action: LifecycleProductAction::SkillInstall { name, content },
    })
}

fn parse_skill_remove_command(payload: &InboundCommandPayload) -> ProductCommandParseResult {
    let args = payload.arguments.trim();
    let id = match skill_remove_ref_argument(args) {
        Ok(id) => id,
        Err(reason) => return invalid_lifecycle_command(reason),
    };
    match lifecycle_package_ref(LifecyclePackageKind::Skill, id) {
        Ok(package_ref) => Ok(ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillRemove { package_ref },
        }),
        Err(error) => invalid_lifecycle_command(error.to_string()),
    }
}

fn extension_package_command(
    payload: &InboundCommandPayload,
    build: fn(LifecyclePackageRef) -> LifecycleProductAction,
) -> ProductCommandParseResult {
    let id = match lifecycle_ref_argument(payload) {
        Ok(id) => id,
        Err(reason) => return invalid_lifecycle_command(reason),
    };
    match lifecycle_package_ref(LifecyclePackageKind::Extension, id) {
        Ok(package_ref) => Ok(ProductCommand::Lifecycle {
            action: build(package_ref),
        }),
        Err(error) => invalid_lifecycle_command(error.to_string()),
    }
}

fn lifecycle_ref_argument(payload: &InboundCommandPayload) -> Result<String, String> {
    let args = payload.arguments.trim();
    json_or_whitespace_field(args, &["id"], || {
        format!("{}.id is required", payload.command)
    })
}

fn skill_remove_ref_argument(args: &str) -> Result<String, String> {
    json_or_whitespace_field(args, &["id", "name"], || {
        "skill_remove.id or skill_remove.name is required".to_string()
    })
}

fn json_or_whitespace_field(
    args: &str,
    keys: &[&str],
    missing_message: impl FnOnce() -> String,
) -> Result<String, String> {
    match serde_json::from_str::<Value>(args) {
        Ok(json) => keys
            .iter()
            .find_map(|key| json.get(*key).and_then(Value::as_str))
            .map(str::to_string)
            .ok_or_else(missing_message),
        Err(_) => Ok(first_argument(args).to_string()),
    }
}

fn first_argument(args: &str) -> &str {
    args.split_whitespace().next().unwrap_or("")
}

fn invalid_lifecycle_command(reason: impl Into<String>) -> ProductCommandParseResult {
    Err(ProductRejection::permanent(
        ProductRejectionKind::InvalidRequest,
        reason.into(),
    ))
}

fn lifecycle_package_ref(
    kind: LifecyclePackageKind,
    id: impl Into<String>,
) -> Result<LifecyclePackageRef, ProductWorkflowError> {
    LifecyclePackageRef::new(kind, id)
}
