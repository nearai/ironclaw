//! Reborn-native product command contract.
//!
//! Slash strings are only an edge syntax. This module starts from normalized
//! command payloads so command parsing does not depend on v1 agent routing or on
//! the product surface that produced the command.

use ironclaw_product_adapters::InboundCommandPayload;
use serde::{Deserialize, Serialize};

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

pub fn product_command_descriptors() -> impl Iterator<Item = &'static ProductCommandDescriptor> {
    COMMAND_SPECS.iter().map(|spec| &spec.descriptor)
}

/// Typed command family produced from a normalized command payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ProductCommand {
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
