use clap::{Args, Subcommand};
use ironclaw_composition::{
    IronClawSkillSummary, ironclaw_skill_summary_json, list_ironclaw_local_skills,
};
use ironclaw_config::{IronClawBootConfig, IronClawProfile};
use std::path::PathBuf;

use crate::context::IronClawCliContext;

#[derive(Debug, Args)]
pub(crate) struct SkillsCommand {
    #[command(subcommand)]
    command: SkillsSubcommand,
}

#[derive(Debug, Subcommand)]
enum SkillsSubcommand {
    /// List configured IronClaw skills.
    List(SkillsListCommand),
}

#[derive(Debug, Args)]
struct SkillsListCommand {
    /// Show extra status details.
    #[arg(short, long)]
    verbose: bool,

    /// Output skills as JSON.
    #[arg(long)]
    json: bool,
}

impl SkillsCommand {
    pub(crate) fn execute(self, context: IronClawCliContext) -> anyhow::Result<()> {
        match self.command {
            SkillsSubcommand::List(command) => command.execute(context),
        }
    }
}

impl SkillsListCommand {
    fn execute(self, context: IronClawCliContext) -> anyhow::Result<()> {
        let config = build_skill_list_config(context.boot_config())?;
        let skills = crate::runtime::block_on_cli(list_ironclaw_local_skills(
            config.owner_id.clone(),
            config.local_dev_root.clone(),
        ))?;

        if self.json {
            let mut output = skills_json(&skills);
            if self.verbose {
                output["details"] = serde_json::json!({
                    "profile": config.profile.to_string(),
                    "ironclaw_home": context.boot_config().home().path(),
                    "local_dev_root": config.local_dev_root,
                    "owner_id": config.owner_id,
                });
            }
            println!("{}", output);
            return Ok(());
        }

        println!("IronClaw skills");
        println!("configured: {}", skills.len());
        println!("source: ironclaw-local");

        if self.verbose {
            println!("profile: {}", config.profile);
            println!(
                "ironclaw_home: {}",
                context.boot_config().home().path().display()
            );
            println!("local_dev_root: {}", config.local_dev_root.display());
            println!("owner_id: {}", config.owner_id);
        }

        for skill in skills {
            print_skill(&skill, self.verbose);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillListConfig {
    owner_id: String,
    local_dev_root: PathBuf,
    profile: IronClawProfile,
}

fn build_skill_list_config(config: &IronClawBootConfig) -> anyhow::Result<SkillListConfig> {
    let config_file = crate::runtime::read_config_file(config)?;
    let profile = crate::runtime::effective_profile(config, config_file.as_ref())?;
    if !profile.supports_local_runtime_skill_management() {
        anyhow::bail!(
            "ironclaw skills currently supports profile=local-dev, profile=local-dev-yolo, profile=hosted-single-tenant, or profile=hosted-single-tenant-volume; got profile={profile}"
        );
    }
    Ok(SkillListConfig {
        owner_id: crate::runtime::default_owner_id(config_file.as_ref()).to_string(),
        local_dev_root: crate::runtime::local_runtime_storage_root(config, profile),
        profile,
    })
}

fn print_skill(skill: &IronClawSkillSummary, verbose: bool) {
    println!(
        "- {} ({})",
        crate::render::terminal_safe_text(&skill.name),
        skill.source.as_str()
    );
    if !skill.description.is_empty() {
        println!(
            "  description: {}",
            crate::render::terminal_safe_text(&skill.description)
        );
    }
    if verbose {
        if !skill.version.is_empty() {
            println!(
                "  version: {}",
                crate::render::terminal_safe_text(&skill.version)
            );
        }
        print_list_field("keywords", &skill.keywords);
        print_list_field("tags", &skill.tags);
        print_list_field("requires_skills", &skill.requires_skills);
    }
}

fn print_list_field(label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let safe_values = values
        .iter()
        .map(|value| crate::render::terminal_safe_text(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if !safe_values.is_empty() {
        println!("  {label}: {}", safe_values.join(", "));
    }
}

fn skills_json(skills: &[IronClawSkillSummary]) -> serde_json::Value {
    serde_json::json!({
        "configured": skills.len(),
        "skills": skills.iter().map(ironclaw_skill_summary_json).collect::<Vec<_>>(),
        "source": "ironclaw-local",
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn terminal_safe_text_replaces_control_characters() {
        assert_eq!(
            crate::render::terminal_safe_text("safe\nforged: row\u{1b}[31m"),
            "safe forged: row [31m"
        );
    }
}
