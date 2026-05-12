use clap::{Args, Subcommand};

const STATUS_NOT_WIRED: &str = "not-wired";
const V1_STATE_NOT_USED: &str = "not-used";
const DETAILS: [&str; 2] = [
    "Reborn hook registry is not wired yet",
    "v1 hook configuration is intentionally not read",
];

#[derive(Debug, Args)]
pub(crate) struct HooksCommand {
    #[command(subcommand)]
    command: HooksSubcommand,
}

#[derive(Debug, Subcommand)]
enum HooksSubcommand {
    /// List configured Reborn hooks.
    List(HooksListCommand),
}

#[derive(Debug, Args)]
struct HooksListCommand {
    /// Show extra status details.
    #[arg(short, long)]
    verbose: bool,

    /// Output hooks as JSON.
    #[arg(long)]
    json: bool,
}

impl HooksCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self.command {
            HooksSubcommand::List(command) => command.execute(),
        }
    }
}

impl HooksListCommand {
    fn execute(self) -> anyhow::Result<()> {
        let details = self.verbose.then_some(DETAILS.as_slice());

        if self.json {
            let mut output = serde_json::json!({
                "configured": 0,
                "hooks": [],
                "status": STATUS_NOT_WIRED,
                "v1_state": V1_STATE_NOT_USED,
            });
            if let Some(details) = details {
                output["details"] = serde_json::json!(details);
            }
            println!("{}", output);
            return Ok(());
        }

        println!("IronClaw Reborn hooks");
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
