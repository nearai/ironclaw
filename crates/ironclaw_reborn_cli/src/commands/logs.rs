use clap::Args;

const STATUS_NOT_WIRED: &str = "not-wired";
const V1_STATE_NOT_USED: &str = "not-used";
const DETAILS: [&str; 2] = [
    "Reborn log source is not wired yet",
    "v1 gateway logs are intentionally not read",
];

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
        let details = self.verbose.then_some(DETAILS.as_slice());

        if self.json {
            let mut output = serde_json::json!({
                "entries": 0,
                "logs": [],
                "status": STATUS_NOT_WIRED,
                "v1_state": V1_STATE_NOT_USED,
            });
            if let Some(details) = details {
                output["details"] = serde_json::json!(details);
            }
            println!("{}", output);
            return Ok(());
        }

        println!("IronClaw Reborn logs");
        println!("entries: 0");
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
