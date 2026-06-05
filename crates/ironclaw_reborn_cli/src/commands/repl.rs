use clap::Args;

use crate::context::RebornCliContext;
use crate::runtime::RuntimeInputOptions;

/// Start an interactive Reborn CLI session backed by the composed runtime.
#[derive(Debug, Args)]
pub(crate) struct ReplCommand {
    /// Confirm trusted-laptop host filesystem access for local-dev-yolo.
    #[arg(long = "confirm-host-access")]
    confirm_host_access: bool,
}

impl ReplCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();
        crate::runtime::execute(
            context,
            None,
            RuntimeInputOptions {
                confirm_host_access: self.confirm_host_access,
            },
        )
    }
}
