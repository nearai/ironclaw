use clap::{Args, Subcommand};

use super::not_yet_implemented;

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
        Err(not_yet_implemented("hooks list"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_reports_not_yet_implemented_regardless_of_flags() {
        for (verbose, json) in [(false, false), (true, false), (false, true), (true, true)] {
            let err = HooksListCommand { verbose, json }.execute().unwrap_err();
            assert_eq!(err.to_string(), "`hooks list` is not implemented yet");
        }
    }
}
