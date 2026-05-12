use clap::{Args, Subcommand};

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
        if self.json {
            let mut output = serde_json::json!({
                "configured": 0,
                "channels": [],
                "status": "not-wired",
                "v1_state": "not-used",
            });
            if self.verbose {
                output["details"] = serde_json::json!([
                    "Reborn channel registry is not wired yet",
                    "v1 channel configuration is intentionally not read"
                ]);
            }
            println!("{}", output);
            return Ok(());
        }

        println!("IronClaw Reborn channels");
        println!("configured: 0");
        println!("status: not-wired");
        println!("v1_state: not-used");

        if self.verbose {
            println!("detail: Reborn channel registry is not wired yet");
            println!("detail: v1 channel configuration is intentionally not read");
        }

        Ok(())
    }
}
