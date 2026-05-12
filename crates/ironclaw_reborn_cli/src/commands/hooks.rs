use clap::{Args, Subcommand};

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
        if self.json {
            println!(
                "{}",
                serde_json::json!({
                    "hooks": [],
                    "status": "not-wired",
                    "v1_state": "not-used",
                })
            );
            return Ok(());
        }

        println!("IronClaw Reborn hooks");
        println!("configured: 0");
        println!("status: not-wired");
        println!("v1_state: not-used");

        if self.verbose {
            println!("detail: Reborn hook registry is not wired yet");
            println!("detail: v1 hook configuration is intentionally not read");
        }

        Ok(())
    }
}
