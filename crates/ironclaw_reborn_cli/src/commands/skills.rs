use clap::{Args, Subcommand};
use ironclaw_reborn_composition::{
    RebornSkillSummary, list_reborn_local_skills, reborn_skill_summary_json,
};
use ironclaw_reborn_config::{RebornBootConfig, RebornProfile};
use std::path::PathBuf;

use crate::context::RebornCliContext;

#[derive(Debug, Args)]
pub(crate) struct SkillsCommand {
    #[command(subcommand)]
    command: SkillsSubcommand,
}

#[derive(Debug, Subcommand)]
enum SkillsSubcommand {
    /// List configured Reborn skills.
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
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        match self.command {
            SkillsSubcommand::List(command) => command.execute(context),
        }
    }
}

impl SkillsListCommand {
    fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let config = build_skill_list_config(context.boot_config())?;
        let skills = crate::runtime::block_on_cli(list_reborn_local_skills(
            config.owner_id.clone(),
            config.local_dev_root.clone(),
        ))?;

        if self.json {
            let mut output = skills_json(&skills);
            if self.verbose {
                output["details"] = serde_json::json!({
                    "profile": config.profile.to_string(),
                    "reborn_home": context.boot_config().home().path(),
                    "local_dev_root": config.local_dev_root,
                    "owner_id": config.owner_id,
                });
            }
            println!("{}", output);
            return Ok(());
        }

        println!("IronClaw Reborn skills");
        println!("configured: {}", skills.len());
        println!("source: reborn-local-dev");

        if self.verbose {
            println!("profile: {}", config.profile);
            println!(
                "reborn_home: {}",
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
    profile: RebornProfile,
}

fn build_skill_list_config(config: &RebornBootConfig) -> anyhow::Result<SkillListConfig> {
    let config_file = crate::runtime::read_config_file(config)?;
    let profile = crate::runtime::effective_profile(config, config_file.as_ref())?;
    match profile {
        RebornProfile::LocalDev
        | RebornProfile::LocalDevYolo
        | RebornProfile::HostedSingleTenant => {}
        RebornProfile::Production | RebornProfile::MigrationDryRun => {
            anyhow::bail!(
                "ironclaw-reborn skills currently supports profile=local-dev, profile=local-dev-yolo, or profile=hosted-single-tenant; got profile={profile}"
            );
        }
    }
    Ok(SkillListConfig {
        owner_id: crate::runtime::default_owner_id(config_file.as_ref()).to_string(),
        local_dev_root: crate::runtime::local_runtime_storage_root(config, profile),
        profile,
    })
}

fn print_skill(skill: &RebornSkillSummary, verbose: bool) {
    println!(
        "- {} ({})",
        terminal_safe_text(&skill.name),
        skill.source.as_str()
    );
    if !skill.description.is_empty() {
        println!("  description: {}", terminal_safe_text(&skill.description));
    }
    if verbose {
        if !skill.version.is_empty() {
            println!("  version: {}", terminal_safe_text(&skill.version));
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
        .map(|value| terminal_safe_text(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if !safe_values.is_empty() {
        println!("  {label}: {}", safe_values.join(", "));
    }
}

fn terminal_safe_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn skills_json(skills: &[RebornSkillSummary]) -> serde_json::Value {
    serde_json::json!({
        "configured": skills.len(),
        "skills": skills.iter().map(reborn_skill_summary_json).collect::<Vec<_>>(),
        "source": "reborn-local-dev",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_safe_text_replaces_control_characters() {
        assert_eq!(
            terminal_safe_text("safe\nforged: row\u{1b}[31m"),
            "safe forged: row [31m"
        );
    }
}
