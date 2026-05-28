use clap::{Args, Subcommand};

#[cfg(feature = "root-llm-provider")]
use crate::context::RebornCliContext;

#[derive(Debug, Args)]
pub(crate) struct ModelsCommand {
    #[command(subcommand)]
    command: ModelsSubcommand,
}

#[derive(Debug, Subcommand)]
enum ModelsSubcommand {
    /// List Reborn LLM providers, or show one provider.
    List(ModelsListCommand),
    /// Show Reborn model route status.
    Status(ModelsStatusCommand),
    /// Set the default Reborn model for the active provider.
    Set(ModelsSetCommand),
    /// Set the default Reborn LLM provider.
    SetProvider(ModelsSetProviderCommand),
}

#[derive(Debug, Args)]
struct ModelsListCommand {
    /// Show only a specific provider by id or alias.
    provider: Option<String>,
    /// Show provider protocol and credential metadata.
    #[arg(short, long)]
    verbose: bool,
    /// Output providers as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ModelsStatusCommand {
    /// Output model status as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ModelsSetCommand {
    /// Model name (for example, gpt-5-mini or claude-sonnet-4-6-20250514).
    model: String,
}

#[derive(Debug, Args)]
struct ModelsSetProviderCommand {
    /// Provider id or alias (for example, openai, anthropic, ollama).
    provider: String,
    /// Also set the model. Defaults to the provider's catalog default.
    #[arg(long)]
    model: Option<String>,
}

impl ModelsCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self.command {
            ModelsSubcommand::List(command) => command.execute(),
            ModelsSubcommand::Status(command) => command.execute(),
            ModelsSubcommand::Set(command) => command.execute(),
            ModelsSubcommand::SetProvider(command) => command.execute(),
        }
    }
}

#[cfg(feature = "root-llm-provider")]
impl ModelsListCommand {
    fn execute(self) -> anyhow::Result<()> {
        let context = RebornCliContext::resolve_from_env()?;
        let admin =
            ironclaw_reborn_composition::RebornProviderAdmin::new(context.boot_config().clone());
        let list = admin.list(
            self.provider.as_deref(),
            self.verbose || self.provider.is_some(),
        )?;
        if self.json {
            println!("{}", serde_json::to_string_pretty(&list)?);
            return Ok(());
        }
        if self.provider.is_some() {
            print_provider_detail(&list);
        } else {
            print_provider_list(&list, self.verbose);
        }
        Ok(())
    }
}

#[cfg(not(feature = "root-llm-provider"))]
impl ModelsListCommand {
    fn execute(self) -> anyhow::Result<()> {
        let slots = ironclaw_reborn_composition::reborn_model_slot_names();

        if self.json {
            let slots = slots
                .iter()
                .map(|slot| serde_json::json!({ "slot": slot }))
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::json!({
                    "slots": slots,
                    "routes": "not-configured",
                    "v1_state": "not-used",
                })
            );
            return Ok(());
        }

        println!("IronClaw Reborn model slots");
        for slot in slots {
            println!("- {}", slot);
        }
        println!("routes: not-configured");
        println!("v1_state: not-used");
        Ok(())
    }
}

#[cfg(feature = "root-llm-provider")]
impl ModelsStatusCommand {
    fn execute(self) -> anyhow::Result<()> {
        let context = RebornCliContext::resolve_from_env()?;
        let admin =
            ironclaw_reborn_composition::RebornProviderAdmin::new(context.boot_config().clone());
        let status = admin.status()?;

        if self.json {
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(());
        }
        print_status(&status);
        Ok(())
    }
}

#[cfg(feature = "root-llm-provider")]
impl ModelsSetCommand {
    fn execute(self) -> anyhow::Result<()> {
        let context = RebornCliContext::resolve_from_env()?;
        let admin =
            ironclaw_reborn_composition::RebornProviderAdmin::new(context.boot_config().clone());
        let outcome = admin.set_model(&self.model)?;
        print_write_outcome(WriteOutcomeKind::Model, &outcome);
        Ok(())
    }
}

#[cfg(not(feature = "root-llm-provider"))]
impl ModelsSetCommand {
    fn execute(self) -> anyhow::Result<()> {
        Err(feature_not_available(&format!("set {}", self.model)))
    }
}

#[cfg(feature = "root-llm-provider")]
impl ModelsSetProviderCommand {
    fn execute(self) -> anyhow::Result<()> {
        let context = RebornCliContext::resolve_from_env()?;
        let admin =
            ironclaw_reborn_composition::RebornProviderAdmin::new(context.boot_config().clone());
        let outcome = admin.set_provider(&self.provider, self.model.as_deref())?;
        print_write_outcome(WriteOutcomeKind::Provider, &outcome);
        Ok(())
    }
}

#[cfg(not(feature = "root-llm-provider"))]
impl ModelsSetProviderCommand {
    fn execute(self) -> anyhow::Result<()> {
        Err(feature_not_available(&format!(
            "set-provider {}",
            self.provider
        )))
    }
}

#[cfg(not(feature = "root-llm-provider"))]
fn feature_not_available(command: &str) -> anyhow::Error {
    anyhow::anyhow!("`models {command}` requires the root-llm-provider feature; v1_state: not-used")
}

#[cfg(not(feature = "root-llm-provider"))]
impl ModelsStatusCommand {
    fn execute(self) -> anyhow::Result<()> {
        let slots = ironclaw_reborn_composition::reborn_model_slot_names();

        if self.json {
            let slot_status: serde_json::Map<String, serde_json::Value> = slots
                .iter()
                .map(|slot| {
                    (
                        (*slot).to_string(),
                        serde_json::Value::from("not-configured"),
                    )
                })
                .collect();
            println!(
                "{}",
                serde_json::json!({
                    "routes": "not-configured",
                    "slots": slot_status,
                    "v1_state": "not-used",
                })
            );
            return Ok(());
        }

        println!("IronClaw Reborn model status");
        println!("routes: not-configured");
        for slot in slots {
            println!("{}: not-configured", slot);
        }
        println!("v1_state: not-used");
        Ok(())
    }
}

#[cfg(feature = "root-llm-provider")]
fn print_provider_list(list: &ironclaw_reborn_composition::RebornProviderList, verbose: bool) {
    println!("IronClaw Reborn LLM providers");
    println!("config_file: {}", list.config_file.display());
    println!("providers_file: {}", list.providers_file.display());
    match list.providers.iter().find(|provider| provider.active) {
        Some(provider) => println!(
            "active: {} ({})",
            provider.id,
            provider.active_model.as_deref().unwrap_or("default model")
        ),
        None => println!("active: not-configured"),
    }
    println!();

    for provider in &list.providers {
        let marker = if provider.active { " *" } else { "" };
        if verbose {
            println!("{}{}", provider.id, marker);
            println!("  description: {}", provider.description);
            println!("  default_model: {}", provider.default_model);
            let Some(metadata) = provider.metadata.as_ref() else {
                println!();
                continue;
            };
            println!("  protocol: {}", metadata.protocol);
            println!("  model_env: {}", metadata.model_env);
            if let Some(env) = metadata.api_key_env.as_deref() {
                println!(
                    "  api_key_env: {} ({})",
                    env,
                    if metadata.api_key_required {
                        "required"
                    } else {
                        "optional"
                    }
                );
            }
            if let Some(base_url) = metadata.base_url.as_deref() {
                println!("  base_url: {base_url}");
            }
            if let Some(kind) = metadata.credential_kind {
                println!("  credential_kind: {kind}");
            }
            println!("  can_list_models: {}", metadata.can_list_models);
            println!();
        } else {
            println!(
                "{:<24} {:<36} {}",
                format!("{}{marker}", provider.id),
                provider
                    .active_model
                    .as_deref()
                    .unwrap_or(&provider.default_model),
                provider.description
            );
        }
    }
    println!();
    println!("* = active provider. v1_state: {}", list.v1_state);
}

#[cfg(feature = "root-llm-provider")]
fn print_provider_detail(list: &ironclaw_reborn_composition::RebornProviderList) {
    let Some(provider) = list.providers.first() else {
        return;
    };
    println!("Provider: {}", provider.id);
    println!("Description: {}", provider.description);
    println!("Default model: {}", provider.default_model);
    println!("Active: {}", if provider.active { "yes" } else { "no" });
    let Some(metadata) = provider.metadata.as_ref() else {
        println!("Provider catalog: {}", list.providers_file.display());
        println!("v1_state: {}", list.v1_state);
        return;
    };
    println!("Protocol: {}", metadata.protocol);
    println!("Model env: {}", metadata.model_env);
    if let Some(api_key_env) = metadata.api_key_env.as_deref() {
        println!(
            "API key env: {} ({})",
            api_key_env,
            if metadata.api_key_required {
                "required"
            } else {
                "optional"
            }
        );
    }
    if let Some(base_url) = metadata.base_url.as_deref() {
        println!("Base URL: {base_url}");
    }
    if let Some(kind) = metadata.credential_kind {
        println!("Credential kind: {kind}");
    }
    println!("Can list models: {}", metadata.can_list_models);
    println!("Provider catalog: {}", list.providers_file.display());
    println!("v1_state: {}", list.v1_state);
}

#[cfg(feature = "root-llm-provider")]
fn print_status(status: &ironclaw_reborn_composition::RebornProviderStatus) {
    println!("IronClaw Reborn model status");
    println!("config_file: {}", status.config_file.display());
    println!("providers_file: {}", status.providers_file.display());
    match status.default.as_ref() {
        Some(selection) => {
            println!(
                "default.provider: {}",
                selection.provider_id.as_deref().unwrap_or("not-configured")
            );
            println!(
                "default.provider_known: {}",
                if selection.provider_known {
                    "yes"
                } else {
                    "no"
                }
            );
            println!(
                "default.model: {}",
                selection.model.as_deref().unwrap_or("provider default")
            );
            if let Some(api_key_env) = selection.api_key_env.as_deref() {
                println!("default.api_key_env: {api_key_env}");
            }
            if let Some(base_url) = selection.base_url.as_deref() {
                println!("default.base_url: {base_url}");
            }
        }
        None => println!("routes: {}", status.routes),
    }
    println!("v1_state: {}", status.v1_state);
}

#[cfg(feature = "root-llm-provider")]
#[derive(Debug, Clone, Copy)]
enum WriteOutcomeKind {
    Model,
    Provider,
}

#[cfg(feature = "root-llm-provider")]
fn print_write_outcome(
    kind: WriteOutcomeKind,
    outcome: &ironclaw_reborn_composition::RebornProviderWriteOutcome,
) {
    match kind {
        WriteOutcomeKind::Model => {
            println!(
                "Model set to `{}` for provider `{}`",
                outcome.model, outcome.provider_id
            );
        }
        WriteOutcomeKind::Provider => {
            println!(
                "Provider set to `{}`, model set to `{}`",
                outcome.provider_id, outcome.model
            );
        }
    }
    println!("Saved to {}", outcome.config_file.display());
    if outcome.missing_api_key
        && let Some(api_key_env) = outcome.api_key_env.as_deref()
    {
        println!(
            "Note: `{}` requires credentials. Set {api_key_env} before running with this provider.",
            outcome.provider_id
        );
    }
    println!("v1_state: {}", outcome.v1_state);
}
