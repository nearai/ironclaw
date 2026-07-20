use clap::Args;

use super::not_yet_implemented;

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
        Err(not_yet_implemented("logs"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_not_yet_implemented_regardless_of_flags() {
        for (verbose, json) in [(false, false), (true, false), (false, true), (true, true)] {
            let err = LogsCommand { verbose, json }.execute().unwrap_err();
            assert_eq!(err.to_string(), "`logs` is not implemented yet");
        }
    }
}
