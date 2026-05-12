use clap::{Args, Subcommand};

const STATUS_NOT_WIRED: &str = "not-wired";
const V1_STATE_NOT_USED: &str = "not-used";
const DETAILS: [&str; 2] = [
    "Reborn channel registry is not wired yet",
    "v1 channel configuration is intentionally not read",
];

#[derive(Debug, Args)]
pub(crate) struct ChannelsCommand {
    #[command(subcommand)]
    command: ChannelsSubcommand,
}

#[derive(Debug, Subcommand)]
enum ChannelsSubcommand {
    /// List configured Reborn channels.
    List(ChannelsListCommand),
}

#[derive(Debug, Args)]
struct ChannelsListCommand {
    /// Show extra status details.
    #[arg(short, long)]
    verbose: bool,

    /// Output channels as JSON.
    #[arg(long)]
    json: bool,
}

impl ChannelsCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        match self.command {
            ChannelsSubcommand::List(command) => command.execute(),
        }
    }
}

impl ChannelsListCommand {
    fn execute(self) -> anyhow::Result<()> {
        let details = self.verbose.then_some(DETAILS.as_slice());

        if self.json {
            let mut output = serde_json::json!({
                "configured": 0,
                "channels": [],
                "status": STATUS_NOT_WIRED,
                "v1_state": V1_STATE_NOT_USED,
            });
            if let Some(details) = details {
                output["details"] = serde_json::json!(details);
            }
            println!("{}", output);
            return Ok(());
        }

        println!("IronClaw Reborn channels");
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
