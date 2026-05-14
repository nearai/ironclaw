use clap::{Args, Subcommand};

const STATUS_NOT_WIRED: &str = "not-wired";
const V1_STATE_NOT_USED: &str = "not-used";
const DETAILS: [&str; 2] = [
    "Reborn skill catalog is not wired yet",
    "v1 skill discovery is intentionally not read",
];

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
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self.command {
            SkillsSubcommand::List(command) => command.execute(),
        }
    }
}

impl SkillsListCommand {
    fn execute(self) -> anyhow::Result<()> {
        let details = self.verbose.then_some(DETAILS.as_slice());

        if self.json {
            let mut output = serde_json::json!({
                "configured": 0,
                "skills": [],
                "status": STATUS_NOT_WIRED,
                "v1_state": V1_STATE_NOT_USED,
            });
            if let Some(details) = details {
                output["details"] = serde_json::json!(details);
            }
            println!("{}", output);
            return Ok(());
        }

        println!("IronClaw Reborn skills");
        println!("configured: 0");
        println!("status: {STATUS_NOT_WIRED}");
        println!("v1_state: {V1_STATE_NOT_USED}");

        if let Some(details) = details {
            for detail in details {
                println!("detail: {detail}");
            }
        }

        Ok(())
    }
}
