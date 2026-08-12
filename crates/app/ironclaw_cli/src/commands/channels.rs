use clap::{Args, Subcommand};

use super::not_yet_implemented;

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
        Err(not_yet_implemented("channels list"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_reports_not_yet_implemented_regardless_of_flags() {
        for (verbose, json) in [(false, false), (true, false), (false, true), (true, true)] {
            let err = ChannelsListCommand { verbose, json }.execute().unwrap_err();
            assert_eq!(err.to_string(), "`channels list` is not implemented yet");
        }
    }
}
