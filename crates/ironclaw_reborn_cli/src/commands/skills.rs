use clap::{Args, Subcommand};
use ironclaw_reborn_composition::{RebornSkillListResult, build_reborn_local_skill_catalog};

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
        let config = crate::runtime::build_skill_catalog_config(context.boot_config())?;
        let catalog = build_reborn_local_skill_catalog(&config.owner_id, &config.local_dev_root)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(catalog.list())?;

        if self.json {
            let mut output = skills_json(&result);
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
        println!("configured: {}", result.count);
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

        for skill in result.skills {
            println!("- {} ({})", skill.name, skill.source.as_str());
            if !skill.description.is_empty() {
                println!("  description: {}", skill.description);
            }
        }

        Ok(())
    }
}

fn skills_json(result: &RebornSkillListResult) -> serde_json::Value {
    serde_json::json!({
        "configured": result.count,
        "skills": result.skills.iter().map(|skill| {
            serde_json::json!({
                "name": skill.name,
                "version": skill.version,
                "description": skill.description,
                "source": skill.source.as_str(),
                "keywords": skill.keywords,
                "tags": skill.tags,
                "requires_skills": skill.requires_skills,
            })
        }).collect::<Vec<_>>(),
        "source": "reborn-local-dev",
    })
}
