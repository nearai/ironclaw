use clap::Args;

use crate::context::RebornCliContext;

/// Start an interactive Reborn CLI session backed by the composed runtime.
#[derive(Debug, Args)]
pub(crate) struct ReplCommand;

impl ReplCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();
        crate::runtime::execute(context, None)
    }
}
