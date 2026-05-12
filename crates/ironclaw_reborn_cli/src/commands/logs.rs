use clap::Args;

#[derive(Debug, Args)]
pub(crate) struct LogsCommand {
    /// Show extra status details.
    #[arg(short, long)]
    verbose: bool,

    /// Output log status as JSON.
    #[arg(long)]
    json: bool,
}

impl LogsCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        if self.json {
            let mut output = serde_json::json!({
                "entries": 0,
                "logs": [],
                "status": "not-wired",
                "v1_state": "not-used",
            });
            if self.verbose {
                output["details"] = serde_json::json!([
                    "Reborn log source is not wired yet",
                    "v1 gateway logs are intentionally not read"
                ]);
            }
            println!("{}", output);
            return Ok(());
        }

        println!("IronClaw Reborn logs");
        println!("entries: 0");
        println!("status: not-wired");
        println!("v1_state: not-used");

        if self.verbose {
            println!("detail: Reborn log source is not wired yet");
            println!("detail: v1 gateway logs are intentionally not read");
        }

        Ok(())
    }
}
