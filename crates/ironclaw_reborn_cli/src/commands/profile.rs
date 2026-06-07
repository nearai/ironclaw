use clap::{Args, Subcommand};
use ironclaw_reborn_config::{REBORN_PROFILE_ENV, RebornProfile};

#[derive(Debug, Args)]
pub(crate) struct ProfileCommand {
    #[command(subcommand)]
    command: ProfileSubcommand,
}

#[derive(Debug, Subcommand)]
enum ProfileSubcommand {
    /// List supported Reborn boot profiles.
    List(ProfileListCommand),
}

#[derive(Debug, Args)]
struct ProfileListCommand {
    /// Output profiles as JSON.
    #[arg(long)]
    json: bool,
}

impl ProfileCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self.command {
            ProfileSubcommand::List(command) => command.execute(),
        }
    }
}

impl ProfileListCommand {
    fn execute(self) -> anyhow::Result<()> {
        let profiles = RebornProfile::all();

        if self.json {
            let profiles = profiles.iter().map(|profile| {
                serde_json::json!({
                    "name": profile.as_str(),
                    "default": *profile == RebornProfile::default(),
                })
            });
            println!(
                "{}",
                serde_json::json!({
                    "profiles": profiles.collect::<Vec<_>>(),
                    "selector": REBORN_PROFILE_ENV,
                })
            );
        } else {
            println!("IronClaw Reborn profiles");
            for profile in profiles {
                if *profile == RebornProfile::default() {
                    println!("- {} (default)", profile);
                } else {
                    println!("- {}", profile);
                }
            }
            println!("Select with {}=<profile>", REBORN_PROFILE_ENV);
        }

        Ok(())
    }
}
