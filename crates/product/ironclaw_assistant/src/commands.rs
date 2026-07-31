//! Reborn-native product command contract.
//!
//! Slash strings are only an edge syntax. This module starts from normalized
//! command payloads so command parsing does not depend on v1 agent routing or on
//! the product surface that produced the command.

use ironclaw_extension_contracts::hosted_mcp::RegisterHostedMcpRequest;
use ironclaw_host_api::error::HostApiError;
use ironclaw_product_contracts::inbound::{
    InboundCommandPayload, ProductRejection, ProductRejectionKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

use crate::lifecycle::{
    LifecycleCommandKind, LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef,
    LifecycleProductAction,
};

pub const PRODUCT_LIFECYCLE_COMMAND_OPERATION_ID: &str = "product.lifecycle.command";
pub const PRODUCT_MODEL_COMMAND_OPERATION_ID: &str = "product.model.command";
pub const PRODUCT_NEW_COMMAND_OPERATION_ID: &str = "product.new.command";
pub const PRODUCT_STATUS_COMMAND_OPERATION_ID: &str = "product.status.command";
pub const PRODUCT_STOP_COMMAND_OPERATION_ID: &str = "product.stop.command";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandAudience {
    User,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductLifecycleCommandInput {
    pub action: LifecycleProductAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductModelCommandInput {
    pub action: ProductModelCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductStatusCommandInput {
    /// Filled from the resolved conversation binding, never external input.
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductNewCommandInput {
    /// Filled from the resolved conversation binding, never external input.
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductNewCommandOutput {
    /// False when the bound thread still has a non-terminal run. Channel
    /// workflows must leave the binding untouched in that case.
    pub can_reset: bool,
    pub result: CommandResultView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductStopCommandInput {
    /// Filled from the resolved conversation binding, never external input.
    pub thread_id: String,
    pub invocation: ProductStopInvocation,
}

/// Channel-neutral presentational result for product commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResultView {
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<CommandResultField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResultField {
    pub label: String,
    pub value: String,
}

/// Public command inventory metadata. Policy decisions based on actor,
/// installation, trigger, or product surface belong to `ProductCommandAdmissionService`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductCommandDescriptor {
    pub name: &'static str,
    pub audience: CommandAudience,
    pub title: &'static str,
    pub description: &'static str,
    pub usage: &'static str,
}

struct ProductCommandSpec {
    descriptor: ProductCommandDescriptor,
    parse: fn(&InboundCommandPayload) -> ProductCommandParseResult,
}

const COMMAND_SPECS: &[ProductCommandSpec] = &[
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "model",
            audience: CommandAudience::User,
            title: "Model",
            description: "Show or switch the active LLM provider and model",
            usage: "/model [<model> | set-provider <provider> [--model <model>]]",
        },
        parse: parse_model_command,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "status",
            audience: CommandAudience::User,
            title: "Status",
            description: "Show what the assistant is doing in this conversation",
            usage: "/status",
        },
        parse: parse_status_command,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "new",
            audience: CommandAudience::User,
            title: "New conversation",
            description: "Start a fresh conversation without deleting the current one",
            usage: "/new",
        },
        parse: parse_new_command,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "stop",
            audience: CommandAudience::User,
            title: "Stop",
            description: "Stop the active run in this conversation",
            usage: "/stop",
        },
        parse: parse_stop_command,
    },
    ProductCommandSpec {
        descriptor: ProductCommandDescriptor {
            name: "interrupt",
            audience: CommandAudience::User,
            title: "Interrupt",
            description: "Interrupt the active run in this conversation",
            usage: "/interrupt",
        },
        parse: parse_interrupt_command,
    },
];

type ProductCommandParseResult = Result<ProductCommand, ProductRejection>;

pub fn product_command_descriptors() -> impl Iterator<Item = ProductCommandDescriptor> {
    LifecycleCommandKind::ALL
        .iter()
        .copied()
        .map(|kind| {
            let (title, description, usage) = lifecycle_command_metadata(kind);
            ProductCommandDescriptor {
                name: kind.command_name(),
                audience: CommandAudience::Admin,
                title,
                description,
                usage,
            }
        })
        .chain(COMMAND_SPECS.iter().map(|spec| spec.descriptor.clone()))
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown product command `{name}`")]
pub struct UnknownProductCommandName {
    name: String,
}

pub fn validate_declared_product_command(name: &str) -> Result<(), UnknownProductCommandName> {
    if product_command_descriptors().any(|descriptor| descriptor.name == name) {
        return Ok(());
    }
    Err(UnknownProductCommandName {
        name: name.to_string(),
    })
}

pub fn declared_command_help_text<I, S>(commands: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    declared_command_help_text_with_prefix(commands, None)
}

/// Same rendering as [`declared_command_help_text`], but renders each name
/// as `{prefix}{name}` instead of the bare `/{name}` form when `prefix` is
/// set — the manifest-declared `[channel.presentation].command_prefix` a
/// channel adapter whose native command namespace requires an app-scoped
/// dispatcher prefix (e.g. a `/ironclaw` slash dispatcher) uses to
/// namespace its commands. `prefix` is rendered exactly as declared,
/// including any trailing separator (a manifest `command_prefix` of
/// `"/ironclaw "` plus `model` yields `/ironclaw model`).
pub fn declared_command_help_text_with_prefix<I, S>(commands: I, prefix: Option<&str>) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let names = commands
        .into_iter()
        .map(|command| command.as_ref().to_string())
        .collect::<BTreeSet<_>>();
    if names.is_empty() {
        return "Commands are not available in this channel.".to_string();
    }
    let names = names
        .into_iter()
        .map(|name| match prefix {
            Some(prefix) => format!("{prefix}{name}"),
            None => format!("/{name}"),
        })
        .collect::<Vec<_>>();
    format!("Available commands:\n{}", names.join("\n"))
}

pub fn render_command_result_text(view: &CommandResultView) -> String {
    let mut text = view.title.clone();
    for field in &view.fields {
        text.push_str(&format!("\n{}: {}", field.label, field.value));
    }
    for line in &view.lines {
        text.push('\n');
        text.push_str(line);
    }
    text
}

/// Typed command family produced from a normalized command payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ProductCommand {
    Lifecycle { action: LifecycleProductAction },
    Model { action: ProductModelCommand },
    New,
    Status,
    Stop { invocation: ProductStopInvocation },
    Unknown { name: String, arguments: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductStopInvocation {
    Stop,
    Interrupt,
}

impl ProductStopInvocation {
    pub fn command_name(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Interrupt => "interrupt",
        }
    }
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
            Self::New => "new",
            Self::Status => "status",
            Self::Stop { invocation } => invocation.command_name(),
            Self::Unknown { name, .. } => name.as_str(),
        }
    }

    pub fn descriptor(&self) -> Option<ProductCommandDescriptor> {
        product_command_descriptors().find(|descriptor| descriptor.name == self.name())
    }
}

/// Execution audience, action-aware: `/model` bare is a user-safe read while
/// its `set`/`set-provider` actions mutate operator-wide LLM configuration.
/// `Unknown` is `User` — it never executes (admission rejects undeclared
/// tokens before the audience step) and must not hide behind the admin gate.
pub fn required_audience(command: &ProductCommand) -> CommandAudience {
    match command {
        ProductCommand::Model {
            action: ProductModelCommand::Status,
        } => CommandAudience::User,
        ProductCommand::Model { .. } => CommandAudience::Admin,
        ProductCommand::New => CommandAudience::User,
        ProductCommand::Status => CommandAudience::User,
        ProductCommand::Stop { .. } => CommandAudience::User,
        ProductCommand::Lifecycle { .. } => CommandAudience::Admin,
        ProductCommand::Unknown { .. } => CommandAudience::User,
    }
}

fn command_spec_for_name(name: &str) -> Option<&'static ProductCommandSpec> {
    COMMAND_SPECS
        .iter()
        .find(|spec| spec.descriptor.name == name)
}

fn parse_model_command(payload: &InboundCommandPayload) -> ProductCommandParseResult {
    let mut args = payload.arguments.split_whitespace();
    let Some(first) = args.next() else {
        return Ok(ProductCommand::Model {
            action: ProductModelCommand::Status,
        });
    };
    if first == "set" {
        let Some(model) = args.next() else {
            return invalid_lifecycle_command("model set requires a model name");
        };
        if model.starts_with('-') {
            return invalid_lifecycle_command(
                "model set requires a model name; flags are only valid after `set-provider`",
            );
        }
        if args.next().is_some() {
            return invalid_lifecycle_command("model set accepts only a model name");
        }
        return Ok(ProductCommand::Model {
            action: ProductModelCommand::Set {
                model: model.to_string(),
            },
        });
    }
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

fn parse_new_command(_payload: &InboundCommandPayload) -> ProductCommandParseResult {
    Ok(ProductCommand::New)
}

fn parse_stop_command(_payload: &InboundCommandPayload) -> ProductCommandParseResult {
    Ok(ProductCommand::Stop {
        invocation: ProductStopInvocation::Stop,
    })
}

fn parse_interrupt_command(_payload: &InboundCommandPayload) -> ProductCommandParseResult {
    Ok(ProductCommand::Stop {
        invocation: ProductStopInvocation::Interrupt,
    })
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

fn parse_lifecycle_command_payload(
    kind: LifecycleCommandKind,
    payload: &InboundCommandPayload,
) -> ProductCommandParseResult {
    Ok(match kind {
        LifecycleCommandKind::ExtensionRegisterHostedMcp => {
            parse_register_hosted_mcp_command(payload)?
        }
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

fn parse_register_hosted_mcp_command(payload: &InboundCommandPayload) -> ProductCommandParseResult {
    let request = serde_json::from_str::<RegisterHostedMcpRequest>(payload.arguments.trim())
        .map_err(|error| {
            tracing::debug!(
                error = %error,
                "hosted MCP registration payload failed to parse"
            );
            ProductRejection::permanent(
                ProductRejectionKind::InvalidRequest,
                "extension_register_hosted_mcp expects a registration JSON payload",
            )
        })?;
    Ok(ProductCommand::Lifecycle {
        action: LifecycleProductAction::ExtensionRegisterHostedMcp { request },
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
    let content = validate_lifecycle_text(content.to_string(), "skill content", 64 * 1024)?;
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
    json_or_whitespace_field(args, &["id", "extension_id"], || {
        format!(
            "{}.id or {}.extension_id is required",
            payload.command, payload.command
        )
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

fn validate_lifecycle_text(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, ProductRejection> {
    if value.trim().is_empty() {
        return invalid_lifecycle_rejection(format!("{label} must not be empty"));
    }
    if value.len() > max_bytes {
        return invalid_lifecycle_rejection(format!("{label} must be at most {max_bytes} bytes"));
    }
    if value.chars().any(|c| c == '\0') {
        return invalid_lifecycle_rejection(format!("{label} must not contain NUL characters"));
    }
    Ok(value)
}

fn invalid_lifecycle_rejection(reason: impl Into<String>) -> Result<String, ProductRejection> {
    Err(ProductRejection::permanent(
        ProductRejectionKind::InvalidRequest,
        reason,
    ))
}

fn lifecycle_command_metadata(
    kind: LifecycleCommandKind,
) -> (&'static str, &'static str, &'static str) {
    match kind {
        LifecycleCommandKind::ExtensionRegisterHostedMcp => (
            "Register hosted MCP",
            "Register a hosted MCP endpoint",
            "/extension_register_hosted_mcp <json>",
        ),
        LifecycleCommandKind::ExtensionSearch => (
            "Search extensions",
            "Search the extension registry",
            "/extension_search <query>",
        ),
        LifecycleCommandKind::ExtensionList => (
            "List extensions",
            "List installed extensions",
            "/extension_list",
        ),
        LifecycleCommandKind::ExtensionInstall => (
            "Install extension",
            "Install an extension by id",
            "/extension_install <id>",
        ),
        LifecycleCommandKind::ExtensionAuth => (
            "Connect extension account",
            "Start authentication for an installed extension",
            "/extension_auth <id>",
        ),
        LifecycleCommandKind::ExtensionActivate => (
            "Activate extension",
            "Activate an installed extension",
            "/extension_activate <id>",
        ),
        LifecycleCommandKind::ExtensionConfigure => (
            "Configure extension",
            "Update an installed extension's configuration values",
            "/extension_configure <id> <json>",
        ),
        LifecycleCommandKind::ExtensionRemove => (
            "Remove extension",
            "Remove an installed extension",
            "/extension_remove <id>",
        ),
        LifecycleCommandKind::SkillSearch => (
            "Search skills",
            "Search the skill registry",
            "/skill_search <query>",
        ),
        LifecycleCommandKind::SkillInstall => (
            "Install skill",
            "Install a skill from JSON content",
            "/skill_install <json>",
        ),
        LifecycleCommandKind::SkillRemove => (
            "Remove skill",
            "Remove an installed skill",
            "/skill_remove <id or name>",
        ),
    }
}

fn lifecycle_package_ref(
    kind: LifecyclePackageKind,
    id: impl Into<String>,
) -> Result<LifecyclePackageRef, HostApiError> {
    LifecyclePackageRef::new(kind, id)
}
